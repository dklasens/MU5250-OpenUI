# AGENTS.md

Guidance for AI coding agents (Claude Code, Codex, etc.) working in this repo.
Humans should start at [README.md](README.md); the system design is in
[ARCHITECTURE.md](ARCHITECTURE.md).

## What this is

A custom control plane for the **ZTE U60 Pro (MU5250)** 5G hotspot:

- `agent/`: `zte-agent`, a Rust HTTP/JSON API that runs **on the device**
  (`192.168.0.1:9090`). It talks to ubus/uci, AT ports, sysfs and procfs.
- `web-app/`: a React 19 + Vite + Tailwind dashboard, served **from the device**
  (`:8080`) by an isolated upstream uhttpd.
- `installer/`: a Tauri desktop installer (macOS and Windows) that unlocks,
  deploys and updates both.
- `process-runner/`: a bounded subprocess runner shared by the agent and the
  installer.

## Read before you touch anything

1. **[docs/SAFETY.md](docs/SAFETY.md).** This device has been bricked once.
   The rules are not optional:
   - No boot hooks outside `/etc/rc.local`. No init.d, procd or firewall-include
     changes, and no uci changes to system services.
   - Never disable or kill a daemon listed in `/etc/config/zte_topsw_daemon.conf`.
     That is the boot sync barrier.
   - Stay out of partitions: no `dd`, `mtd`, `fw_setenv`, `abctl`, or
     QFPROM/fuse writes.
   - Never write the USB composition node (`usb_op`) live.
   - `scripts/research/` is quarantined exploit tooling. Never run it.
   - Always `sh -n /etc/rc.local` after any rc.local edit.
2. **[web-app/design.md](web-app/design.md)** before any dashboard UI change.
   It is a locked design system (Cobalt). Amend design.md instead of overriding
   it per screen.
3. **Mirror stock firmware behaviour.** When the agent diverges from stock and
   that causes a regression, prefer a narrow passive guard that defers to
   stock state (for example the `mode_main_state` charging guard in `usb.rs`)
   over adding new active intervention.

## The device

Verified on the connected unit, 2026-10-01:

| | |
|---|---|
| Model | ZTE MU5250 "U60 Pro", `MU5250_HW1.0` |
| Firmware | `XCBZ_HK_MU5250V1.0.0B04` (HK B04, built 2026-06-03) |
| SoC | Qualcomm SDX75 (`qcom,sdxpinn`), 4× aarch64 cores, ~1.6 GB RAM |
| OS | ZTE-patched OpenWrt (`r24012-d8dd03c46f`), kernel 5.15 (`-perf`), busybox ash |
| Writable | `/etc` (overlay, **wiped by FOTA**) and `/data` (survives FOTA). Rootfs is read-only. |
| LAN | `br-lan` at `192.168.0.1`. USB-C tethering is ECM (`ecm0`); Wi-Fi is `wlan0` (2.4 GHz) and `wlan2` (5 GHz). |
| WAN | `rmnet_data*` / `rmnet_ipa0` (IPA hardware offload; see ARCHITECTURE.md) |
| Firewall | wan zone `input=REJECT`, so `:8080` and `:2222` are LAN-only in practice |

Services the deploy path installs (all started from `/etc/rc.local`):

| Service | Where | Started by |
|---|---|---|
| Agent API | `192.168.0.1:9090` (LAN IP only, not loopback) | `/data/local/tmp/start_zte_agent.sh` |
| Dashboard | `0.0.0.0:8080` serving `/data/www.current` (symlink to `/data/open-u60-dashboards/<id>`) | `/data/local/tmp/start_dashboard.sh` |
| SSH | Dropbear on `:2222`, key-only, `/data/bin/dropbear` | `/data/local/tmp/start_dropbear.sh` |

Useful on-device facts:

- Agent state lives in `/data/local/tmp/`: `charge_limit.json`, `usb_config.json`,
  `start_ttl.sh`, `lan_transition.json`, `esim_state.json`, `esim_journal.json`,
  `zte-agent.log`, the CSV logs, and deploy snapshots under `open-u60-transactions/`.
