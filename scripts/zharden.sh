#!/bin/bash
# zharden.sh — post-unlock hardening for the MU5250 (U60 Pro) on B04+.
#
# Run AFTER scripts/zunlock.py (adb up) and setup.sh (agent installed).
# Idempotent: safe to re-run anytime; each step no-ops when already done.
#
# What it does (details in COMMUNITY-UNLOCK.md):
#   1. installs dropbear SSH (port 2222, key auth) into /data
#   2. cleans rc.local: keeps stock + agent/dropbear lines, removes usb_op
#      write lines (so every boot = stock ECM tethering; adb on demand)
#   3. adds the dashboard uhttpd instance on :8080
#   4. disables FOTA auto-update
#   5. offers a final reboot into the clean state
#
# DESIGN RULE (2026-07-21): shell/ssh/adb only. This script deliberately
# installs NO boot hooks outside /etc/rc.local and does NOT modify system
# services (no firewall includes/hooks). A boot-time hook that stalls or
# fails can wedge the device before any recovery interface exists. rc.local
# is not FOTA-preserved, so after a firmware update simply re-run:
# zunlock.py -> setup.sh -> zharden.sh (~15 min, see COMMUNITY-UNLOCK.md).
#
# Usage: bash scripts/zharden.sh [--gw 192.168.0.1]
set -euo pipefail

GW="${1:-192.168.0.1}"; GW="${GW#--gw }"; GW="${GW#--gw=}"
SSH_PORT=2222
SSH="ssh -p $SSH_PORT -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile=$HOME/.ssh/known_hosts.d/zte -o ConnectTimeout=5 root@$GW"
DROPBEAR_URL="https://downloads.openwrt.org/releases/23.05.4/targets/armsr/armv8/packages/dropbear_2022.82-6_aarch64_generic.ipk"

info() { echo -e "\033[0;36m[*]\033[0m $1"; }
ok()   { echo -e "\033[0;32m[+]\033[0m $1"; }
warn() { echo -e "\033[1;33m[!]\033[0m $1"; }

# ── Channel: prefer adb (always present right after zunlock) ─────────────
if adb devices 2>/dev/null | grep -q 'device$'; then
  CH=adb; info "channel: adb"
elif $SSH 'true' 2>/dev/null; then
  CH=ssh; info "channel: ssh"
else
  echo "No channel: run scripts/zunlock.py first (adb), or have dropbear up (ssh)." >&2
  exit 1
fi
rcmd() { if [ "$CH" = adb ]; then adb shell "$@"; else $SSH "$@"; fi; }

# ── 1. dropbear into /data ───────────────────────────────────────────────
if rcmd 'test -x /data/bin/dropbear'; then
  ok "dropbear already installed"
else
  info "installing dropbear to /data/bin ..."
  TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
  curl -sfL "$DROPBEAR_URL" -o "$TMP/dropbear.ipk"
  (cd "$TMP" && tar xzf dropbear.ipk data.tar.gz)
  if [ "$CH" = adb ]; then
    adb push "$TMP/data.tar.gz" /tmp/data.tar.gz >/dev/null
    rcmd 'cd /tmp && tar xzf data.tar.gz ./usr/sbin/dropbear ./usr/bin/dbclient ./usr/bin/dropbearkey && mkdir -p /data/bin && cp usr/sbin/dropbear usr/bin/dbclient usr/bin/dropbearkey /data/bin/ && chmod +x /data/bin/* && rm -rf /tmp/usr /tmp/data.tar.gz'
  else
    cat "$TMP/data.tar.gz" | $SSH 'cat > /tmp/data.tar.gz; cd /tmp && tar xzf data.tar.gz ./usr/sbin/dropbear ./usr/bin/dbclient ./usr/bin/dropbearkey && mkdir -p /data/bin && cp usr/sbin/dropbear usr/bin/dbclient usr/bin/dropbearkey /data/bin/ && chmod +x /data/bin/* && rm -rf /tmp/usr /tmp/data.tar.gz'
  fi
  rcmd 'test -x /data/bin/dropbear'
  ok "dropbear installed (manual ipk extract — opkg is unusable on this firmware)"
fi

