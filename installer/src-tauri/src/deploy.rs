use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use flate2::read::GzDecoder;
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tar::Archive;
use tauri::{AppHandle, Emitter};

use crate::device::{adb_args, compatible_adb_devices};
use crate::identity::{Identity, PROBE};
use crate::model::{InstallMode, InstallOutcome, InstallRequest, InstallerError, ProgressEvent};
use crate::process::{find_on_path, output_text, run};
use crate::unlock;

const REPOSITORY: &str = "dklasens/MU5250-OpenUI";
const RELEASE_API: &str = "https://api.github.com/repos/dklasens/MU5250-OpenUI/releases/latest";
const DROPBEAR_URL: &str = "https://downloads.openwrt.org/releases/23.05.4/targets/armsr/armv8/packages/dropbear_2022.82-6_aarch64_generic.ipk";
const DROPBEAR_SHA256: &str = "4fadd1b8529f22fb5d64ee27159d11f4feb68224657953d298a1acf85a83a5c0";
const DASHBOARD_HTTPD_URL: &str = "https://downloads.openwrt.org/releases/23.05.4/packages/aarch64_generic/base/uhttpd_2023-06-25-34a8a74d-2_aarch64_generic.ipk";
const DASHBOARD_HTTPD_SHA256: &str =
    "bd3f010e71a5ea2ef6405e44dbe8c9e697454ce954c197f177ff0c13b9cf5991";
const REMOTE_BIN: &str = "/data/zte-agent";
/// The on-demand eSIM helper, installed and rolled back with the agent.
const REMOTE_ESIM_HELPER: &str = "/data/zte-esim";
const STARTUP_SCRIPT: &str = "/data/local/tmp/start_zte_agent.sh";
const DASHBOARD_STARTUP_SCRIPT: &str = "/data/local/tmp/start_dashboard.sh";

#[derive(Clone)]
pub struct Reporter {
    app: AppHandle,
    control: std::sync::Arc<crate::control::Control>,
}

impl Reporter {
    pub fn new(app: AppHandle, control: std::sync::Arc<crate::control::Control>) -> Self {
        Self { app, control }
    }

    fn emit(
        &self,
        kind: &str,
        message: impl Into<String>,
        step: Option<&str>,
        status: Option<&str>,
    ) {
        let _ = self.app.emit(
            "installer-progress",
            ProgressEvent {
                kind: kind.into(),
                message: message.into(),
                step: step.map(str::to_owned),
                status: status.map(str::to_owned),
            },
        );
    }

    pub fn log(&self, message: impl Into<String>) {
        self.emit("log", message, None, None);
    }

    pub fn active(&self, step: &str, message: &str) {
        self.emit("operation", message, Some(step), Some("running"));
    }

    pub fn step(&self, step: &str, status: &str, message: &str) {
        self.emit("step", message, Some(step), Some(status));
    }
}

#[derive(Clone)]
pub(crate) enum Channel {
    Adb {
        executable: PathBuf,
        serial: String,
    },
    Ssh {
        executable: PathBuf,
        gateway: String,
        known_hosts: PathBuf,
    },
}

impl Channel {
    fn name(&self) -> &'static str {
        match self {
            Self::Adb { .. } => "ADB",
            Self::Ssh { .. } => "SSH",
        }
    }

    pub(crate) fn shell(&self, command: &str, check: bool) -> Result<String, InstallerError> {
        self.shell_timeout(command, check, Duration::from_secs(60))
    }
    fn shell_timeout(
        &self,
        command: &str,
        check: bool,
        timeout: Duration,
    ) -> Result<String, InstallerError> {
        match self {
            Self::Adb { executable, serial } => {
                let marker = "__MU5250_RC__";
                let wrapped = format!("({command}); printf '\\n{marker}%s\\n' $?");
                let args = adb_args(serial, &["shell", &wrapped]);
                let output = crate::process::run_timeout(
                    executable,
                    &args,
                    None,
                    "an ADB command",
                    timeout,
                )?;
                let (stdout, stderr) = output_text(&output);
                let marker_index = stdout.rfind(marker);
                let (body, remote_code) = marker_index.map_or((stdout.as_str(), 1), |index| {
                    let code = stdout[index + marker.len()..]
                        .trim()
                        .parse::<i32>()
                        .unwrap_or(1);
                    (stdout[..index].trim(), code)
                });
                if check && (!output.status.success() || remote_code != 0) {
                    return Err(InstallerError::new(
                        "The modem rejected an ADB operation",
                        "Keep the modem connected. Copy the technical details before retrying.",
                        format!(
                            "Remote exit code {remote_code}\nCommand: {command}\n{body}\n{stderr}"
                        ),
                    ));
                }
                Ok(body.trim().to_owned())
            }
            Self::Ssh {
                executable,
                gateway,
                known_hosts,
            } => {
                let args = ssh_args(gateway, known_hosts, command);
                let output = crate::process::run_timeout(
                    executable,
                    &args,
                    None,
                    "an SSH command",
                    timeout,
                )?;
                let (stdout, stderr) = output_text(&output);
                if check && !output.status.success() {
                    return Err(InstallerError::new(
                        "The modem rejected an SSH operation",
                        "Check that the modem is still reachable and retry. Existing files are left intact.",
                        format!("Exit code {:?}\nCommand: {command}\n{stdout}\n{stderr}", output.status.code()),
                    ));
                }
                Ok(stdout)
            }
        }
    }

    pub(crate) fn push(&self, local: &Path, remote: &str) -> Result<(), InstallerError> {
        match self {
            Self::Adb { executable, serial } => {
                let args = vec![
                    "-s".into(),
                    serial.into(),
                    "push".into(),
                    local.as_os_str().to_owned(),
                    remote.into(),
                ];
                let output = run(executable, &args, None, "an ADB file transfer")?;
                if !output.status.success() {
                    let (stdout, stderr) = output_text(&output);
                    return Err(InstallerError::new(
                        "A file could not be copied to the modem",
                        "Reconnect the USB cable and retry.",
                        format!("adb push {} {remote}\n{stdout}\n{stderr}", local.display()),
                    ));
                }
            }
            Self::Ssh {
                executable,
                gateway,
                known_hosts,
            } => {
                let bytes = fs::read(local).map_err(|error| {
                    InstallerError::internal("reading a deployment file", error)
                })?;
                let args = ssh_args(
                    gateway,
                    known_hosts,
                    &format!("cat > {}", shell_quote(remote)),
                );
                let output = run(executable, &args, Some(&bytes), "an SSH file transfer")?;
                if !output.status.success() {
                    let (stdout, stderr) = output_text(&output);
                    return Err(InstallerError::new(
                        "A file could not be copied to the modem",
                        "Keep the modem connected and retry.",
                        format!("SSH stream to {remote}\n{stdout}\n{stderr}"),
                    ));
                }
            }
        }
        Ok(())
    }

    fn reboot(&self) -> Result<(), InstallerError> {
        if let Self::Adb { executable, serial } = self {
            let output = run(
                executable,
                &adb_args(serial, &["reboot"]),
                None,
                "ADB reboot",
            )?;
            if !output.status.success() {
                let (stdout, stderr) = output_text(&output);
                return Err(InstallerError::new(
                    "Installation succeeded, but the modem did not reboot",
                    "Reboot it manually to restore normal USB tethering.",
                    format!("adb reboot\n{stdout}\n{stderr}"),
                ));
            }
        }
        Ok(())
    }
}