- **eSIM profiles on the connected card belong to the user.** Agents never activate, deactivate,
  rename, lock, install or delete profiles, and never send receipts; card access from agents is
  read-only (inventory). The user may keep a local, untracked `esim.md` with per-card rules
  (which profiles must never be touched); read it if present. Activation codes are secrets: never
  log or echo them. A killed `zte-esim` leaks a QMI logical channel until the SIM resets; stop it
  with SIGTERM.
- Agent logs: `/data/local/tmp/zte-agent.log`, plus `.1` for the previous
  file. Each line carries device-local time and uptime; each file is capped at
  256 KiB; the log survives reboots. The agent writes this itself (`diag_log.rs`)
  because the stock busybox `syslogd -l 1` keeps only emergency messages and
  `logread` doesn't work here.
- **The system clock holds local time labelled as UTC.** The firmware sets it
  from the network (NITZ) with `TZ=UTC`, so epoch-based timestamps from the
  device (CSV logs, `sampled_at_ms`) are ahead of true UTC by the local offset.
- **busybox gotchas:** `logread -f` and `dmesg -w` are broken. Read `/dev/kmsg` or
  `dmesg`. **Never read `/proc/kmsg`:** it blocks, and it consumes messages that
  other readers would otherwise get.
- The device is **not reachable over USB while powered off or charging**. Its
  USB network gadget is not presented to the host in that state.
- Firmware version: `ubus call zwrt_zte_mdm.api get_zwrt_common_info '{}'`.
  `/proc/version` only gives the kernel version.

## Connecting

```sh
ssh -p 2222 -o UserKnownHostsFile=~/.ssh/known_hosts.d/zte root@192.168.0.1
```

The deploy tooling pins the host key in `~/.ssh/known_hosts.d/zte`, not in
`~/.ssh/known_hosts` (the default file has a stale entry for `[192.168.0.1]:2222`).
If the pinned key stops matching, Dropbear was probably reinstalled. **Ask the
user before re-pinning.** Never pass `StrictHostKeyChecking=no`.

Default to **read-only** inspection on the device: `cat`, `ubus call … get*/list`,
`uci get/show`, `ls`, `ps`, `dmesg`. Anything that mutates device state
(uci set/commit, ubus set*, iptables, killing processes, reboots, writing files)
needs the user's go-ahead first unless they have already asked for it.
Redact IMEI, IMSI, ICCID, MSISDN and serials from anything you print or commit.

## Build, test, deploy

Prerequisites: Rust with the `aarch64-unknown-linux-musl` target and
`aarch64-linux-musl-gcc` on PATH (Homebrew), Node `^20.19 || >=22.12`, Python 3.

```sh
# Checks. These mirror CI (.github/workflows/checks.yml); run them before calling work done.
cargo fmt --all --check
cargo test -p zte-agent -p process-runner -p zte-esim -p euicc --locked
cargo clippy -p zte-agent -p process-runner -p zte-esim -p euicc --all-targets --locked -- -D warnings
python3 scripts/check-api-contract.py        # agent routes <-> dashboard calls <-> mock agent
python3 -m unittest discover -s tests
(cd web-app && npm ci && npm run build && npm run lint && npm test)

# Local dashboard demo, no hardware needed (mock agent on :9090)
(cd web-app && bash tools/demo.sh)           # stop with: bash tools/demo.sh stop

# Deploy to the connected device over SSH (transactional, with snapshot + rollback)
./deploy.sh              # builds the agent + zte-esim helper, pushes both, rewrites the startup script, restarts, verifies
./deploy-dashboard.sh    # npm ci + build, stages the release, switches /data/www.current, verifies
# Both accept --dry-run, --gateway ADDR and --adb-serial SERIAL.
```

- `deploy.sh` re-renders the startup script, so it **needs the agent password**
  (`ZTE_AGENT_PASSWORD`, or an interactive prompt) and optional `ZTE_AGENT_PIN`.
  Ask the user for these. Do not scrape them from the device's startup script.
- Always run `--dry-run` first when changing deploy tooling.
- After deploying, verify on-device: `md5sum /data/zte-agent` against the local
  build, `pidof zte-agent`, `tail /data/local/tmp/zte-agent.log`, and a live API call. The user
  expects a live-device check before a fix is called done.
- First-time install, unlock and post-FOTA recovery: [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md).

## Repo map