# ssh key + host keys + authorized_keys
[ -f "$HOME/.ssh/id_ed25519" ] || ssh-keygen -t ed25519 -f "$HOME/.ssh/id_ed25519" -N "" >/dev/null
rcmd 'mkdir -p /etc/dropbear /data/dropbear && chmod 700 /etc/dropbear'
PUB=$(cat "$HOME/.ssh/id_ed25519.pub")
rcmd "grep -qF '$PUB' /etc/dropbear/authorized_keys 2>/dev/null || echo '$PUB' >> /etc/dropbear/authorized_keys; chmod 600 /etc/dropbear/authorized_keys"
rcmd 'for k in ed25519 rsa; do f=/etc/dropbear/dropbear_${k}_host_key; [ -s "$f" ] || /data/bin/dropbearkey -t $k -f $f >/dev/null 2>&1; done'
rcmd 'cp /etc/dropbear/authorized_keys /etc/dropbear/dropbear_*_host_key /data/dropbear/ 2>/dev/null; chmod 600 /data/dropbear/*'
rcmd 'printf "#!/bin/sh\n/data/bin/dropbear -p 2222 -r /etc/dropbear/dropbear_ed25519_host_key -r /etc/dropbear/dropbear_rsa_host_key\n" > /data/local/tmp/start_dropbear.sh && chmod +x /data/local/tmp/start_dropbear.sh'
ok "ssh keys, host keys, startup script in place"

# ── 2. rc.local: service lines present, usb_op writes removed ────────────
rcmd '
grep -qF "start_zte_agent.sh" /etc/rc.local || sed -i "/^exit 0/i sh /data/local/tmp/start_zte_agent.sh" /etc/rc.local
grep -qF "start_dropbear.sh" /etc/rc.local || sed -i "/^exit 0/i sh /data/local/tmp/start_dropbear.sh" /etc/rc.local
# remove only usb_op WRITE lines (echo 1 > ...usb_op); the stock flash-protect
# block READS usb_op and must stay (deleting it breaks rc.local syntax)
sed -i "/^echo [0-9] > .*usb_op/d" /etc/rc.local
sh -n /etc/rc.local'
ok "rc.local: agent+dropbear lines present, usb_op writes gone, syntax OK"

# ── 3. dashboard uhttpd instance ─────────────────────────────────────────
rcmd 'uci -q get uhttpd.dashboard >/dev/null 2>&1 || {
  uci set uhttpd.dashboard=uhttpd
  uci set uhttpd.dashboard.listen_http="0.0.0.0:8080"
  uci set uhttpd.dashboard.home="/data/www"
  uci set uhttpd.dashboard.no_dirlists="1"
  uci commit uhttpd
}; /etc/init.d/uhttpd restart 2>/dev/null; true'
ok "dashboard instance on :8080"

# ── 4. auto-update OFF ───────────────────────────────────────────────────
rcmd 'ubus call zwrt_zte_dm set_update_mode "{\"dm_update_mode\":\"0\"}" >/dev/null 2>&1; uci get zwrt_zte_dm.dm_update.dm_update_mode' | grep -q 0 \
  && ok "FOTA auto-update disabled" || warn "could not confirm dm_update_mode=0 — check manually"

# ── 5. start dropbear now + verify ssh ───────────────────────────────────
rcmd 'pidof dropbear >/dev/null 2>&1 || sh /data/local/tmp/start_dropbear.sh'
sleep 2
if $SSH 'echo ok' >/dev/null 2>&1; then
  ok "SSH verified: ssh -p 2222 root@$GW"
else
  warn "SSH not yet reachable (firewall may need a reload, or reboot once)"
fi

echo ""
ok "Hardening complete. Every boot = stock ECM tethering + agent :9090 + ssh :2222."
echo "    Dashboard: http://$GW:8080   (deploy with: bash deploy-dashboard.sh)"
echo "    ADB on demand: ssh -p 2222 root@$GW 'echo 1 > /sys/class/android_usb/android0/usb_op' + reboot"
if [ "$CH" = adb ]; then
  echo ""
  echo "Reboot now to drop the ADB composition and return USB tethering? [y/N]"
  read -r ANS
  if [ "$ANS" = y ] || [ "$ANS" = Y ]; then
    adb reboot
    echo "Rebooting — ~90s. Verify afterwards: ping $GW, then ssh -p 2222 root@$GW"
  fi
fi