fn ssh_args(gateway: &str, known_hosts: &Path, command: &str) -> Vec<OsString> {
    let mut args = vec![
        "-p".into(),
        "2222".into(),
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "StrictHostKeyChecking=accept-new".into(),
        "-o".into(),
        format!(
            "UserKnownHostsFile=\"{}\"",
            known_hosts
                .to_string_lossy()
                .replace('\\', "/")
                .replace('"', "\\\"")
        )
        .into(),
        "-o".into(),
        "ConnectTimeout=10".into(),
        format!("root@{gateway}").into(),
        command.into(),
    ];
    if let Ok(key) = crate::host::key_path() {
        if key.is_file() {
            args.splice(0..0, ["-i".into(), key.into_os_string()]);
            // Permit migration from earlier releases without replacing personal keys.
            if let Some(home) = dirs::home_dir() {
                for name in ["id_ed25519", "id_rsa", "id_ecdsa"] {
                    let legacy = home.join(".ssh").join(name);
                    if legacy.is_file() {
                        args.splice(0..0, ["-i".into(), legacy.into_os_string()]);
                    }
                }
            }
        }
    }
    args
}

fn ssh_channel(gateway: &str) -> Result<Channel, InstallerError> {
    let executable = find_on_path("ssh").ok_or_else(|| {
        InstallerError::new(
            "The SSH client is not available",
            "On Windows, enable the OpenSSH Client optional feature, then detect again.",
            "Could not find ssh or ssh.exe on PATH.",
        )
    })?;
    let home = dirs::home_dir().ok_or_else(|| {
        InstallerError::internal("locating the SSH directory", "home directory unavailable")
    })?;
    let known_hosts = home.join(".ssh/known_hosts.d/zte");
    if let Some(parent) = known_hosts.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            InstallerError::internal("creating the SSH known-hosts directory", error)
        })?;
    }
    Ok(Channel::Ssh {
        executable,
        gateway: gateway.into(),
        known_hosts,
    })
}

