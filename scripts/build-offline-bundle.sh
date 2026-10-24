#!/bin/bash
# Prepare this working tree for installer testing without publishing a release.
set -euo pipefail
umask 077
ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
DESTINATION=${1:-"$ROOT_DIR/dist/install-bundle"}
[ ! -e "$DESTINATION" ] || { echo 'Choose a new output directory; an existing bundle will not be overwritten.' >&2; exit 1; }
mkdir -p "$(dirname "$DESTINATION")"
WORK=$(mktemp -d "$(dirname "$DESTINATION")/.bundle-XXXXXX")
trap 'rm -rf "$WORK"' EXIT
cd "$ROOT_DIR"
cargo build --locked --release --target aarch64-unknown-linux-musl -p zte-agent -p zte-esim
(cd web-app && npm ci && npm run build)
python3 scripts/deploy-components.py --prepare-only --bundle-output "$WORK"
cp target/aarch64-unknown-linux-musl/release/zte-agent "$WORK/zte-agent"
cp target/aarch64-unknown-linux-musl/release/zte-esim "$WORK/zte-esim"
tar czf "$WORK/dashboard-dist.tar.gz" -C web-app/dist .
python3 - "$WORK" <<'PY'
import hashlib,json,sys
from pathlib import Path
root=Path(sys.argv[1])
checksums={name:hashlib.sha256((root/name).read_bytes()).hexdigest() for name in ['zte-agent','zte-esim','dashboard-dist.tar.gz']}
(root/'sha256sums.txt').write_text(''.join(f'{checksum}  {name}\n' for name,checksum in checksums.items()))
files={name:hashlib.sha256((root/name).read_bytes()).hexdigest() for name in ['zte-agent','zte-esim','dashboard-dist.tar.gz','sha256sums.txt','dropbear.ipk','uhttpd.ipk']}
(root/'bundle.json').write_text(json.dumps({'format_version':1,'release':'local working tree','files':files},indent=2))
PY
mv "$WORK" "$DESTINATION"
echo "Offline bundle ready: $DESTINATION"