```
agent/src/server.rs      routing table (source of truth for the API), AT allowlist, TTL
agent/src/handlers.rs    AppState, /api/dashboard batch, per-source TTL caches
agent/src/cache.rs       Cached<T> / Observed<T> (TTL + last-good + freshness metadata)
agent/src/esim.rs        eSIM manager: detection, profile cache, one change at a time, journal, receipts
esim-helper/             zte-esim: on-demand card helper (QMI UIM over QRTR, ES10 allowlist, HTTPS receipts)
euicc/                   LGPL-2.1 SGP.22 card library used only by zte-esim (see THIRD_PARTY_NOTICES.md)
agent/src/*.rs           one module per domain: cell, wifi, lan, router, sms, usb, charge_policy, …
process-runner/          bounded subprocess I/O (deadline + output cap)
web-app/src/data/api.ts  endpoint bindings + firmware response mappers (source of truth, client side)
web-app/src/data/poll.ts shared poller (visibility-aware, non-overlapping, SWR cache)
web-app/src/features/    home, signal, network, modem, system groups
web-app/tools/mock_agent.py  stdlib mock agent used by the demo and the contract check
installer/               Tauri app (src-tauri/ is Rust; src/ is React)
scripts/                 deploy-components.py, zharden.sh, zunlock.py, contract/secret/release checks
scripts/research/        QUARANTINED. Do not run.
docs/                    SAFETY, DEPLOYMENT, AGENT (endpoints), DASHBOARD, REMEDIATION, reference/
zte-script-ng.js         community-vetted reference of safe ubus calls (AGPL, reference only)
```

Gitignored local material you may find in the working tree: `firmware/`
(an extracted rootfs, handy for reading stock scripts under
`firmware/extracted/system_a_root/`), `logs/`, `loopdebug-capture/`,
`device_report.json`, `ubus_probe_report.json`, `back_parameter` and
`adb-lock-investigation.md`. The last two hold secrets. **Never commit them,
and never quote them.**

## Conventions

- **Three-way API contract.** A route change touches `agent/src/server.rs`,
  `web-app/src/data/api.ts` and `web-app/tools/mock_agent.py` together, plus
  `docs/AGENT.md`. `check-api-contract.py` enforces it.
- **JSON envelope:** `{"ok": true, "data": …}` / `{"ok": false, "error": "…"}`.
  Handlers return `(u16, serde_json::Value)`.
- **Every subprocess goes through `BoundedCommand::bounded_output()`**
  (10 s deadline, 2 MiB output cap). Never call a bare `Command::output()`.
- **Poll-driven reads go behind `Observed<T>`/`Cached<T>`** with a TTL.
  `/api/dashboard` is the dashboard's heartbeat, so don't add per-page pollers
  for data it already carries.
- **Destructive endpoints** go in `DESTRUCTIVE_PATHS` (needs `X-Confirm: true`).
  AT commands must be added to the exact-match allowlist; the blocklist is
  defence in depth.
- Firmware quirk: `zwrt_bsp.charger` `direct_power_supply_mode: "enable"`
  **stops** charging. The inversion is deliberate. Don't "fix" it.
- State files are written with `storage::atomic_write`. They stay under `/data/local/tmp/`.
- Keep the agent dependency-light (serde, serde_json, tiny_http, sha2, libc).
  No async runtime, no TLS stack.
- Commit messages: imperative, sentence case, describe the behaviour change
  (see `git log`).

## Using subagents

Lower-cost subagents (for example `model: sonnet` for moderate
reasoning) are encouraged where they are effective. Pass them precise file
paths and a narrow question. Good fits:

- Read-only searches through the extracted firmware (`firmware/extracted/`):
  finding which stock script sets a uci key, or which binary registers a
  ubus method.
- Sweeping for doc drift, such as endpoint counts or TTL values in
  `docs/*.md` versus the code.
- Mechanical, well-specified edits: updating `mock_agent.py` fixtures to a
  new response shape, or adding a band to a lookup table.
- Summarising large captured JSON or logs (`device_report.json`, `logs/`).

Keep these on the main model and don't delegate them:

- Anything that runs commands on the device, deploys, or touches
  `scripts/zunlock.py`, `zharden.sh`, rc.local handling, `usb.rs`,
  `charge_policy.rs`, `esim.rs` or `esim-helper/`.
- Changes to the auth, AT allowlist, kill-bloat or safety logic.
- Final review of a change before telling the user it is done.
