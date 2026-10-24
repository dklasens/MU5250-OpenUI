//! Validated offline artifacts and archive resource limits.
use crate::model::InstallerError;
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};

pub const MAX_DOWNLOAD: u64 = 64 * 1024 * 1024;
const MAX_EXPANDED: u64 = 64 * 1024 * 1024;
const NAMES: [&str; 6] = [
    "sha256sums.txt",
    "zte-agent",
    "zte-esim",
    "dashboard-dist.tar.gz",
    "dropbear.ipk",
    "uhttpd.ipk",
];
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format_version: u32,
    release: String,
    files: BTreeMap<String, String>,
}
fn error(detail: impl ToString) -> InstallerError {
    InstallerError::new(
        "The installation bundle is invalid",
        "Use a complete verified bundle exported by this installer, or download again.",
        detail.to_string(),
    )
}
pub fn compatible_release(release: &str) -> Result<(), InstallerError> {
    let version = release.strip_prefix('v').unwrap_or(release);
    let parts: Vec<_> = version.split('.').collect();
    let installer: Vec<_> = env!("CARGO_PKG_VERSION").split('.').collect();
    if !(2..=3).contains(&parts.len())
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
        || parts[..2] != installer[..2]
    {
        return Err(InstallerError::new("This software release needs a matching installer", "Download the installer published with the selected agent/dashboard release, then check again.", "Installer and payload major/minor versions must match; prerelease payloads are not accepted."));
    }
    Ok(())
}

pub fn find_cached(release: Option<&str>) -> Option<PathBuf> {
    let parent = dirs::cache_dir()?.join("open-u60-pro/bundles");
    let mut entries: Vec<_> = fs::read_dir(parent)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .collect();
    entries.sort_by_key(|entry| {
        std::cmp::Reverse(entry.metadata().ok().and_then(|m| m.modified().ok()))
    });
    entries.into_iter().find_map(|entry| {
        let path = entry.path();
        let tag = load(&path).ok()?;
        release
            .is_none_or(|expected| expected == tag)
            .then_some(path)
    })
}

pub fn read_limited(path: &Path) -> Result<Vec<u8>, InstallerError> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(error)?
        .take(MAX_DOWNLOAD + 1)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    if bytes.len() as u64 > MAX_DOWNLOAD {
        return Err(error("file exceeds 64 MiB"));
    }
    Ok(bytes)
}
pub fn load(path: &Path) -> Result<String, InstallerError> {
    let manifest: Manifest =
        serde_json::from_slice(&read_limited(&path.join("bundle.json"))?).map_err(error)?;
    if manifest.format_version != 1
        || manifest.files.len() != NAMES.len()
        || manifest.release.is_empty()
    {
        return Err(error("unsupported bundle manifest"));
    }
    for name in NAMES {
        let actual = hex::encode(Sha256::digest(read_limited(&path.join(name))?));
        if manifest.files.get(name) != Some(&actual) {
            return Err(error(format!("checksum mismatch for {name}")));
        }
    }
    compatible_release(&manifest.release)?;
    Ok(manifest.release)
}
pub fn export(work: &Path, release: &str) -> Result<PathBuf, InstallerError> {
    let parent = dirs::cache_dir()
        .ok_or_else(|| error("cache directory unavailable"))?
        .join("open-u60-pro/bundles");
    fs::create_dir_all(&parent).map_err(error)?;
    let temporary = tempfile::Builder::new()
        .prefix(".preparing-")
        .tempdir_in(&parent)
        .map_err(error)?;
    let mut files = BTreeMap::new();
    for name in NAMES {
        let bytes = read_limited(&work.join(name))?;
        files.insert(name.to_owned(), hex::encode(Sha256::digest(&bytes)));
        fs::write(temporary.path().join(name), bytes).map_err(error)?;
    }
    let manifest = Manifest {
        format_version: 1,
        release: release.into(),
        files,
    };
    fs::write(
        temporary.path().join("bundle.json"),
        serde_json::to_vec_pretty(&manifest).map_err(error)?,
    )
    .map_err(error)?;
    let encoded = serde_json::to_vec(&manifest).map_err(error)?;
    let destination = parent.join(hex::encode(Sha256::digest(&encoded)));
    if destination.exists() {
        load(&destination)?;
        return Ok(destination);
    }
    fs::rename(temporary.path(), &destination).map_err(error)?;
    Ok(destination)
}

pub fn validate_agent(bytes: &[u8]) -> Result<(), InstallerError> {
    if bytes.len() < 64
        || &bytes[..6] != b"\x7fELF\x02\x01"
        || u16::from_le_bytes([bytes[18], bytes[19]]) != 183
    {
        return Err(error(
            "agent must be an aarch64 ELF64 little-endian executable",
        ));
    }
    Ok(())
}

