#!/usr/bin/env python3
"""zbackup.py — decrypt / inspect / patch / re-encrypt the MU5250 config
backup (`back_parameter`) for the ADB-restore route.

Real on-device format (from backup_config.sh, shared by amenekowo):
  back_parameter          = openssl enc -des-ede3-cbc -md sha256
                            -pass pass:"<IMEI><suffix>"
    └─ outer .tgz         = tmp/back_parameter_r1.tgz + tmp/back_parameter_r.md5
       └─ inner r1.tgz    = tar of /etc conffiles (sysupgrade keep-list);
                            restore extracts it with `tar -C /` after an
                            md5 check against the .md5 file.

Password goes via env ZBACKUP_PW or --pw — never hardcode it here.
The <suffix> is a fixed ZTE platform string; obtain it from the community
(see COMMUNITY-UNLOCK.md).

Usage:
  export ZBACKUP_PW="<IMEI><suffix>"
  zbackup.py dec   back_parameter outer.tgz
  zbackup.py peek  outer.tgz
  zbackup.py patch outer.tgz patched_outer.tgz
  zbackup.py enc   patched_outer.tgz new_back_parameter
"""

import argparse
import hashlib
import io
import os
import subprocess
import sys
import tarfile

# Boot payload: force USB debug composition (adbd). Path matches the one the
# stock rc.local itself reads (= /sys/devices/virtual/android_usb/android0/usb_op).
PAYLOAD = "echo 1 > /sys/class/android_usb/android0/usb_op"
RC_LOCAL = "etc/rc.local"
INNER = "tmp/back_parameter_r1.tgz"
MD5 = "tmp/back_parameter_r.md5"


def openssl(direction, src, dst, args):
    pw = args.pw or os.environ.get("ZBACKUP_PW")
    if not pw:
        sys.exit("error: set ZBACKUP_PW or pass --pw")
    cmd = ["openssl", "enc", f"-{args.cipher}", f"-{direction}",
           "-pass", f"pass:{pw}", "-md", args.md, "-in", src, "-out", dst]
    subprocess.run(cmd, check=True)
    with open(dst, "rb") as f:
        head = f.read(265)
    kind = ("gzip" if head[:2] == b"\x1f\x8b" else
            "tar" if head[257:262] == b"ustar" else "unknown")
    print(f"{dst}: {os.path.getsize(dst)} bytes, looks like {kind}")


def cmd_dec(args):
    openssl("d", args.src, args.dst, args)


def cmd_enc(args):
    openssl("e", args.src, args.dst, args)


def read_outer(path):
    """Return (inner_tgz_bytes, [(TarInfo, bytes|None), ...] of inner members)."""
    with tarfile.open(path, "r:gz") as t:
        inner = t.extractfile(INNER).read()
    with tarfile.open(fileobj=io.BytesIO(inner), mode="r:gz") as t:
        members = [(m, t.extractfile(m).read() if m.isfile() else None)
                   for m in t.getmembers()]
    return inner, members


def cmd_peek(args):
    inner, members = read_outer(args.src)
    print(f"inner tgz: {len(inner)} bytes, {len(members)} members, "
          f"md5 {hashlib.md5(inner).hexdigest()}")
    with tarfile.open(args.src, "r:gz") as t:
        stored = t.extractfile(MD5).read().decode().strip()
    print(f"stored md5: {stored} ({'MATCH' if stored == hashlib.md5(inner).hexdigest() else 'MISMATCH!'})")
    for m, data in members:
        if m.name == RC_LOCAL and m.isfile():
            print(f"--- {m.name} (mode {oct(m.mode)}, uid {m.uid}, gid {m.gid}) ---")
            print(data.decode("utf-8", "replace"))
            return
    print(f"(no {RC_LOCAL} in archive)")


def cmd_patch(args):
    _, members = read_outer(args.src)
    out, rc = [], None
    for m, data in members:
        if m.name == RC_LOCAL and m.isfile():
            rc = (m, data)
            continue  # re-appended modified below; last member wins on extract
        out.append((m, data))
    if rc is None:
        sys.exit(f"error: {RC_LOCAL} not in archive (unexpected on MU5250)")

    m, data = rc
    text = data.decode("utf-8", "replace")
    if PAYLOAD in text:
        sys.exit("rc.local already contains the payload — nothing to do")
    lines = text.splitlines()
    for i, line in enumerate(lines):  # insert right after the shebang
        if line.strip() == "#!/bin/sh":
            lines.insert(i + 1, PAYLOAD)
            break
    else:
        lines.insert(0, PAYLOAD)
    new = ("\n".join(lines) + "\n").encode()
    m.size = len(new)
    out.append((m, new))
    print(f"patched {m.name}: payload after shebang (mode {oct(m.mode)}, uid {m.uid})")

    ibuf = io.BytesIO()
    with tarfile.open(fileobj=ibuf, mode="w:gz") as t:
        for mi, d in out:
            t.addfile(mi, io.BytesIO(d) if d is not None else None)
    inner = ibuf.getvalue()
    digest = hashlib.md5(inner).hexdigest()
    print(f"new inner tgz: {len(inner)} bytes, md5 {digest}")

    def meta(name, size):
        mi = tarfile.TarInfo(name)
        mi.mode, mi.uid, mi.gid, mi.uname, mi.gname = 0o644, 0, 0, "root", "root"
        mi.size = size
        return mi

    with tarfile.open(args.dst, "w:gz") as t:
        t.addfile(meta(INNER, len(inner)), io.BytesIO(inner))
        md5f = (digest + "\n").encode()
        t.addfile(meta(MD5, len(md5f)), io.BytesIO(md5f))
    print(f"wrote {args.dst}")


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--pw", help="backup password (default: env ZBACKUP_PW)")
    ap.add_argument("--cipher", default="des-ede3-cbc")
    ap.add_argument("--md", default="sha256")
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name, fn, a, b in [("dec", cmd_dec, "encrypted backup", "output outer .tgz"),
                           ("enc", cmd_enc, "outer .tgz", "encrypted output")]:
        p = sub.add_parser(name)
        p.add_argument("src", help=a)
        p.add_argument("dst", help=b)
        p.set_defaults(fn=fn)
    p = sub.add_parser("peek")
    p.add_argument("src", help="outer .tgz")
    p.set_defaults(fn=cmd_peek)
    p = sub.add_parser("patch")
    p.add_argument("src", help="input outer .tgz")
    p.add_argument("dst", help="patched outer .tgz")
    p.set_defaults(fn=cmd_patch)
    args = ap.parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
