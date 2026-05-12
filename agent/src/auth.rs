use std::collections::HashMap;
use std::fs;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::util::MutexExt;

const SALT_PATH: &str = "/data/.zte-agent-salt";
const TOKEN_TTL_SECS: u64 = 3600;
const MAX_TOKENS: usize = 10;
const HASH_ITERATIONS: u32 = 10_000;
const MAX_LOGIN_ATTEMPTS: u32 = 5;
const LOGIN_LOCKOUT_SECS: u64 = 30;

pub struct AuthState {
    password_hash: Mutex<Option<String>>,
    pin_hash: Mutex<Option<String>>,
    salt: Mutex<Vec<u8>>,
    tokens: Mutex<Vec<Token>>,
    failed_logins: Mutex<HashMap<String, LoginAttempt>>,
}

struct Token {
    value: String,
    expires: u64,
}

struct LoginAttempt {
    count: u32,
    locked_until: u64,
}

impl AuthState {
    pub fn new() -> Self {
        let salt = load_or_create_salt();
        Self {
            password_hash: Mutex::new(None),
            pin_hash: Mutex::new(None),
            salt: Mutex::new(salt),
            tokens: Mutex::new(Vec::new()),
            failed_logins: Mutex::new(HashMap::new()),
        }
    }

    pub fn set_password(&self, password: &str) {
        let salt = self.salt.safe_lock();
        let hash = iterated_hash(&salt, password);
        *self.password_hash.safe_lock() = Some(hash);
    }

    pub fn set_pin(&self, pin: &str) -> Result<(), String> {
        validate_pin(pin)?;
        let salt = self.salt.safe_lock();
        let hash = iterated_hash(&salt, pin);
        *self.pin_hash.safe_lock() = Some(hash);
        Ok(())
    }

    pub fn has_password(&self) -> bool {
        self.password_hash.safe_lock().is_some()
    }

    pub fn login_password(&self, password: &str, client_ip: &str) -> LoginResult {
        self.login_against(password, &self.password_hash, client_ip)
    }

    pub fn login_pin(&self, pin: &str, client_ip: &str) -> LoginResult {
        if validate_pin(pin).is_err() {
            self.record_failed_login(client_ip, epoch_secs());
            return LoginResult::Invalid;
        }
        self.login_against(pin, &self.pin_hash, client_ip)
    }

    fn login_against(
        &self,
        credential: &str,
        stored_hash: &Mutex<Option<String>>,
        client_ip: &str,
    ) -> LoginResult {
        let now = epoch_secs();

        {
            let mut attempts = self.failed_logins.safe_lock();
            if let Some(attempt) = attempts.get(client_ip) {
                if attempt.count >= MAX_LOGIN_ATTEMPTS && now < attempt.locked_until {
                    return LoginResult::Locked {
                        retry_after_secs: attempt.locked_until - now,
                    };
                }
                if now >= attempt.locked_until {
                    attempts.remove(client_ip);
                }
            }
        }

        let salt = self.salt.safe_lock();
        let hash = iterated_hash(&salt, credential);
        let stored = stored_hash.safe_lock();
        if stored.as_deref() != Some(&hash) {
            drop(stored);
            drop(salt);
            self.record_failed_login(client_ip, now);
            return LoginResult::Invalid;
        }
        drop(stored);
        drop(salt);

        {
            let mut attempts = self.failed_logins.safe_lock();
            attempts.remove(client_ip);
        }

        let mut random_bytes = [0u8; 16];
        read_urandom(&mut random_bytes);

        let token_bytes = {
            let mut hasher = Sha256::new();
            hasher.update(&random_bytes);
            hasher.update(hash.as_bytes());
            hasher.update(now.to_le_bytes());
            let digest = hasher.finalize();
            let mut arr = [0u8; 16];
            arr.copy_from_slice(&digest[..16]);
            arr
        };
        let token = hex_encode(&token_bytes);

        let mut tokens = self.tokens.safe_lock();
        tokens.retain(|t| t.expires > now);
        if tokens.len() >= MAX_TOKENS {
            tokens.remove(0);
        }
        tokens.push(Token {
            value: token.clone(),
            expires: now + TOKEN_TTL_SECS,
        });

        LoginResult::Ok { token }
    }

    fn record_failed_login(&self, client_ip: &str, now: u64) {
        let mut attempts = self.failed_logins.safe_lock();
        let entry = attempts
            .entry(client_ip.to_string())
            .or_insert(LoginAttempt {
                count: 0,
                locked_until: 0,
            });
        entry.count += 1;
        if entry.count >= MAX_LOGIN_ATTEMPTS {
            entry.locked_until = now + LOGIN_LOCKOUT_SECS;
        }
    }

    pub fn validate(&self, token: &str) -> bool {
        let now = epoch_secs();
        let mut tokens = self.tokens.safe_lock();
        tokens.retain(|t| t.expires > now);
        tokens.iter().any(|t| t.value == token)
    }
}

pub enum LoginResult {
    Ok { token: String },
    Invalid,
    Locked { retry_after_secs: u64 },
}

fn validate_pin(pin: &str) -> Result<(), String> {
    if pin.len() != 6 || !pin.bytes().all(|b| b.is_ascii_digit()) {
        return Err("PIN must be exactly 6 digits".into());
    }
    Ok(())
}

fn iterated_hash(salt: &[u8], password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(salt);
    hasher.update(password.as_bytes());
    let mut digest = hasher.finalize();
    for _ in 1..HASH_ITERATIONS {
        let mut hasher = Sha256::new();
        hasher.update(salt);
        hasher.update(&digest);
        digest = hasher.finalize();
    }
    hex_encode(&digest)
}

fn load_or_create_salt() -> Vec<u8> {
    if let Ok(salt_hex) = fs::read_to_string(SALT_PATH) {
        if let Ok(salt) = hex_decode(salt_hex.trim()) {
            if salt.len() >= 16 {
                return salt;
            }
        }
    }
    let mut salt = vec![0u8; 32];
    read_urandom(&mut salt);
    let _ = fs::write(SALT_PATH, hex_encode(&salt));
    salt
}

fn read_urandom(buf: &mut [u8]) {
    use std::io::Read;
    if let Ok(mut f) = fs::File::open("/dev/urandom") {
        let _ = f.read_exact(buf);
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const LUT: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(LUT[(b >> 4) as usize] as char);
        s.push(LUT[(b & 0x0f) as usize] as char);
    }
    s
}

fn hex_decode(hex: &str) -> Result<Vec<u8>, ()> {
    if hex.len() % 2 != 0 {
        return Err(());
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for chunk in hex.as_bytes().chunks(2) {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        bytes.push((hi << 4) | lo);
    }
    Ok(bytes)
}

fn hex_nibble(b: u8) -> Result<u8, ()> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(()),
    }
}

fn epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
