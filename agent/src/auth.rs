use std::collections::HashMap;
use std::fs;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::util::MutexExt;

const SALT_PATH: &str = "/data/.zte-agent-salt";
const TOKEN_TTL: Duration = Duration::from_secs(3600);
const MAX_TOKENS: usize = 10;
const HASH_ITERATIONS: u32 = 10_000;
const MAX_LOGIN_ATTEMPTS: u32 = 5;
const LOGIN_LOCKOUT_SECS: u64 = 30;
/// Forget a client's failure history after this long without a failed attempt,
/// so the map can't accumulate one row per IP that ever mistyped a password.
const LOGIN_ATTEMPT_TTL_SECS: u64 = 3600;

pub struct AuthState {
    password_hash: Mutex<Option<String>>,
    pin_hash: Mutex<Option<String>>,
    salt: Mutex<Vec<u8>>,
    tokens: Mutex<Vec<Token>>,
    failed_logins: Mutex<HashMap<String, LoginAttempt>>,
}

/// Sessions expire on the monotonic clock: the wall clock jumps when the
/// network sets the time after an attach (for example after a SIM switch), and
/// that must not end a session early or keep one alive.
struct Token {
    value: String,
    expires: Instant,
    kind: CredentialKind,
}

/// Which credential opened a session. Some actions need the password.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialKind {
    Password,
    Pin,
}

struct LoginAttempt {
    count: u32,
    locked_until: u64,
    last_attempt: u64,
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

    pub fn has_pin(&self) -> bool {
        self.pin_hash.safe_lock().is_some()
    }

    pub fn has_password(&self) -> bool {
        self.password_hash.safe_lock().is_some()
    }

    pub fn login_password(&self, password: &str, client_ip: &str) -> LoginResult {
        self.login_against(
            password,
            &self.password_hash,
            client_ip,
            CredentialKind::Password,
        )
    }

    pub fn login_pin(&self, pin: &str, client_ip: &str) -> LoginResult {
        if validate_pin(pin).is_err() {
            self.record_failed_login(client_ip, epoch_secs());
            return LoginResult::Invalid;
        }
        self.login_against(pin, &self.pin_hash, client_ip, CredentialKind::Pin)
    }

