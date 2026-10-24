#!/usr/bin/env python3
"""Reject device identity values and backup-key material in tracked files."""

from __future__ import annotations

import re
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

DEVICE_ID = re.compile(
    rb"(?i)\b(?:imei|imsi|iccid|sim_imei|sim_imsi|sim_iccid)\b"
    rb"[^\r\n]{0,80}?[\"']([0-9]{14,22})[\"']"
)
LITERAL_SUFFIX = re.compile(
    rb"(?im)\bZTE_BACKUP_SUFFIX\s*=\s*([\"'])([^\"'\r\n]+)\1"
)
# Test fixtures use obvious placeholders; real identifiers never hold ten zeros in a row.
SYNTHETIC_ID = re.compile(rb"0{10}")
PRIVATE_KEY = re.compile(rb"-----BEGIN (?:[A-Z ]+ )?PRIVATE KEY-----")
TOKEN = re.compile(
    rb"(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|"
    rb"AKIA[0-9A-Z]{16}|AIza[0-9A-Za-z_-]{20,}|sk-[A-Za-z0-9]{20,}|"
    rb"sk-(?:proj|svcacct)-[A-Za-z0-9_-]{20,})"
)


def candidate_files() -> list[Path]:
    result = subprocess.run(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        cwd=ROOT,
        check=True,
        stdout=subprocess.PIPE,
    )
    return [ROOT / item.decode() for item in result.stdout.split(b"\0") if item]


def main() -> int:
    failures: list[str] = []
    for path in candidate_files():
        try:
            data = path.read_bytes()
        except (FileNotFoundError, IsADirectoryError):
            continue

        if any(not SYNTHETIC_ID.search(m.group(1)) for m in DEVICE_ID.finditer(data)):
            failures.append(f"{path.relative_to(ROOT)}: hard-coded device identity")
        for match in LITERAL_SUFFIX.finditer(data):
            value = match.group(2).strip()
            if value and not value.startswith((b"$", b"<")):
                failures.append(f"{path.relative_to(ROOT)}: literal ZTE backup suffix")
                break
        if PRIVATE_KEY.search(data):
            failures.append(f"{path.relative_to(ROOT)}: private key material")
        if TOKEN.search(data):
            failures.append(f"{path.relative_to(ROOT)}: credential-like token")

    if failures:
        print("Device-secret audit failed:")
        for failure in failures:
            print(f"  {failure}")
        return 1

    print("OK: no tracked device identities, ZTE backup suffixes, private keys, or credential tokens")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