pub(crate) fn management_channel(
    gateway: &str,
    mode: InstallMode,
    adb: Option<&Path>,
    serial: Option<&str>,
) -> Result<Channel, InstallerError> {
    let channel = match mode {
        InstallMode::Adb => Channel::Adb {
            executable: adb
                .ok_or_else(|| InstallerError::internal("device verification", "ADB missing"))?
                .to_owned(),
            serial: serial
                .ok_or_else(|| InstallerError::internal("device verification", "serial missing"))?
                .into(),
        },
        InstallMode::Ssh => ssh_channel(gateway)?,
        InstallMode::Unlock => {
            return Err(InstallerError::internal(
                "device verification",
                "locked devices require authenticated web identity",
            ))
        }
    };
    Ok(channel)
}
pub(crate) fn probe_identity(
    gateway: &str,
    mode: InstallMode,
    adb: Option<&Path>,
    serial: Option<&str>,
) -> Result<Identity, InstallerError> {
    Identity::from_probe(&management_channel(gateway, mode, adb, serial)?.shell(PROBE, false)?)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn wait_for_modem_adb(adb: &Path, reporter: &Reporter) -> Result<String, InstallerError> {
    let deadline = Instant::now() + Duration::from_secs(240);
    while Instant::now() < deadline {
        if let Ok(devices) = compatible_adb_devices(adb) {
            if devices.len() == 1 {
                return Ok(devices[0].serial.clone());
            }
            if devices.len() > 1 {
                return Err(InstallerError::new(
                    "More than one compatible modem appeared after reboot",
                    "Disconnect the modem you do not want to change, then detect again.",
                    format!(
                        "Compatible serials: {}",
                        devices
                            .iter()
                            .map(|device| device.serial.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
            }
        }
        reporter.log("[*] Waiting for the ZTE modem to appear over ADB…");
        std::thread::sleep(Duration::from_secs(3));
    }
    Err(InstallerError::new(
        "The modem did not appear in ADB after reboot",
        "Check the USB cable. On Windows, install or select an ADB-compatible USB driver, then detect again.",
        "No compatible ZTE MU5250 transport appeared within 240 seconds.",
    ))
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

struct ReleaseAssets {
    tag: String,
    urls: HashMap<String, String>,
}

fn latest_release(tag: Option<&str>) -> Result<ReleaseAssets, InstallerError> {
    let url = tag
        .map(|tag| format!("https://api.github.com/repos/{REPOSITORY}/releases/tags/{tag}"))
        .unwrap_or_else(|| RELEASE_API.into());
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60))
        .user_agent("open-u60-pro-installer")
        .build()
        .map_err(|error| InstallerError::internal("creating the download client", error))?;
    let release: GitHubRelease = client
        .get(&url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .and_then(reqwest::blocking::Response::json)
        .map_err(|error| {
            InstallerError::new(
                "The latest installer files could not be found",
                "Check the internet connection and try again.",
                format!("GET {RELEASE_API}: {error}"),
            )
        })?;
    let urls = release
        .assets
        .into_iter()
        .map(|asset| (asset.name, asset.browser_download_url))
        .collect::<HashMap<_, _>>();
    for required in [
        "zte-agent",
        "zte-esim",
        "dashboard-dist.tar.gz",
        "sha256sums.txt",
    ] {
        if !urls.contains_key(required) {
            return Err(InstallerError::new(
                "The latest release is incomplete",
                "Try again later or report the release packaging problem.",
                format!(
                    "Release {} in {REPOSITORY} is missing {required}",
                    release.tag_name
                ),
            ));
        }
    }
    crate::bundle::compatible_release(&release.tag_name)?;
    Ok(ReleaseAssets {
        tag: release.tag_name,
        urls,
    })
}

fn download(url: &str, destination: &Path) -> Result<Vec<u8>, InstallerError> {
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .user_agent("open-u60-pro-installer")
        .build()
        .map_err(|error| InstallerError::internal("creating the download client", error))?;
    let response = client
        .get(url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| InstallerError::internal("downloading installation file", error))?;
    let mut bytes = Vec::new();
    response
        .take(crate::bundle::MAX_DOWNLOAD + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| InstallerError::internal("reading installation download", error))?;
    if bytes.len() as u64 > crate::bundle::MAX_DOWNLOAD {
        return Err(InstallerError::internal(
            "downloading installation file",
            "file exceeds 64 MiB",
        ));
    }
    fs::write(destination, &bytes)
        .map_err(|error| InstallerError::internal("saving a download", error))?;
    Ok(bytes)
}

fn fetch_release_assets(
    release: &ReleaseAssets,
    work: &Path,
    reporter: &Reporter,
    offline: Option<&Path>,
) -> Result<HashMap<String, PathBuf>, InstallerError> {
    let fetch = |name: &str, url: &str| -> Result<Vec<u8>, InstallerError> {
        if let Some(bundle) = offline {
            let bytes = crate::bundle::read_limited(&bundle.join(name))?;
            fs::write(work.join(name), &bytes)
                .map_err(|e| InstallerError::internal("copying offline artifact", e))?;
            Ok(bytes)
        } else {
            download(url, &work.join(name))
        }
    };
    reporter.log(format!("[*] Using release {}", release.tag));
    reporter.log("[*] Downloading release checksums…");
    let sums_bytes = fetch(
        "sha256sums.txt",
        release
            .urls
            .get("sha256sums.txt")
            .map(String::as_str)
            .unwrap_or_default(),
    )?;
    let sums_text = String::from_utf8_lossy(&sums_bytes);
    let sums = sums_text
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let sum = fields.next()?.to_owned();
            let name = fields.next()?.trim_start_matches('*').to_owned();
            Some((name, sum))
        })
        .collect::<HashMap<_, _>>();
    let mut files = HashMap::new();
    for name in ["zte-agent", "zte-esim", "dashboard-dist.tar.gz"] {
        let expected = sums.get(name).ok_or_else(|| {
            InstallerError::new(
                "A release checksum is missing",
                "No downloaded files were installed. Report the release packaging problem.",
                format!("sha256sums.txt has no entry for {name}"),
            )
        })?;
        reporter.log(format!("[*] Downloading {name}…"));
        let path = work.join(name);
        let bytes = fetch(
            name,
            release
                .urls
                .get(name)
                .map(String::as_str)
                .unwrap_or_default(),
        )?;
        let actual = hex::encode(Sha256::digest(&bytes));
        if &actual != expected {
            return Err(InstallerError::new(
                "A downloaded file failed integrity verification",
                "Nothing from the failed download was installed. Retry on a trusted network.",
                format!("{name}: expected {expected}, received {actual}"),
            ));
        }
        if name == "dashboard-dist.tar.gz" {
            crate::bundle::validate_dashboard(&bytes)?;
        } else {
            crate::bundle::validate_agent(&bytes)?;
        }
        reporter.log(format!("[+] {name} verified (SHA-256 {}…)", &actual[..16]));
        files.insert(name.into(), path);
    }
    for (name, url, checksum) in [
        ("dropbear", DROPBEAR_URL, DROPBEAR_SHA256),
        ("uhttpd", DASHBOARD_HTTPD_URL, DASHBOARD_HTTPD_SHA256),
    ] {
        reporter.log(format!(
            "[*] Downloading and verifying {name} before deployment…"
        ));
        let ipk = fetch(&format!("{name}.ipk"), url)?;
        verify_pinned_download(&ipk, checksum, name)?;
        let data = extract_ipk_data(&ipk, name)?;
        // The pinned Dropbear IPK uses symlinks to one multicall binary.
        // Copy that regular file under each argv[0] name; materialise no archive links.
        let programs: &[(&str, &str)] = if name == "dropbear" {
            &[
                ("usr/sbin/dropbear", "dropbear"),
                ("usr/sbin/dropbear", "dbclient"),
                ("usr/sbin/dropbear", "dropbearkey"),
            ]
        } else {
            &[("usr/sbin/uhttpd", "dashboard-uhttpd")]
        };
        for (source, destination) in programs {
            let bytes = crate::bundle::package_file(&data, source)?;
            let path = work.join(destination);
            fs::write(&path, bytes)
                .map_err(|e| InstallerError::internal("saving verified package binary", e))?;
            files.insert((*destination).into(), path);
        }
    }
    for (name, contents) in [
        (
            "start_dropbear.sh",
            include_str!("../../../scripts/device/start-dropbear.sh"),
        ),
        (
            "start_dashboard.sh",
            include_str!("../../../scripts/device/start-dashboard.sh"),
        ),
    ] {
        let path = work.join(name);
        fs::write(&path, contents)
            .map_err(|e| InstallerError::internal("preparing service script", e))?;
        files.insert(name.into(), path);
    }
    Ok(files)
}

fn startup_script(password: &str, pin: &str) -> String {
    let mut lines = vec![
        "#!/bin/sh".to_owned(),
        format!("export ZTE_AGENT_PASSWORD={}", shell_quote(password)),
    ];
    if !pin.is_empty() {
        lines.push(format!("export ZTE_AGENT_PIN={}", shell_quote(pin)));
    } else {
        lines.push("unset ZTE_AGENT_PIN".into());
    }
    lines.extend([
        "# The agent writes its own rotated log: /data/local/tmp/zte-agent.log".into(),
        "trap '' HUP\nnohup /data/zte-agent >/dev/null 2>&1 </dev/null &".into(),
    ]);
    format!("{}\n", lines.join("\n"))
}

#[cfg(test)]
fn adb_agent_login_command(gateway: &str, password: &str) -> String {
    let payload = json!({ "password": password }).to_string();
    format!(
        "/usr/bin/curl --fail --silent --show-error --connect-timeout 5 --max-time 10 -H 'Content-Type: application/json' --data-binary {} {}",
        shell_quote(&payload),
        shell_quote(&format!("http://{gateway}:9090/api/auth/login")),
    )
}

fn agent_json(
    channel: &Channel,
    _gateway: &str,
    path: &str,
    payload: Option<Value>,
    token: Option<&str>,
    mobile: bool,
) -> Option<Value> {
    let gateway = channel
        .shell("uci -q get zwrt_router.network.lan_ipaddr", false)
        .ok()?;
    if !gateway
        .parse::<std::net::Ipv4Addr>()
        .is_ok_and(|ip| ip.is_private())
    {
        return None;
    }
    let mut command = format!(
        "/usr/bin/curl --fail --silent --show-error --connect-timeout 5 --max-time 10 {}",
        shell_quote(&format!("http://{gateway}:9090{path}"))
    );
    if let Some(payload) = payload {
        command.push_str(&format!(
            " -H 'Content-Type: application/json' --data-binary {}",
            shell_quote(&payload.to_string())
        ));
    }
    if let Some(token) = token {
        command.push_str(&format!(
            " -H {}",
            shell_quote(&format!("Authorization: Bearer {token}"))
        ));
    }
    if mobile {
        command.push_str(" -A 'Mozilla/5.0 Mobile'");
    }
    // False suppresses command/body details in diagnostics: those can contain
    // credentials and bearer tokens. The JSON envelope proves success instead.
    let response: Value = serde_json::from_str(&channel.shell(&command, false).ok()?).ok()?;
    (response["ok"].as_bool() == Some(true)).then_some(response)
}
fn agent_login(channel: &Channel, gateway: &str, password: &str) -> bool {
    agent_json(
        channel,
        gateway,
        "/api/auth/login",
        Some(json!({"password": password})),
        None,
        false,
    )
    .is_some_and(|response| {
        response
            .pointer("/data/token")
            .and_then(Value::as_str)
            .is_some()
    })
}
fn verify_agent_credentials(channel: &Channel, gateway: &str, password: &str, pin: &str) -> bool {
    let Some(login) = agent_json(
        channel,
        gateway,
        "/api/auth/login",
        Some(json!({"password": password})),
        None,
        false,
    ) else {
        return false;
    };
    let Some(token) = login.pointer("/data/token").and_then(Value::as_str) else {
        return false;
    };
    let Some(device) = agent_json(channel, gateway, "/api/device", None, Some(token), false) else {
        return false;
    };
    if device
        .pointer("/data/auth/pin_enabled")
        .and_then(Value::as_bool)
        != Some(!pin.is_empty())
    {
        return false;
    }
    pin.is_empty()
        || agent_json(
            channel,
            gateway,
            "/api/auth/login",
            Some(json!({"pin": pin})),
            None,
            true,
        )
        .is_some_and(|response| {
            response
                .pointer("/data/token")
                .and_then(Value::as_str)
                .is_some()
        })
}

fn deploy_agent(
    channel: &Channel,
    gateway: &str,
    password: &str,
    pin: &str,
    files: &HashMap<String, PathBuf>,
    work: &Path,
    reporter: &Reporter,
) -> Result<(), InstallerError> {
    let local = &files["zte-agent"];
    let local_bytes =
        fs::read(local).map_err(|error| InstallerError::internal("reading zte-agent", error))?;
    let local_sha = hex::encode(Sha256::digest(&local_bytes));
    let remote_sha = channel.shell(
        &format!("sha256sum {REMOTE_BIN} 2>/dev/null | awk '{{print $1}}'"),
        false,
    )?;
    let helper = &files["zte-esim"];
    let helper_bytes =
        fs::read(helper).map_err(|error| InstallerError::internal("reading zte-esim", error))?;
    let helper_sha = hex::encode(Sha256::digest(&helper_bytes));
    let remote_helper_sha = channel.shell(
        &format!("sha256sum {REMOTE_ESIM_HELPER} 2>/dev/null | awk '{{print $1}}'"),
        false,
    )?;
    if remote_helper_sha.trim() != helper_sha {
        reporter.log("[*] Staging and verifying the eSIM helper on the modem…");
        let staged = format!("{REMOTE_ESIM_HELPER}.new");
        channel.push(helper, &staged)?;
        // A helper started by the running agent holds the card lock until it exits.
        channel.shell(&format!("set -e\ntest \"$(sha256sum {staged} | awk '{{print $1}}')\" = {helper_sha}\nchmod 700 {staged}\nfor i in 1 2 3 4 5 6 7 8 9 10; do pidof zte-esim >/dev/null || break; sleep 1; done\nmv -f {staged} {REMOTE_ESIM_HELPER}"), true)?;
        reporter.log("[+] eSIM helper installed");
    }
    // The version request touches no card; it proves the helper runs and speaks the agent's
    // protocol (HELPER_PROTOCOL in agent/src/esim.rs).
    let version = channel.shell(
        &format!("printf '%s\\n' '{{\"protocol\":2,\"op\":\"version\"}}' | {REMOTE_ESIM_HELPER}"),
        true,
    )?;
    if !version.contains("\"protocol\":2") {
        return Err(InstallerError::new(
            "The eSIM helper did not start",
            "Copy the log and retry. SIM Management stays unavailable until the helper runs.",
            format!("{REMOTE_ESIM_HELPER} answered: {}", version.trim()),
        ));
    }

    let changed = remote_sha != local_sha;
    if changed {
        reporter.log("[*] Staging and verifying the agent binary on the modem…");
        let staged = format!("{REMOTE_BIN}.new");
        channel.push(local, &staged)?;
        channel.shell(&format!("test \"$(sha256sum {staged} | awk '{{print $1}}')\" = {local_sha} && chmod 700 {staged}"), true)?;
        channel.shell("killall zte-agent 2>/dev/null; sleep 1", false)?;
        channel.shell(&format!("mv -f {staged} {REMOTE_BIN}"), true)?;
        reporter.log("[+] Agent binary installed");
    } else {
        reporter.log("[+] Agent binary is already current");
    }

    let desired_script = startup_script(password, pin);
    let desired_hash = hex::encode(Sha256::digest(desired_script.as_bytes()));
    let current_hash = channel.shell(
        &format!("sha256sum {STARTUP_SCRIPT} 2>/dev/null | awk '{{print $1}}'"),
        false,
    )?;
    let credentials_changed = current_hash.trim() != desired_hash;
    if credentials_changed {
        let script_path = work.join("start_zte_agent.sh");
        fs::write(&script_path, desired_script)
            .map_err(|error| InstallerError::internal("creating agent startup script", error))?;
        channel.shell("mkdir -p /data/local/tmp", true)?;
        let staged = format!("{STARTUP_SCRIPT}.new");
        channel.push(&script_path, &staged)?;
        channel.shell(&format!("set -e\nsh -n {staged}\ntest \"$(sha256sum {staged} | awk '{{print $1}}')\" = {desired_hash}\nchmod 700 {staged}\nmv {staged} {STARTUP_SCRIPT}"), true)?;
        reporter.log("[+] Agent credentials and startup script updated");
    } else {
        reporter.log("[+] Agent startup script is already current");
    }

    let rc_line = format!("sh {STARTUP_SCRIPT}");
    if channel
        .shell(
            &format!(
                "grep -qF '{}' /etc/rc.local 2>/dev/null && echo OK",
                rc_line
            ),
            false,
        )?
        .trim()
        != "OK"
    {
        update_rc_local(channel, &[&rc_line], false)?;
        reporter.log("[+] Agent auto-start added to rc.local");
    }

    if changed || credentials_changed || !agent_login(channel, gateway, password) {
        reporter.log("[*] Starting the agent…");
        channel.shell("killall zte-agent 2>/dev/null; true", false)?;
        std::thread::sleep(Duration::from_secs(1));
        channel.shell(&format!("sh {STARTUP_SCRIPT}"), true)?;
        std::thread::sleep(Duration::from_secs(2));
    }
    if !verify_agent_credentials(channel, gateway, password, pin) {
        return Err(InstallerError::new(
            "The agent was installed but credential verification failed",
            "Copy the log and retry. On an already provisioned modem, check logread -e zte-agent over SSH.",
            format!("Agent login at http://{gateway}:9090/api/auth/login returned no token via {}.", channel.name()),
        ));
    }
    reporter.log("[+] Agent is running and authenticated");
    Ok(())
}

fn ensure_local_ssh_key(reporter: &Reporter) -> Result<String, InstallerError> {
    let public = crate::host::ensure_key()?;
    reporter.log("[+] Dedicated installer SSH identity is ready");
    Ok(public)
}

fn verify_pinned_download(
    bytes: &[u8],
    expected: &str,
    package: &str,
) -> Result<(), InstallerError> {
    let actual = hex::encode(Sha256::digest(bytes));
    if actual == expected {
        return Ok(());
    }
    Err(InstallerError::new(
        format!("The {package} package failed integrity verification"),
        "Nothing from the package was installed. Retry on a trusted network.",
        format!("Expected SHA-256 {expected}, received {actual}"),
    ))
}

fn extract_ipk_data(ipk: &[u8], package: &str) -> Result<Vec<u8>, InstallerError> {
    let decoder = GzDecoder::new(Cursor::new(ipk));
    let mut archive = Archive::new(decoder);
    for item in archive
        .entries()
        .map_err(|error| InstallerError::internal("reading the Dropbear package", error))?
    {
        let mut entry = item
            .map_err(|error| InstallerError::internal("reading a Dropbear package entry", error))?;
        let path = entry
            .path()
            .map_err(|error| InstallerError::internal("reading a Dropbear package path", error))?;
        if path.file_name().and_then(|value| value.to_str()) == Some("data.tar.gz") {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).map_err(|error| {
                InstallerError::internal("extracting package data.tar.gz", error)
            })?;
            return Ok(bytes);
        }
    }
    Err(InstallerError::new(
        format!("The downloaded {package} package has an unexpected layout"),
        "Nothing from the package was installed. Retry or report the packaging change.",
        "data.tar.gz was not found in the IPK.",
    ))
}

fn update_rc_local(
    channel: &Channel,
    lines: &[&str],
    remove_debug: bool,
) -> Result<(), InstallerError> {
    let mut script = "set -e\ncp -p /etc/rc.local /etc/rc.local.open-u60-new\n".to_string();
    for line in lines {
        // Caller supplies fixed program-owned lines, never credentials or user input.
        script.push_str(&format!("if ! grep -qFx {} /etc/rc.local.open-u60-new; then\n  if grep -q '^exit 0' /etc/rc.local.open-u60-new; then sed -i '/^exit 0/i {line}' /etc/rc.local.open-u60-new; else printf '%s\\n' {} >> /etc/rc.local.open-u60-new; fi\nfi\n", shell_quote(line), shell_quote(line)));
    }
    if remove_debug {
        script.push_str("sed -i '/^echo [0-9] > .*usb_op$/d' /etc/rc.local.open-u60-new\n");
    }
    script.push_str(
        "sh -n /etc/rc.local.open-u60-new\nmv /etc/rc.local.open-u60-new /etc/rc.local\nsync",
    );
    channel.shell(&script, true).map(|_| ())
}

fn install_script(
    channel: &Channel,
    files: &HashMap<String, PathBuf>,
    name: &str,
    destination: &str,
) -> Result<(), InstallerError> {
    let bytes = fs::read(&files[name])
        .map_err(|e| InstallerError::internal("reading service script", e))?;
    let checksum = hex::encode(Sha256::digest(&bytes));
    let staged = format!("{destination}.new");
    channel.push(&files[name], &staged)?;
    channel.shell(&format!("set -e\ntest \"$(sha256sum {staged} | awk '{{print $1}}')\" = {checksum}\nsh -n {staged}\nchmod 700 {staged}\nmv -f {staged} {destination}"), true).map(|_| ())
}

fn install_program(
    channel: &Channel,
    files: &HashMap<String, PathBuf>,
    name: &str,
) -> Result<(), InstallerError> {
    let checksum = hex::encode(Sha256::digest(crate::bundle::read_limited(&files[name])?));
    let destination = format!("/data/bin/{name}");
    let staged = format!("{destination}.new");
    channel.shell("mkdir -p /data/bin", true)?;
    if channel
        .shell(
            &format!("sha256sum {destination} 2>/dev/null | awk '{{print $1}}'"),
            false,
        )?
        .trim()
        == checksum
    {
        return Ok(());
    }
    channel.push(&files[name], &staged)?;
    channel.shell(&format!("set -e\ntest \"$(sha256sum {staged} | awk '{{print $1}}')\" = {checksum}\nchmod 700 {staged}\nmv -f {staged} {destination}"), true).map(|_| ())
}

fn password_auth_advertised(debug: &str) -> Result<bool, InstallerError> {
    let methods: Vec<&str> = debug
        .lines()
        .filter_map(|line| {
            line.split_once("Authentications that can continue:")
                .map(|(_, methods)| methods.trim())
        })
        .collect();
    if methods.is_empty() {
        return Err(InstallerError::new(
            "SSH authentication policy could not be verified",
            "Keep the modem connected and inspect the installation diagnostics.",
            "OpenSSH did not report the authentication methods offered by the modem.",
        ));
    }
    Ok(methods.iter().any(|line| {
        line.split(',')
            .any(|method| matches!(method.trim(), "password" | "keyboard-interactive"))
    }))
}

fn verify_ssh_security(gateway: &str) -> Result<(), InstallerError> {
    let channel = ssh_channel(gateway)?;
    let Channel::Ssh {
        executable,
        known_hosts,
        ..
    } = channel
    else {
        unreachable!()
    };
    let mut args = ssh_args(gateway, &known_hosts, "true");
    let key = crate::host::key_path()?;
    while let Some(index) = args.iter().position(|v| v == "-i") {
        args.drain(index..index + 2);
    }
    args.splice(
        0..0,
        [
            "-i".into(),
            key.into_os_string(),
            "-o".into(),
            "IdentitiesOnly=yes".into(),
        ],
    );
    let positive = crate::process::run_timeout(
        &executable,
        &args,
        None,
        "verifying the installed SSH key",
        Duration::from_secs(15),
    )?;
    if !positive.status.success() {
        return Err(InstallerError::internal(
            "verifying the installed SSH key",
            "the configured Ed25519 key could not authenticate",
        ));
    }

    args.splice(
        0..0,
        [
            "-vv".into(),
            "-o".into(),
            "PubkeyAuthentication=no".into(),
            "-o".into(),
            "NumberOfPasswordPrompts=0".into(),
        ],
    );
    let output = crate::process::run_timeout(
        &executable,
        &args,
        None,
        "SSH authentication policy verification",
        Duration::from_secs(15),
    )?;
    if output.status.success()
        || password_auth_advertised(&String::from_utf8_lossy(&output.stderr))?
    {
        return Err(InstallerError::new(
            "SSH still permits password authentication",
            "Installation has not been accepted. The previous configuration will be restored.",
            "The modem's SSH authentication methods did not enforce key-only access.",
        ));
    }
    Ok(())
}

fn install_dashboard_httpd(
    channel: &Channel,
    files: &HashMap<String, PathBuf>,
    reporter: &Reporter,
) -> Result<(), InstallerError> {
    install_program(channel, files, "dashboard-uhttpd")?;
    reporter.log("[+] Isolated dashboard web server installed and verified");
    Ok(())
}

fn harden(
    channel: &Channel,
    gateway: &str,
    files: &HashMap<String, PathBuf>,
    reporter: &Reporter,
) -> Result<(), InstallerError> {
    for name in ["dropbear", "dbclient", "dropbearkey"] {
        install_program(channel, files, name)?;
    }
    channel.shell(
        "[ -f /data/bin/dropbear ] && [ -x /data/bin/dropbear ]",
        true,
    )?;
    reporter.log("[+] Dropbear binaries verified");

    reporter.log("[*] Configuring SSH keys and persistent host keys…");
    let public_key = ensure_local_ssh_key(reporter)?;
    let quoted_key = shell_quote(&public_key);
    channel.shell(
        "mkdir -p /etc/dropbear /data/dropbear && chmod 700 /etc/dropbear",
        true,
    )?;
    channel.shell(&format!("grep -qF {quoted_key} /etc/dropbear/authorized_keys 2>/dev/null || echo {quoted_key} >> /etc/dropbear/authorized_keys; chmod 600 /etc/dropbear/authorized_keys"), true)?;
    channel.shell("for k in ed25519 rsa; do f=/etc/dropbear/dropbear_${k}_host_key; [ -s \"$f\" ] || /data/bin/dropbearkey -t $k -f $f >/dev/null 2>&1; done", true)?;
    channel.shell("cp /etc/dropbear/authorized_keys /etc/dropbear/dropbear_*_host_key /data/dropbear/ 2>/dev/null; chmod 600 /data/dropbear/*", true)?;
    install_script(
        channel,
        files,
        "start_dropbear.sh",
        "/data/local/tmp/start_dropbear.sh",
    )?;

    install_dashboard_httpd(channel, files, reporter)?;

    reporter.log("[*] Configuring the isolated dashboard listener…");
    channel.shell("mkdir -p /data/www /data/local/tmp", true)?;
    install_script(
        channel,
        files,
        "start_dashboard.sh",
        DASHBOARD_STARTUP_SCRIPT,
    )?;
    channel.shell("set -e\nif uci -q get uhttpd.dashboard >/dev/null; then uci -q delete uhttpd.dashboard; uci commit uhttpd; /etc/init.d/uhttpd restart; fi", true)?;

    reporter.log("[*] Validating the safe rc.local startup entries…");
    update_rc_local(
        channel,
        &[
            "sh /data/local/tmp/start_zte_agent.sh",
            "sh /data/local/tmp/start_dropbear.sh",
            "sh /data/local/tmp/start_dashboard.sh",
        ],
        true,
    )?;
    reporter.log("[+] rc.local is configured and passes its syntax check");

    reporter.log("[*] Starting and checking the dashboard web service…");
    channel.shell(
        &format!("set -e\nroot=/data/www\nif [ -L /data/www.current ]; then root=$(readlink -f /data/www.current); fi\necho DASHBOARD_READY > \"$root/.installer-health\"\nchmod 644 \"$root/.installer-health\"\nsh {DASHBOARD_STARTUP_SCRIPT}"),
        true,
    )?;
    std::thread::sleep(Duration::from_secs(1));
    channel.shell("test \"$(/usr/bin/curl --fail --silent --show-error --connect-timeout 5 --max-time 10 http://127.0.0.1:8080/.installer-health)\" = DASHBOARD_READY && rm -f /data/www/.installer-health /data/www.current/.installer-health", true)?;
    reporter.log("[+] Dashboard listener is running on port 8080");

    reporter.log("[*] Disabling unattended firmware updates…");
    let fota = channel.shell("ubus call zwrt_zte_dm set_update_mode '{\"dm_update_mode\":\"0\"}' >/dev/null 2>&1; uci get zwrt_zte_dm.dm_update.dm_update_mode", false)?;
    if fota.lines().any(|line| line.trim() == "0") {
        reporter.log("[+] Unattended firmware updates are disabled");
    } else {
        return Err(InstallerError::new("Firmware auto-update could not be disabled", "Installation has not been accepted. Inspect the modem's firmware-update settings before retrying.", "dm_update_mode readback was not 0"));
    }

    channel.shell("sh /data/local/tmp/start_dropbear.sh", true)?;
    std::thread::sleep(Duration::from_secs(1));
    verify_ssh_security(gateway)?;
    reporter.log(format!("[+] Key-only SSH verified at root@{gateway}:2222"));

    Ok(())
}

fn deploy_dashboard(
    channel: &Channel,
    gateway: &str,
    files: &HashMap<String, PathBuf>,
    reporter: &Reporter,
) -> Result<(), InstallerError> {
    reporter.log("[*] Staging a complete dashboard release…");
    let archive = &files["dashboard-dist.tar.gz"];
    let expected =
        hex::encode(Sha256::digest(fs::read(archive).map_err(|e| {
            InstallerError::internal("reading dashboard archive", e)
        })?));
    let version = uuid::Uuid::new_v4().to_string();
    let root = format!("/data/open-u60-dashboards/{version}");
    channel.push(archive, "/tmp/dashboard-dist.tar.gz")?;
    channel.shell(&format!("set -e\ntest \"$(sha256sum /tmp/dashboard-dist.tar.gz | awk '{{print $1}}')\" = {expected}\nmkdir -p {root}\ntar xzf /tmp/dashboard-dist.tar.gz -C {root}\ntest -s {root}/index.html\ncp {root}/index.html {root}/mobile.html\nchmod -R a+rX {root}\nrm -f /data/www.current.new\nln -s {root} /data/www.current.new\nmv -Tf /data/www.current.new /data/www.current\nrm -f /tmp/dashboard-dist.tar.gz"), true)?;
    channel.shell(&format!("sh {DASHBOARD_STARTUP_SCRIPT}"), true)?;
    std::thread::sleep(Duration::from_secs(1));
    let page = channel.shell("/usr/bin/curl --fail --silent --show-error --connect-timeout 5 --max-time 10 http://127.0.0.1:8080/", true)?;
    if !page.contains("<div id=\"root\"></div>") {
        return Err(InstallerError::new(
            "The dashboard was copied but its verification page was unexpected",
            "The deployment transaction will restore the previous dashboard. Copy the diagnostic details before retrying.",
            format!(
                "First response bytes: {}",
                page.chars().take(500).collect::<String>()
            ),
        ));
    }
    reporter.log(format!("[+] Dashboard verified at http://{gateway}:8080"));
    Ok(())
}

fn storage_preflight(
    channel: &Channel,
    files: &HashMap<String, PathBuf>,
    reporter: &Reporter,
) -> Result<(), InstallerError> {
    let dashboard = crate::bundle::validate_dashboard(&crate::bundle::read_limited(
        &files["dashboard-dist.tar.gz"],
    )?)?;
    let payload = files
        .values()
        .try_fold(0_u64, |sum, path| fs::metadata(path).map(|m| sum + m.len()))
        .map_err(|e| InstallerError::internal("sizing installation files", e))?;
    let output = channel.shell("set -e\nfor tool in sh sha256sum tar cp mv readlink awk df du sync ubus uci; do command -v \"$tool\" >/dev/null; done\ntest -x /usr/bin/curl\nsh -n /etc/rc.local\ntest -w /data\ntest -w /etc\ntest ! -e /data/local/tmp/open-u60-transactions/active\ntest ! -d /data/local/tmp/open-u60-transactions/lock\ndf -Pk /data | awk 'END {print $4}'\ndf -Pk /tmp | awk 'END {print $4}'\ndf -Pk /etc | awk 'END {print $4}'\n(du -sk /data/zte-agent /data/zte-esim /data/www /data/www.current /data/bin /data/dropbear /data/local/tmp/start_*.sh /etc/rc.local /etc/dropbear /etc/config/uhttpd 2>/dev/null || true) | awk '{n+=$1} END {print n+0}'", true)?;
    let values = output
        .lines()
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| InstallerError::internal("reading device free space", e))?;
    if values.len() != 4 {
        return Err(InstallerError::internal(
            "reading device free space",
            "unexpected df/du output",
        ));
    }
    let data_required = payload + dashboard + values[3] * 2048 + 16 * 1024 * 1024;
    let tmp_required = payload + 16 * 1024 * 1024;
    if values[0] * 1024 < data_required || values[1] * 1024 < tmp_required || values[2] < 2048 {
        return Err(InstallerError::new("The modem needs more free storage before installation", "Keep the existing installation. Free space from old downloads or recovery snapshots only after saving any recovery files you need.", format!("Free KiB data/tmp/etc: {:?}; required data {data_required} bytes, tmp {tmp_required} bytes, etc 2048 KiB", &values[..3])));
    }
    reporter.log(format!(
        "[+] Storage preflight passed, including snapshot and rollback space ({} MiB data budget)",
        data_required.div_ceil(1024 * 1024)
    ));
    Ok(())
}

fn write_manifest(
    channel: &Channel,
    identity: &Identity,
    release: &str,
    files: &HashMap<String, PathBuf>,
    work: &Path,
) -> Result<(), InstallerError> {
    let mut checksums = std::collections::BTreeMap::new();
    for (name, path) in files {
        checksums.insert(
            name,
            hex::encode(Sha256::digest(crate::bundle::read_limited(path)?)),
        );
    }
    let manifest = json!({"format_version": 1, "release": release, "installer_version": env!("CARGO_PKG_VERSION"),
        "device": identity, "files": checksums});
    let bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| InstallerError::internal("writing deployment manifest", e))?;
    let path = work.join("installed-manifest.json");
    fs::write(&path, &bytes)
        .map_err(|e| InstallerError::internal("writing deployment manifest", e))?;
    let checksum = hex::encode(Sha256::digest(&bytes));
    channel.push(&path, "/data/open-u60-manifest.json.new")?;
    channel.shell(&format!("set -e\ntest \"$(sha256sum /data/open-u60-manifest.json.new | awk '{{print $1}}')\" = {checksum}\nchmod 600 /data/open-u60-manifest.json.new\nmv /data/open-u60-manifest.json.new /data/open-u60-manifest.json\nsync"), true).map(|_| ())
}

