#!/bin/bash
set -e

# Default settings based on deploy.sh
DEVICE="192.168.0.1"
SSH_PORT=2222
TARGET_DIR=${1:-"/data/www"}

# Build the dashboard first
echo "Building the dashboard..."
cd web-app
npm run build
cd ..

# Ensure build succeeded
if [ ! -d "web-app/dist" ]; then
    echo "Error: web-app/dist directory not found. Build failed?"
    exit 1
fi

echo "Deploying dashboard to root@$DEVICE:$TARGET_DIR via SSH tar pipe (Port $SSH_PORT)..."

# The device's dropbear has no sftp-server and no remote scp binary, so scp
# fails. Stream the dist tree over ssh instead (same pattern as setup.sh).
SSH="ssh -p $SSH_PORT -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile=$HOME/.ssh/known_hosts.d/zte root@$DEVICE"
tar czf - -C web-app/dist . | $SSH "mkdir -p $TARGET_DIR && tar xzf - -C $TARGET_DIR"

# ZTE's patched uhttpd redirects mobile user-agents to /mobile.html.
# Copy index.html so the SPA loads on phones and tablets too.
$SSH "cp $TARGET_DIR/index.html $TARGET_DIR/mobile.html"

echo "Dashboard deploy successful!"
