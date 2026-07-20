# MU5250-OpenUI

Credit to: https://github.com/jesther-ai/open-u60-pro (which this project is based on).

A custom control plane for the ZTE U60 Pro (MU5250): a Rust agent on the modem
exposing a JSON API (`http://192.168.0.1:9090`), a React dashboard served from
the modem (`http://192.168.0.1:8080`), and tooling to provision and update both.

## Unlocking locked firmware (B04 / CN B28+)

**Newer firmware locked ADB away — this repo brings it back.** ZTE removed the
web-accessible USB-debug toggle (`zwrt_bsp.usb.set`) in HK
`BD_XCBZHKMU5250V1.0.0B04` and CN `B28` (on B04 the method is deleted from the
daemon itself, not just hidden from the web ACL).

The way back in is the device's own **config backup/restore path**: the backup
is an openssl-encrypted config tar, and restore runs as root and extracts
whatever it validates. [`scripts/zunlock.py`](scripts/zunlock.py) automates the
whole flow:

1. logs in and downloads a fresh config backup
2. decrypts it (`des-ede3-cbc` / sha256), inserts a boot-time USB-debug line
   into `etc/rc.local` — the sysfs path is auto-discovered from the device's
   own backup, no firmware-specific constants
3. repacks it byte-compatible with the device's own md5-checked format,
   re-encrypts, uploads (sha256-verified), triggers restore
4. the device reboots and comes back with **adbd enabled** — root shell via
   `adb shell`, ready for `setup.sh`

```sh
python3 scripts/zunlock.py --dry-run   # validate everything except the upload
python3 scripts/zunlock.py             # full run, asks before restoring
```

You need your router admin password and the platform backup-key **suffix**
(deliberately not published — see the guide for how to obtain it). Full
instructions, safety notes, and post-unlock hardening (including how to make
the install survive future FOTA updates):
**[COMMUNITY-UNLOCK.md](COMMUNITY-UNLOCK.md)**.

## The agent

Rust backend on the modem (`agent/`), talks to ubus, AT ports, sysfs/procfs
and device services. `agent/src/server.rs` is the canonical routing table.

- binds `192.168.0.1:9090` (override `ZTE_AGENT_BIND`, `ZTE_AGENT_THREADS`)
- auth: `POST /api/auth/login` (password from `ZTE_AGENT_PASSWORD`, or an
  optional 6-digit mobile PIN); bearer tokens, 1h TTL, rate-limited login,
  LAN-only CORS; destructive actions require `X-Confirm: true`
- JSON envelope: `{ "ok": true, "data": … }` / `{ "ok": false, "error": … }`

Endpoint families (see `server.rs` for the full table):

- **status**: dashboard, device, battery, CPU, memory, thermal, system top
- **network**: signal, traffic/speed, WAN/LAN, clients, rmnet
- **Wi-Fi**: status/settings, per-band SSID/key/channel/bandwidth/TX power, guest
- **modem**: data usage, online/airplane, network mode, scan + manual register
- **cell/band lock**: NR & LTE band locks, PCI+ARFCN cell locks, neighbor scans,
  STC params, signal-quality detection
- **router services**: DNS, LAN, firewall/NAT/DMZ/UPnP/port-forward/filter,
  VPN, QoS, domain filter, full APN profile management
- **SMS**: mailbox operations, forwarding rules/log/retry
- **SIM/calls**: SIM info, PIN/PUK, network unlock, calls/USSD/STK
- **extras**: DoH proxy, speed test, LAN tests, scheduler, TTL clamping,
  read-only AT console, signal/connection CSV loggers, USB mode switch

## The dashboard

React/Vite SPA (`web-app/`), talks to the agent at `http://<host>:9090`,
tokens in `sessionStorage`. Pages shipped today (`web-app/src/App.tsx`):

- **Dashboard** — signal, battery, live rates, device, WAN, data usage
- **Signal** — per-carrier LTE/NR breakdown (PCI, ARFCN, RSRP/RSRQ/SINR)
- **Connected** — clients by Wi-Fi/USB-C/wired with link details
- **Wi-Fi** — per-band configuration incl. persistence and TX power
- **Router** — LAN/DHCP/DNS with presets
- **Modem** — APN profiles and TTL clamping
- **Band & Cell Locking** — network mode, band/cell locks, one-click from live cells
- **Metrics** — full thermal map, battery health, fuel gauge
- **Advanced** — signal/connection loggers, AT console
- **Settings** — device/SIM info, agent and device power actions, USB mode

Backend capabilities not yet surfaced in the nav include SMS + forwarding,
DoH, speed test, scheduler, SIM PIN flows, calls/USSD/STK, and advanced
firewall/QoS controls (see "API Features Not Yet Surfaced" in git history or
`server.rs`).

## Development and deployment

- `./setup.sh` — first-time provisioning (build/download agent, push via adb,
  boot persistence, optional dropbear SSH on port 2222)
- `./deploy.sh` — push agent updates (SSH or adb)
- `./deploy-dashboard.sh` — build and push the web app (ssh tar pipe)
- `scripts/` — recon and unlock tooling (`zunlock.py`, `zharden.sh`, `zbackup.py`, …)

## Source of truth

If this README and the code ever disagree:

- `agent/src/server.rs`: HTTP routing table
- `agent/src/auth.rs`: auth and token behavior
- `web-app/src/App.tsx`: pages actually mounted in the UI
- `web-app/src/api.ts`: client-side API bindings and payload shapes