pub fn perform_install(
    request: InstallRequest,
    snapshot: crate::model::DetectionSnapshot,
    work: &Path,
    reporter: Reporter,
    pinned_release: Option<String>,
) -> Result<InstallOutcome, InstallerError> {
    let mode = snapshot
        .mode
        .ok_or_else(|| InstallerError::internal("reading deployment plan", "mode missing"))?;
    let operation = snapshot
        .operation
        .ok_or_else(|| InstallerError::internal("reading deployment plan", "operation missing"))?;
    let adb_path = snapshot.adb_path;
    crate::host::prerequisites(adb_path.as_deref(), &request.gateway)?;
    let selected_serial = snapshot.adb_serial;
    let expected_identity = snapshot.identity;
    reporter.log(format!(
        "[*] {} started for {}",
        operation.label(),
        request.gateway
    ));
    reporter.log(format!("[*] Temporary workspace: {}", work.display()));

    reporter.active(
        "prepare",
        "Downloading and verifying the complete installation bundle…",
    );
    let explicit_offline = request
        .bundle_path
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .map(Path::new);
    let mut cached = None;
    let release = if let Some(path) = explicit_offline {
        ReleaseAssets {
            tag: crate::bundle::load(path)?,
            urls: HashMap::new(),
        }
    } else {
        match latest_release(pinned_release.as_deref()) {
            Ok(release) => {
                cached = crate::bundle::find_cached(Some(&release.tag));
                release
            }
            Err(error) => {
                cached = crate::bundle::find_cached(pinned_release.as_deref());
                if let Some(path) = &cached {
                    reporter.log("[+] Using a previously verified offline bundle; release lookup is unavailable");
                    ReleaseAssets {
                        tag: crate::bundle::load(path)?,
                        urls: HashMap::new(),
                    }
                } else {
                    return Err(error);
                }
            }
        }
    };
    if pinned_release
        .as_ref()
        .is_some_and(|tag| tag != &release.tag)
    {
        return Err(InstallerError::new(
            "The selected release changed",
            "Check the new bundle before installing.",
            "The install release did not match the checked release.",
        ));
    }
    let offline = explicit_offline.or(cached.as_deref());
    if cached.is_some() {
        reporter.log("[+] Reusing cached files after checksum verification");
    }
    let files = fetch_release_assets(&release, work, &reporter, offline)?;
    if offline.is_none() {
        let saved = crate::bundle::export(work, &release.tag)?;
        reporter.log(format!(
            "[+] Verified offline bundle saved: {}",
            saved.display()
        ));
    }
    reporter.step(
        "prepare",
        "complete",
        "All installation files verified before device changes",
    );

    reporter.control.checkpoint()?;
    let mut expected_identity = expected_identity;
    let mut channel = match mode {
        InstallMode::Unlock => {
            reporter.active("unlock", "Preparing and validating the modem backup…");
            let unlock_work = work.join("unlock");
            let unlocked_identity = unlock::run_unlock(
                &request.gateway,
                &request.router_password,
                &request.backup_suffix,
                request.dry_run,
                &unlock_work,
                &|message| reporter.log(message),
                &|identity| {
                    expected_identity
                        .as_ref()
                        .ok_or_else(|| {
                            InstallerError::internal("unlock check", "Checked identity missing")
                        })?
                        .require_same(identity)?;
                    reporter.control.await_approval(|| reporter.emit("confirm", "The backup and all downloads have been validated. Ready to unlock and reboot?", None, None))
                },
            )?;
            expected_identity = Some(unlocked_identity.clone());
            reporter.step("unlock", "complete", "Backup preparation complete");
            if request.dry_run {
                for step in ["wait", "agent", "ssh", "dashboard"] {
                    reporter.step(step, "skipped", "Not run during a dry run");
                }
                return Ok(InstallOutcome {
                    result: "dryRun".into(),
                    verified_identity: Some(unlocked_identity.clone()),
                    device_model: unlocked_identity.model.clone(), firmware: unlocked_identity.firmware.clone(), release: release.tag.clone(),
                    title: "Dry run completed safely".into(),
                    message: "The modem backup was downloaded, decrypted, checked, and patched in memory. Nothing was uploaded and the modem was not changed.".into(),
                    operation,
                    dashboard_url: None,
                    api_url: None,
                    ssh_address: None,
                    diagnostic_path: None,
                });
            }
            reporter.active("wait", "Waiting for the modem to restart in ADB mode…");
            let adb = adb_path.ok_or_else(|| {
                InstallerError::new(
                "ADB is unavailable after the restore",
                "Use the packaged installer or install Android platform-tools, then detect again.",
                "The detection snapshot had no ADB executable.",
            )
            })?;
            let serial = wait_for_modem_adb(&adb, &reporter)?;
            reporter.step("wait", "complete", "Modem reconnected over ADB");
            Channel::Adb {
                executable: adb,
                serial,
            }
        }
        InstallMode::Adb => {
            reporter.step("unlock", "skipped", "Already unlocked");
            reporter.step("wait", "skipped", "ADB is already connected");
            Channel::Adb {
                executable: adb_path.ok_or_else(|| {
                    InstallerError::internal("starting the ADB install", "ADB path missing")
                })?,
                serial: selected_serial.ok_or_else(|| {
                    InstallerError::internal("starting the ADB install", "ADB serial missing")
                })?,
            }
        }
        InstallMode::Ssh => {
            reporter.step("unlock", "skipped", "Already provisioned");
            reporter.step("wait", "skipped", "Using SSH");
            ssh_channel(&request.gateway)?
        }
    };
    let actual_identity = Identity::from_probe(&channel.shell(PROBE, false)?)?;
    expected_identity
        .as_ref()
        .ok_or_else(|| {
            InstallerError::internal(
                "verifying deployment identity",
                "no verified identity in detection plan",
            )
        })?
        .require_same(&actual_identity)?;
    reporter.log(format!(
        "[+] Verified {} ({}) via {}",
        actual_identity.model,
        actual_identity.firmware,
        channel.name()
    ));

    let saved_script = if request.password_action == crate::model::PasswordAction::Keep
        || request.pin_action == crate::model::PinAction::Keep
    {
        channel.shell(&format!("cat {STARTUP_SCRIPT}"), false)?
    } else {
        String::new()
    };
    let credentials = crate::credentials::resolve(&request, &saved_script)?;
    storage_preflight(&channel, &files, &reporter)?;
    if request.dry_run {
        reporter.log("[+] Dry run complete: device identity and installation files verified; no deployment files written");
        return Ok(InstallOutcome {
            verified_identity: Some(actual_identity.clone()),
        device_model: actual_identity.model.clone(), firmware: actual_identity.firmware.clone(), release: release.tag.clone(),
            result: "dryRun".into(), title: "Deployment dry run completed".into(),
            message: format!("Verified {} ({}) and release {}. No installation files or startup settings were changed.", actual_identity.model, actual_identity.firmware, release.tag),
            operation, dashboard_url: None, api_url: None, ssh_address: None, diagnostic_path: None,
        });
    }
    // Repeat after preparation: the transport must still reach the detected unit.
    actual_identity.require_same(&Identity::from_probe(&channel.shell(PROBE, false)?)?)?;
    reporter.control.enter_critical()?;
    reporter.emit(
        "critical",
        "Keep the installer open until verification finishes.",
        None,
        None,
    );
    let transaction =
        crate::transaction::DeploymentTransaction::begin(&channel, &actual_identity, work)?;
    let installation = (|| -> Result<(), InstallerError> {
        reporter.active("agent", "Installing the verified agent…");
        deploy_agent(
            &channel,
            &request.gateway,
            &credentials.password,
            &credentials.pin,
            &files,
            work,
            &reporter,
        )?;
        reporter.step("agent", "complete", "Agent installed and authenticated");

        reporter.active("ssh", "Configuring secure SSH access…");
        harden(&channel, &request.gateway, &files, &reporter)?;
        reporter.step("ssh", "complete", "SSH and startup configuration complete");

        reporter.active("dashboard", "Deploying and verifying the dashboard…");
        deploy_dashboard(&channel, &request.gateway, &files, &reporter)?;
        reporter.step("dashboard", "complete", "Dashboard deployed and verified");

        write_manifest(&channel, &actual_identity, &release.tag, &files, work)?;
        if matches!(&channel, Channel::Adb { .. }) && request.reboot_after {
            reporter.active("reboot", "Rebooting and checking startup over SSH…");
            let previous_boot = channel.shell("cat /proc/sys/kernel/random/boot_id", true)?;
            channel.reboot()?;
            let deadline = Instant::now() + Duration::from_secs(180);
            let ssh = ssh_channel(&request.gateway)?;
            loop {
                if Instant::now() >= deadline {
                    return Err(InstallerError::new(
                        "Post-reboot verification timed out",
                        "Reconnect to the same modem and use the retained recovery snapshot.",
                        transaction.recovery_details(),
                    ));
                }
                std::thread::sleep(Duration::from_secs(3));
                if let Ok(boot) = ssh.shell_timeout(
                    "cat /proc/sys/kernel/random/boot_id",
                    true,
                    Duration::from_secs(8),
                ) {
                    if boot.is_empty() || boot == previous_boot {
                        continue;
                    }
                    actual_identity
                        .require_same(&Identity::from_probe(&ssh.shell(PROBE, false)?)?)?;
                    channel = ssh.clone();
                    if !verify_agent_credentials(
                        &channel,
                        &request.gateway,
                        &credentials.password,
                        &credentials.pin,
                    ) {
                        continue;
                    }
                    if channel.shell("/usr/bin/curl --fail --silent --show-error --connect-timeout 5 --max-time 10 http://127.0.0.1:8080/ | grep -q '<div id=\"root\"></div>'", true).is_err() { continue; }
                    verify_ssh_security(&request.gateway)?;
                    break;
                }
            }
            reporter.step("reboot", "complete", "New boot, device identity, agent authentication, key-only SSH and dashboard verified");
        }
        transaction.complete(&channel)
    })();
    if let Err(mut error) = installation {
        let recovery = transaction.restore(&channel);
        error.guidance = match recovery {
            Ok(()) => "The previous installation was restored. Keep the diagnostic details and detect again before retrying.".into(),
            Err(recovery_error) => format!("Automatic recovery could not finish: {}. {}", recovery_error.summary, transaction.recovery_details()),
        };
        return Err(error);
    }
    reporter.log(format!("[+] {}", transaction.recovery_details()));

    reporter.log("[+] Installation completed successfully");
    Ok(InstallOutcome {
        result: "success".into(),
        verified_identity: Some(actual_identity.clone()),
        device_model: actual_identity.model.clone(),
        firmware: actual_identity.firmware.clone(),
        release: release.tag.clone(),
        title: format!("{} completed", operation.label()),
        message: if request.reboot_after {
            "The modem rebooted and its agent, SSH access and dashboard were verified.".into()
        } else {
            "The agent, secure SSH access, and dashboard are ready.".into()
        },
        operation,
        dashboard_url: Some(format!("http://{}:8080", request.gateway)),
        api_url: Some(format!("http://{}:9090", request.gateway)),
        ssh_address: Some(format!(
            "ssh -i \"{}\" -p 2222 root@{}",
            crate::host::key_path()?.display(),
            request.gateway
        )),
        diagnostic_path: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_known_hosts_supports_windows_paths_with_spaces() {
        let args = ssh_args(
            "192.168.0.1",
            Path::new(r"C:\Users\Sample User\known_hosts"),
            "true",
        );
        assert!(args
            .iter()
            .any(|arg| arg == "UserKnownHostsFile=\"C:/Users/Sample User/known_hosts\""));
    }
    #[test]
    fn ssh_policy_requires_an_explicit_key_only_advertisement() {
        assert!(
            !password_auth_advertised("debug1: Authentications that can continue: publickey")
                .unwrap()
        );
        assert!(password_auth_advertised(
            "debug1: Authentications that can continue: publickey,password"
        )
        .unwrap());
        assert!(password_auth_advertised(
            "debug1: Authentications that can continue: keyboard-interactive"
        )
        .unwrap());
        assert!(password_auth_advertised("connection refused").is_err());
    }
    #[test]
    fn pin_changes_and_clearing_change_the_complete_startup_script() {
        assert_ne!(
            startup_script("password", "123456"),
            startup_script("password", "654321")
        );
        assert!(startup_script("password", "").contains("unset ZTE_AGENT_PIN"));
        assert!(!startup_script("password", "").contains("export ZTE_AGENT_PIN"));
    }
    #[test]
    fn adb_login_uses_device_lan_address_without_forwarding() {
        let command = adb_agent_login_command("192.168.0.1", "quote'password");
        assert!(command.contains("/usr/bin/curl"));
        assert!(command.contains("192.168.0.1:9090/api/auth/login"));
        assert!(!command.contains("adb forward"));
        assert!(command.contains("quote'\\''password"));
    }

    #[test]
    fn source_keeps_explicit_dropbear_and_dashboard_checks() {
        let source = include_str!("deploy.rs");
        assert!(source.contains("[ -f /data/bin/dropbear ] && [ -x /data/bin/dropbear ]"));
        assert!(source.contains("/data/bin/dashboard-uhttpd"));
        assert!(source.contains("start_dashboard.sh"));
        assert!(source.contains("uci -q delete uhttpd.dashboard"));
        assert!(source.contains("http://127.0.0.1:8080/"));
        assert!(source.contains("<div id=\\\"root\\\"></div>"));
    }

    #[test]
    fn shell_quote_handles_credentials_with_quotes() {
        assert_eq!(shell_quote("can't"), "'can'\\''t'");
    }

    #[test]
    fn preserved_credentials_round_trip_the_actual_startup_writer() {
        let mut request = crate::model::sample_request();
        request.password_action = crate::model::PasswordAction::Keep;
        request.pin_action = crate::model::PinAction::Keep;
        for password in ["can't", "quote'\"\\$()\nline", "ordinary-password"] {
            for pin in ["", "123456"] {
                let saved = startup_script(password, pin);
                let credentials = crate::credentials::resolve(&request, &saved).unwrap();
                assert_eq!(credentials.password, password);
                assert_eq!(credentials.pin, pin);
            }
        }
    }
}
