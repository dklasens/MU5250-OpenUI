# Unlocking the ZTE U60 Pro (MU5250) on locked firmware (B04 and similar)

Newer MU5250 firmware (HK `BD_XCBZHKMU5250V1.0.0B04`, CN `B28+`) removed the
web-accessible USB-debug toggle (`zwrt_bsp.usb.set`) — on B04 the method is
deleted from the daemon itself, so no web trick can re-enable ADB.

What still works is the **config backup/restore path**: the backup is an
openssl-encrypted tar of the system config, and the restore process runs as
root and extracts whatever you give it. `scripts/zunlock.py` uses that to
plant one line in `etc/rc.local` that re-enables the USB debug composition
(adbd) at boot.

The script is fully self-contained (Python 3 stdlib + `openssl` CLI) and
contains **no secrets**: the backup-key suffix is an input, the device IMEI
is read from the device, and the USB-debug sysfs path is discovered from the
device's own stock `rc.local` inside the backup.

## Requirements

- The router's **admin password** (you set it — it's your web UI login)
- The **backup-key suffix** for this device family (see below)
- A computer on the device's network (WiFi or USB), Python 3, openssl
- `adb` installed for afterwards (`brew install android-platform-tools`)

## The backup-key suffix

The backup encryption password is `<device IMEI><suffix>` — the IMEI is
per-device (the script reads it itself), the suffix is a fixed string shared
across this ZTE platform generation. It is deliberately **not** published
here, at the request of the researchers who shared it (publishing it gets it
killed in the next firmware). To obtain it:

- ask the community — e.g. the
  [`amenekowo/mu5250_tweaking`](https://github.com/amenekowo/mu5250_tweaking)
  issue tracker or related Discords, or
- extract it yourself from a rooted SDX75-era ZTE MBB unit: the web server
  binary (`zte_web`) builds the backup password in memory; the suffix is
  visible in its strings near the backup/restore code paths.

Pass it via `--suffix`, the `ZTE_BACKUP_SUFFIX` env var, or the interactive
(hidden) prompt. It never touches this repo.

## Usage

```sh
python3 scripts/zunlock.py --dry-run     # everything except the upload (safe)
python3 scripts/zunlock.py               # full run, asks before restoring
```

What happens on a full run:

1. logs into the web UI, requests a fresh config backup, downloads it
2. decrypts it (`openssl enc -d -des-ede3-cbc -md sha256`)
3. inserts the USB-debug line into `etc/rc.local` (right after the shebang,
   path discovered from the stock file), preserving file modes/ownership
4. rebuilds the package exactly as the device does (inner tgz → md5 sidecar →
   outer tgz → re-encrypt) — passes the device's own restore-time md5 check
5. uploads it (`/cgi-bin/cgi-upload`, verifies the server's sha256 matches),
   triggers `device_restore_proc` — the device restores and reboots
6. ~60–90s later: `adb devices` shows the unit (serial `0123456789ABCDEF`),
   root shell via `adb shell`

Your settings are preserved — the patched package is built from a backup
taken seconds earlier.

## After the unlock

ADB is a bootstrap channel, not a good permanent interface (its composition
drops USB networking on this firmware, and it only applies at boot). For a
durable setup:

- install dropbear (SSH) into `/data` — the rootfs is read-only except `/etc`
  and `/data`, and `/data` survives firmware updates
- start your services from `/etc/rc.local`
- **protect against the next update**: FOTA preserves the UCI config dir and
  `/data`, but not `rc.local` — hook boot via a `config include` section in
  `/etc/config/firewall` pointing at a script in `/data` that self-heals the
  rest, and disable auto-update:
  `ubus call zwrt_zte_dm set_update_mode '{"dm_update_mode":"0"}'`

## Safety

- The restore reboots the device and briefly interrupts connectivity (~90s).
- The script verifies the upload hash before triggering anything; a mismatch
  aborts before any state change.
- Never write the USB composition node manually outside boot time (live
  writes can kill the gadget until reboot), and never experiment with A/B
  slot switching (`abctl --set_active`) — mixed-slot boots can brick the unit.

## Credits

Backup-crypto details and the original payload hint: the
`amenekowo/mu5250_tweaking` community (with thanks — they asked that the key
material itself not be republished, and this tool honors that).
B04 daemon/ACL analysis: community contributors on the issue tracker.
