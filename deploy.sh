#!/bin/bash
set -euo pipefail
ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
cd "$ROOT_DIR"
echo 'Building agent and eSIM helper…'
cargo build --locked --release --target aarch64-unknown-linux-musl -p zte-agent -p zte-esim
RELEASE="$ROOT_DIR/target/aarch64-unknown-linux-musl/release"
exec python3 "$ROOT_DIR/scripts/deploy-components.py" --agent "$RELEASE/zte-agent" --esim-helper "$RELEASE/zte-esim" "$@"