    fn login_against(
        &self,
        credential: &str,
        stored_hash: &Mutex<Option<String>>,
        client_ip: &str,
        kind: CredentialKind,
    ) -> LoginResult {
        let now = epoch_secs();

        {
            let mut attempts = self.failed_logins.safe_lock();
            if let Some(attempt) = attempts.get(client_ip) {
                if attempt.count >= MAX_LOGIN_ATTEMPTS {
                    if now < attempt.locked_until {
                        return LoginResult::Locked {
                            retry_after_secs: attempt.locked_until - now,
                        };
                    }
                    // Lockout expired — clear the counter and let this attempt run.
                    // Only clear here: clearing on every unlocked attempt (as this
                    // used to) reset `count` to 1 each time, so the threshold was
                    // never reached and the lockout never armed.
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
            hasher.update(random_bytes);
            hasher.update(hash.as_bytes());
            hasher.update(now.to_le_bytes());
            let digest = hasher.finalize();
            let mut arr = [0u8; 16];
            arr.copy_from_slice(&digest[..16]);
            arr
        };
        let token = hex_encode(&token_bytes);

        let mut tokens = self.tokens.safe_lock();
        let now = Instant::now();
        tokens.retain(|t| t.expires > now);
        if tokens.len() >= MAX_TOKENS {
            tokens.remove(0);
        }
        tokens.push(Token {
            value: token.clone(),
            expires: now + TOKEN_TTL,
            kind,
        });

        LoginResult::Ok { token }
    }

    fn record_failed_login(&self, client_ip: &str, now: u64) {
        let mut attempts = self.failed_logins.safe_lock();
        attempts.retain(|_, a| {
            now < a.locked_until || now.saturating_sub(a.last_attempt) < LOGIN_ATTEMPT_TTL_SECS
        });
        let entry = attempts
            .entry(client_ip.to_string())
            .or_insert(LoginAttempt {
                count: 0,
                locked_until: 0,
                last_attempt: now,
            });
        entry.count += 1;
        entry.last_attempt = now;
        if entry.count >= MAX_LOGIN_ATTEMPTS {
            entry.locked_until = now + LOGIN_LOCKOUT_SECS;
        }
    }

    /// Validate a bearer token, sliding its expiry forward on success. Without
    /// the slide a dashboard left open is logged out mid-poll on the hour.
    /// Returns the credential that opened the session.
    pub fn validate(&self, token: &str) -> Option<CredentialKind> {
        let now = Instant::now();
        let mut tokens = self.tokens.safe_lock();
        tokens.retain(|t| t.expires > now);
        let t = tokens.iter_mut().find(|t| t.value == token)?;
        t.expires = now + TOKEN_TTL;
        Some(t.kind)
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
        hasher.update(digest);
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
    if !hex.len().is_multiple_of(2) {
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

#[cfg(test)]
mod tests {
    use super::*;

    const IP: &str = "192.168.0.42";

    fn authed() -> AuthState {
        let auth = AuthState::new();
        auth.set_password("correct-horse");
        auth
    }

    fn token_of(result: LoginResult) -> String {
        match result {
            LoginResult::Ok { token } => token,
            LoginResult::Invalid => panic!("expected success, got Invalid"),
            LoginResult::Locked { .. } => panic!("expected success, got Locked"),
        }
    }

    #[test]
    fn lockout_arms_after_max_attempts() {
        let auth = authed();
        for i in 0..MAX_LOGIN_ATTEMPTS {
            assert!(
                matches!(auth.login_password("wrong", IP), LoginResult::Invalid),
                "attempt {i} should be Invalid, not locked yet"
            );
        }
        // Armed: even the correct password is refused while locked out.
        assert!(matches!(
            auth.login_password("correct-horse", IP),
            LoginResult::Locked { .. }
        ));
    }

    #[test]
    fn lockout_is_scoped_to_the_client_ip() {
        let auth = authed();
        for _ in 0..MAX_LOGIN_ATTEMPTS {
            let _ = auth.login_password("wrong", IP);
        }
        assert!(matches!(
            auth.login_password("wrong", IP),
            LoginResult::Locked { .. }
        ));
        // A different client is unaffected.
        let _ = token_of(auth.login_password("correct-horse", "192.168.0.99"));
    }

    #[test]
    fn successful_login_clears_failure_count() {
        let auth = authed();
        for _ in 0..MAX_LOGIN_ATTEMPTS - 1 {
            let _ = auth.login_password("wrong", IP);
        }
        let _ = token_of(auth.login_password("correct-horse", IP));

        // Counter reset, so we get a fresh full budget of attempts.
        for i in 0..MAX_LOGIN_ATTEMPTS {
            assert!(
                matches!(auth.login_password("wrong", IP), LoginResult::Invalid),
                "attempt {i} after reset should be Invalid"
            );
        }
    }

    #[test]
    fn validate_slides_token_expiry() {
        let auth = authed();
        let token = token_of(auth.login_password("correct-horse", IP));

        // Age the token to one second short of expiry.
        {
            let mut tokens = auth.tokens.safe_lock();
            tokens[0].expires = Instant::now() + Duration::from_secs(1);
        }

        assert_eq!(auth.validate(&token), Some(CredentialKind::Password));
        let expires = auth.tokens.safe_lock()[0].expires;
        assert!(
            expires >= Instant::now() + TOKEN_TTL - Duration::from_secs(2),
            "validate() should push expiry back out to the full TTL"
        );
    }

    #[test]
    fn validate_rejects_expired_and_unknown_tokens() {
        let auth = authed();
        let token = token_of(auth.login_password("correct-horse", IP));
        assert_eq!(auth.validate("not-a-real-token"), None);

        auth.tokens.safe_lock()[0].expires = Instant::now() - Duration::from_millis(1);
        assert_eq!(auth.validate(&token), None);
    }

    #[test]
    fn sessions_remember_which_credential_opened_them() {
        let auth = authed();
        auth.set_pin("123456").unwrap();
        let by_pin = token_of(auth.login_pin("123456", IP));
        let by_password = token_of(auth.login_password("correct-horse", IP));
        assert_eq!(auth.validate(&by_pin), Some(CredentialKind::Pin));
        assert_eq!(auth.validate(&by_password), Some(CredentialKind::Password));
    }
}