/// Dashboard payloads need regular files and directories only. Validate the
/// entire compressed stream, paths, duplicates and expansion before extraction.
pub fn validate_dashboard(bytes: &[u8]) -> Result<u64, InstallerError> {
    let decoder = GzDecoder::new(Cursor::new(bytes));
    let mut archive = tar::Archive::new(decoder.take(MAX_EXPANDED + 1024 * 1024));
    let mut total = 0_u64;
    let mut paths = HashSet::new();
    let mut has_index = false;
    for item in archive.entries().map_err(error)? {
        let mut entry = item.map_err(error)?;
        let path = entry.path().map_err(error)?.into_owned();
        if path
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
            || path.as_os_str().to_string_lossy().contains('\\')
        {
            return Err(error("dashboard archive has an unsafe path"));
        }
        let path: PathBuf = path
            .components()
            .filter(|part| !matches!(part, Component::CurDir))
            .collect();
        let kind = entry.header().entry_type();
        if !kind.is_file() && !kind.is_dir() {
            return Err(error("dashboard archive contains a link or special file"));
        }
        if !paths.insert(path.clone()) || paths.len() > 10_000 {
            return Err(error(
                "dashboard archive has duplicate or excessive entries",
            ));
        }
        let size = entry.size();
        total = total
            .checked_add(size)
            .ok_or_else(|| error("archive expansion overflow"))?;
        if size > 16 * 1024 * 1024 || total > MAX_EXPANDED {
            return Err(error("dashboard archive exceeds expansion limits"));
        }
        if path == Path::new("index.html") && kind.is_file() && size > 0 {
            has_index = true;
        }
        std::io::copy(&mut entry, &mut std::io::sink()).map_err(error)?;
    }
    if !has_index {
        return Err(error("dashboard archive has no nonempty index.html"));
    }
    Ok(total)
}

/// Extract only explicitly named regular program files from pinned packages;
/// package paths and symlinks are never materialised on the host or modem.
pub fn package_file(bytes: &[u8], wanted: &str) -> Result<Vec<u8>, InstallerError> {
    let decoder = GzDecoder::new(Cursor::new(bytes));
    let mut archive = tar::Archive::new(decoder.take(MAX_EXPANDED));
    let mut found = None;
    for item in archive.entries().map_err(error)? {
        let mut entry = item.map_err(error)?;
        let path = entry.path().map_err(error)?;
        let normalized: PathBuf = path
            .components()
            .filter(|p| !matches!(p, Component::CurDir))
            .collect();
        if normalized == Path::new(wanted) {
            if !entry.header().entry_type().is_file()
                || entry.size() > 8 * 1024 * 1024
                || found.is_some()
            {
                return Err(error(format!("invalid package file {wanted}")));
            }
            let mut data = Vec::new();
            entry.read_to_end(&mut data).map_err(error)?;
            found = Some(data);
        }
    }
    found.ok_or_else(|| error(format!("package file {wanted} missing")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::GzEncoder, Compression};
    fn archive(entries: &[(&str, tar::EntryType)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
        for (name, kind) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(*kind);
            header.set_mode(0o644);
            header.set_size(1);
            header.set_cksum();
            builder.append_data(&mut header, name, &b"x"[..]).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }
    #[test]
    fn rejects_unreviewed_payload_protocol_versions() {
        assert!(compatible_release(&format!("v{}", env!("CARGO_PKG_VERSION"))).is_ok());
        for tag in ["v999.0", "v1.0", "v2.4-beta", "../../payload", "v2", ""] {
            assert!(compatible_release(tag).is_err());
        }
    }
    #[test]
    fn rejects_links_duplicates_and_missing_index() {
        assert!(validate_dashboard(&archive(&[("index.html", tar::EntryType::Regular)])).is_ok());
        assert!(validate_dashboard(&archive(&[("index.html", tar::EntryType::Symlink)])).is_err());
        assert!(validate_dashboard(&archive(&[
            ("index.html", tar::EntryType::Regular),
            ("index.html", tar::EntryType::Regular)
        ]))
        .is_err());
        assert!(validate_dashboard(&archive(&[("other.html", tar::EntryType::Regular)])).is_err());
    }
    #[test]
    fn corrupted_and_incomplete_bundles_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let mut files = BTreeMap::new();
        for name in NAMES {
            fs::write(temp.path().join(name), name).unwrap();
            files.insert(name.into(), hex::encode(Sha256::digest(name.as_bytes())));
        }
        let manifest = Manifest {
            format_version: 1,
            release: format!("v{}", env!("CARGO_PKG_VERSION")),
            files,
        };
        fs::write(
            temp.path().join("bundle.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert_eq!(
            load(temp.path()).unwrap(),
            format!("v{}", env!("CARGO_PKG_VERSION"))
        );
        fs::write(temp.path().join("zte-agent"), "corruption").unwrap();
        assert!(load(temp.path()).is_err());
    }
}
