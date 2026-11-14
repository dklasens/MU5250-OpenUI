# MU5250-OpenUI

A custom control plane for the ZTE U60 Pro (MU5250) 5G modem: a Rust agent
running on the device exposes a JSON API (`http://192.168.0.1:9090`), and a
React dashboard served from the device (`http://192.168.0.1:8080`) turns it
into a full-featured modem management UI — plus tooling to unlock, provision
and update both.

Credit: based on [jesther-ai/open-u60-pro](https://github.com/jesther-ai/open-u60-pro).

<img width="2730" height="1708" alt="Dashboard-Home" src="https://github.com/user-attachments/assets/ed8594a8-af21-4b97-a898-531c7bb6c03a" />
<img width="2734" height="1708" alt="Signal" src="https://github.com/user-attachments/assets/793c3797-b8d7-4933-bf28-b7c3a4650f72" />
<img width="2730" height="1706" alt="BandLock" src="https://github.com/user-attachments/assets/0f911d00-9604-44a8-b0fb-b20cd9fa10c5" />



## Architecture

```
Browser ── HTTP/JSON ──► React dashboard (:8080, isolated uhttpd, /data/www.current)
                              │
                              │ bearer-token JSON API
                              ▼
                         zte-agent (:9090, Rust, tiny_http thread pool)
                              │
              ┌───────────────┼────────────────────────┐
              ▼               ▼                        ▼
        ubus / uci        AT ports              sysfs / procfs
     (wifi, router,     (signal, SMS,        (thermals, battery,
      clients, WAN)      cell/band lock)      charge control)
```

- **Agent (`agent/`)** — Rust HTTP backend, no async runtime, minimal deps
  (`serde`, `tiny_http`, `sha2`, `libc`). Talks to ubus/uci, AT ports, sysfs
  and device services. TTL caches decouple client poll rate from expensive
  subprocess reads, with bounded execution and output. Radio samples are shared
  with loggers; buffered writes and streamed CSV exports limit overhead. The
  charge-limit enforcer combines `ubus listen` events with periodic policy
  reconciliation. Auth via bearer tokens with sliding 1 h expiry,
  rate-limited login, LAN-only bind. Destructive actions require
  `X-Confirm: true`; the AT console is allowlisted to read-only commands.
- **Dashboard (`web-app/`)** — React 19 + Vite + Tailwind SPA served by the
  device itself. Lazy-loaded groups, light/dark theme, bottom tabs on phones
  / sidebar on desktop. A shared polling implementation provides in-memory
  caching, visibility-aware scheduling, non-overlapping requests and protection
  against obsolete responses after edits or navigation. Home uses a batched
  `/api/dashboard` request, with source freshness and collection errors surfaced
  in the interface.
- **Contract** — `scripts/check-api-contract.py` checks route coverage across
  the agent, dashboard client and mock agent; regression tests cover selected
  response shapes and behavior.

## Dashboard features

| Group | What you get |
|---|---|
| **Home** | live signal, modem mode, throughput, battery, connection, device info and data usage from a single batched poll |
| **Signal** | per-carrier LTE/NR detail (PCI, ARFCN, RSRP/RSRQ/SINR), matching UL and Active/Idle indicators on desktop and mobile when reported, network mode, band lock, one-tap cell lock from live cells |
| **Network** | clients by Wi-Fi/USB-C/Ethernet with link details, validated per-band Wi-Fi updates with rollback on observed failures, LAN/DHCP changes with reconnection confirmation and automatic rollback, DNS |
| **Modem** | manual APN profiles, data usage + reset day, TTL clamping, SMS (inbox/sent, compose, delete) |
| **System** | thermals, battery health, reconciled charge control (stop/resume + limit enforcer), signal/connection loggers with CSV export, read-only AT console, on-demand process list, device/SIM info, guarded USB mode + powerbank transitions, power actions |

v2.3 also refreshes the home/login and Signal icons and browser favicon. The
screenshots above are retained as an overview; minor artwork and status details
may differ from the current release.

Details: [docs/DASHBOARD.md](docs/DASHBOARD.md) (pages, source layout, local
demo without hardware) and [docs/AGENT.md](docs/AGENT.md) (endpoint
reference).

## Validation

Release builds run Rust, dashboard and native-installer checks in GitHub Actions,
including failure-injection tests for deployment and recovery. The v2.3 changes
also passed a staged physical deployment with stock service, WAN, SSH and reboot
checks. See [docs/REMEDIATION.md](docs/REMEDIATION.md) for evidence and remaining
hardware/platform limits. Firmware emulators and local test environments are not
included in this repository or release.

## Quick start

### Desktop installer (recommended)

Windows and macOS users can install, repair, or update the complete stack with
the native **Open U60 Pro Installer** from [GitHub Releases](https://github.com/dklasens/MU5250-OpenUI/releases/latest). It has a guided
interface, bundles ADB, selects only compatible ZTE MU5250 devices, and requires
no terminal or language runtimes. Packages are available for macOS 15+ Apple Silicon
and Intel (`.dmg` / `.app.zip`) and Windows x64 (MSI, setup EXE or portable ZIP).
The installer downloads checksum-verified agent/dashboard assets; verified
offline bundles can also be reused. Windows may require a compatible USB driver
and the built-in OpenSSH Client. The v2.5 Mac apps are ad-hoc signed, not
Apple-notarized; Windows packages are unsigned. See [`installer/README.md`](installer/README.md).

For a locked modem, enter its router admin password and the **backup-key suffix**.
The backup key/suffix is available in the
[upstream unlock discussion, issue #8](https://github.com/jesther-ai/open-u60-pro/issues/8).
Use the suffix appropriate to the device firmware; v2.5 requires this input and
does not bundle the key. The dashboard password is chosen separately during
installation, with an optional six-digit PIN.

Start with **Check device**, then review the model, firmware and release before
installing. Updates preserve existing dashboard passwords and PIN settings by
default; changing either is explicit. The installer provides native folder
browsing, an **Open dashboard** action and guided recovery after an interrupted
deployment. See the [v2.5 wireframes](docs/INSTALLER-WIREFRAMES.md).

Deployment checks device identity, firmware,
storage and artifacts before activation, retains recovery snapshots, and verifies
key-only SSH access. Failed deployments attempt snapshot recovery; retain the
reported recovery files until the installation has been checked.

### Terminal flow

For locked firmware, follow the compatibility guidance in
[docs/DEPLOYMENT.md](docs/DEPLOYMENT.md). The staged unlock/deployment was tested
on HK B04; this does not establish support for every later HK or CN release.

The source dashboard build requires Node.js `^20.19.0 || >=22.12.0` and npm;
`deploy-dashboard.sh` installs the locked dependencies with `npm ci`. The
native desktop installer uses prebuilt assets and does not require Node.js.

```sh
python3 scripts/zunlock.py --dry-run  # validate backup preparation before unlocking
python3 scripts/zunlock.py     # 1. unlock → adbd (config backup/restore route)
bash setup.sh                  # 2. download or build, then install the agent
bash scripts/zharden.sh        # 3. SSH, rc.local cleanup, dashboard :8080, FOTA off
bash deploy-dashboard.sh       # 4. build + push the web UI
```

Full instructions, requirements (backup-key suffix), updates and post-FOTA
recovery: **[docs/DEPLOYMENT.md](docs/DEPLOYMENT.md)**.

## Repository structure

```
agent/          Rust agent (runs on the modem, port 9090)
web-app/        React dashboard (served from the modem, port 8080)
installer/      native macOS/Windows installer (Tauri + React)
process-runner/ shared bounded subprocess runner for agent and installer
scripts/        unlock + hardening + recon tooling
  research/     quarantined exploit tools — see its README before touching
docs/           documentation (below)
setup.sh        first-time provisioning (unlock + agent install)
deploy.sh       agent updates over SSH
deploy-dashboard.sh   dashboard build + push
zte-script-ng.js      community-vetted reference of safe ubus calls
```

## Documentation

| Doc | Contents |
|---|---|
| [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) | unlock, install, harden, update, post-FOTA recovery |
| [installer/README.md](installer/README.md) | desktop installer, offline bundles, recovery and platform requirements |
| [docs/releases/v2.5.md](docs/releases/v2.5.md) | v2.5 changes: SIM Management, agent resilience, cell-lock state, validation and limitations |
| [docs/releases/v2.4.md](docs/releases/v2.4.md) | v2.4 installer changes, downloads, validation and limitations |
| [docs/INSTALLER-WIREFRAMES.md](docs/INSTALLER-WIREFRAMES.md) | publishable installer wireframes with sample data |
| [docs/AGENT.md](docs/AGENT.md) | agent architecture, endpoint reference, safety constraints |
| [docs/DASHBOARD.md](docs/DASHBOARD.md) | dashboard pages, source layout, dev + local demo |
| [docs/SAFETY.md](docs/SAFETY.md) | **read first** — brick-prevention rules, daemon sync barrier, recovery commands, safety audit |
| [docs/reference/](docs/reference/) | device reference material (rpcd ACL dump, USB mode findings) |

## Safety in one paragraph

This device was bricked once by going beyond the sanctioned path. The rules
that keep it alive: **shell/ssh/adb only** — no boot hooks outside
`/etc/rc.local`, no system-service modifications, never disable a
`zte_topsw_daemon.conf` daemon via init.d, stay out of partitions, and treat
`scripts/research/` as quarantined. Everything else — including what the
deploy path does and deliberately does not touch — is in
[docs/SAFETY.md](docs/SAFETY.md).

Dashboard/API traffic remains HTTP on the LAN; key-only SSH does not encrypt
browser traffic. Physical Wi-Fi/LAN changes, USB/charger failure paths and all
firmware variants have not been comprehensively tested. Recovery snapshots
reduce deployment risk but cannot guarantee against device failure.

## License

[MIT](LICENSE). Derived in part from
[jesther-ai/open-u60-pro](https://github.com/jesther-ai/open-u60-pro)
(MIT, Copyright (c) 2025-present Jesther Silvestre).

Exception: `zte-script-ng.js` is a community reference script licensed
separately under **AGPLv3+** — see its header.

## Source of truth

If this README and the code ever disagree:

- `agent/src/server.rs` — HTTP routing table
- `agent/src/auth.rs` — auth and token behavior
- `web-app/src/App.tsx` — navigation groups mounted in the UI
- `web-app/src/data/api.ts` — client-side API bindings and payload shapes
