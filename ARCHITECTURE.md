# Architecture

How the pieces fit together. For endpoint-level detail see
[docs/AGENT.md](docs/AGENT.md). For dashboard pages see
[docs/DASHBOARD.md](docs/DASHBOARD.md). For device rules see
[docs/SAFETY.md](docs/SAFETY.md).

## System overview

```
 Phone / laptop (LAN: Wi-Fi wlan0/wlan2, USB-C ecm0)
   │
   │  GET http://192.168.0.1:8080/        static SPA (index.html, JS, fonts)
   ├────────────────────────────────►  dashboard-uhttpd  (/data/bin, upstream OpenWrt build)
   │                                     serves /data/www.current → /data/open-u60-dashboards/<id>
   │
   │  fetch http://192.168.0.1:9090/api/*   Bearer token, JSON envelope
   └────────────────────────────────►  zte-agent  (/data/zte-agent, Rust, tiny_http)
                                          │
        ┌───────────────┬─────────────────┼──────────────────┬──────────────────┐
        ▼               ▼                 ▼                  ▼                  ▼
   ubus (fork+exec) uci (fork+exec)  AT tty            sysfs / procfs     iptables, iw, bridge
   zte_nwinfo_api   wireless,        /dev/at_mdm0…     battery, thermal,  (TTL clamp, Wi-Fi
   zwrt_data/_wlan/ zwrt_router,     (read-only        usb gadget,        stations, bridge FDB)
   _bsp/_router/…   dhcp             allowlist)        /proc/stat, usb gadget configfs
        │
        ▼
   ZTE stock daemons (zte_topsw_*, zwrt_*)  →  SDX75 modem baseband
```

The stock ZTE web UI (`:80`/`:443`) and its patched uhttpd are left alone.
The dashboard gets its own upstream uhttpd, because the ZTE build registers
a singleton ubus object that makes a second instance unreliable.

## Components

| Component | Language | Runs on | Role |
|---|---|---|---|
| `agent/` (`zte-agent`) | Rust 2021, static musl aarch64 | device | JSON API over device services. With its eSIM helper, it is the only on-device code the project adds. |
| `esim-helper/` (`zte-esim`) | Rust 2021, static musl aarch64 | device | On-demand eSIM helper the agent runs per card operation: SGP.22 card commands over QMI UIM (QRTR) and operator receipts over HTTPS. Carries the LGPL `euicc/` library and the TLS stack, so the agent carries neither. No daemon, no boot hook. |
| `euicc/` | Rust (LGPL-2.1-only) | device (in `zte-esim`) | Rust translation of lpac's libeuicc. See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). |
| `process-runner/` | Rust | device + desktop | Bounded subprocess execution: one deadline covers stdin, run and pipe drain, and output is capped. The process group is killed on timeout, or (for `zte-esim`) sent SIGTERM with a grace period first. |
| `web-app/` | React 19, Vite, Tailwind 3 | browser, served from device | Dashboard SPA. It holds no server-side state. |
| `installer/` | Tauri 2 (Rust + React) | macOS / Windows | Guided detect, unlock, deploy, repair and update. Bundles ADB. |
| `scripts/` | Python 3, POSIX sh | developer machine / device | Terminal deploy (`deploy-components.py`), hardening (`zharden.sh`), unlock (`zunlock.py`), CI checks. |

## The agent

### Process model

- `main()` does one-shot migrations and loads auth from the environment
  (`ZTE_AGENT_PASSWORD`, `ZTE_AGENT_PIN`). It starts the background threads,
  runs boot-time enforcement, then enters `server::start`.
- **HTTP:** `tiny_http` with `ZTE_AGENT_THREADS` workers (default 4, clamped
  to 1–16). There is no async runtime. Each worker blocks in `recv_timeout(500ms)`.
  If the accept thread dies (EMFILE, interface churn), a supervisor drains the
  workers and rebuilds the listener with backoff (1 s up to 30 s). The listener
  also rebuilds when a confirmed LAN change bumps `Binding::generation`.
- **Bind:** the configured LAN IPv4 (`zwrt_router.network.lan_ipaddr`) on
  `:9090`, not loopback, not `0.0.0.0`. `ZTE_AGENT_BIND` pins it, which
  disables LAN IP changes.

Long-lived background threads:

| Thread | Cadence | Purpose |
|---|---|---|
| Event bus (`event_bus.rs`) | blocking | One `ubus listen` child. Dispatches events to subscribers over bounded channels (capacity 64, drop-on-full) and restarts after 5 s if the child exits |
| Charge enforcer (`charge_policy.rs`) | 60 s while a limit is active, else 300 s, plus `BSP_CHARGER_EVENT` | Reconciles charger state against the persisted policy |
| Logger sessions (`logging.rs`) | 1–60 s, only while running | Signal and connection CSV logging from the shared radio sample |
| USB boot enforcement (`usb.rs`) | once at boot, up to 75 s | Re-applies NCM only if it was explicitly persisted, and skips power-off-charging states |
| LAN transition (`lan.rs`) | only during a LAN change | Applies the change, then waits up to 120 s for confirmation before rolling back |

### Request lifecycle (`server.rs`)

1. `OPTIONS` → CORS preflight. `Access-Control-Allow-Origin` echoes only
   `http://` origins on RFC 1918 addresses or localhost.
2. Auth: everything except `POST /api/auth/login` and
   `POST /api/router/lan/confirm` needs `Authorization: Bearer <token>`.
   Tokens are random, kept in memory (at most 10), and slide to 1 h on each use.
   Login is rate-limited per IP: 5 failures arms a 30 s lockout. The password
   hash is 10 000 × salted SHA-256, salted by `/data/.zte-agent-salt`.
   PIN login is accepted only from mobile user agents.
3. Destructive paths (`reboot`, `shutdown`, `kill-bloat`) need `X-Confirm: true`.
4. CSV downloads stream straight from the file, with length snapshotted at open.
5. Everything else: the body is read with a 1 MiB cap, then
   `route(method, path)` dispatches to a handler that returns
   `(status, serde_json::Value)`, which goes out as `{"ok", "data"|"error"}`.

### Data sources and caching (`cache.rs`, `handlers.rs`)

Every ubus/uci read is a fork+exec. Measured on the device, a `ubus call`
costs about 4–5 ms wall-clock, so the cost is real but modest. Poll-driven
reads sit behind one of two wrappers:

- `Cached<T>`: value plus timestamp. The lock is held during refresh, so
  concurrent callers share one refresh.
- `Observed<T>`: the same, plus it **keeps the last good value** when a
  refresh fails, throttles retries to the TTL, and reports `Freshness`
  (`sampled_at_ms`, `age_ms`, `ttl_ms`, `stale`, `error`). The dashboard
  surfaces this as source warnings.

`GET /api/dashboard` is the heartbeat batch:

| Field | Source | TTL |
|---|---|---|
| `device`, `memory` | procfs, plus firmware identity (`zwrt_zte_mdm.api get_zwrt_common_info`, read once) | none (free) |
| `cpu` | `/proc/stat` delta | 2 s minimum window |
| `battery` | `/sys/class/power_supply/battery/*` + `usb/online` (external power) | none |
| `speed` | `ubus zwrt_data get_wwandst` `real_*` counters, averaged between samples (see below) | 1 s |
| `signal` | `ubus zte_nwinfo_api nwinfo_get_netinfo` (shared `Arc<Observed>` with the loggers) | 1 s |
| `thermal` | `ubus zwrt_bsp.thermal get_cpu_temp` | 10 s |
| `wan`, `wan6` | `ubus network.interface.zte_wan[6] status` | 30 s |
| `data_usage` | `ubus zwrt_data get_wwandst` + `get_wwandst_clearday` (+ uci cycle dates, 300 s) | 30 s |
| `sources` | freshness metadata for the six `Observed` sources above | — |

**Throughput** comes from the modem's own WAN counters, not the `rmnet_*`
netdevs. With IPA hardware offload, forwarded client traffic largely bypasses
the netdev counters; on 2026-10-01 they read about 17× less than the modem.
`real_rx_speed`/`real_tx_speed` are the byte delta over the modem's last
one-second window (bytes/s). The agent reports the average rate since its
previous sample instead, which is smoother at the dashboard's 3 s poll. It
uses the firmware's one-second figure when there is no sample from the last
10 s or the counters reset on reconnect.

Other endpoints read on demand: Wi-Fi (`zwrt_wlan report` + `uci show wireless`
+ `iw`), clients (`luci-rpc getDHCPLeases` + `/proc/net/arp` + `iw station dump`
+ `bridge fdb`), full thermal zones (sysfs), process list (`/proc`, 3 s cache),
and USB status (`zwrt_bsp.usb list` + configfs + UDC sysfs).

### Mutations

| Area | Mechanism | Safety net |
|---|---|---|
| Wi-Fi | `uci_transaction`: a private `uci -t` staging dir, compare-and-set against a snapshot, commit, `zwrt_wlan reload` | Restores the previous values on any failure. A global `WIFI_CHANGE` mutex serialises changes. |
| LAN / DHCP | `lan.rs`: journal in `/data/local/tmp/lan_transition.json`, apply after responding, rebind the listener | The client must `POST /api/router/lan/confirm` with a per-change token within 120 s, otherwise it rolls back. The journal is replayed at boot (`state.lan.recover()`). |
| Band / cell lock, network mode | `zte_nwinfo_api` (`nwinfo_set_*bandlock`, `nwinfo_lock_*_cell`, `nwinfo_set_netselect`) | Validated against firmware band lists; NR band lock is SA-only |
| APN | `zwrt_apn_object` | Switches to manual mode, and restores the previous mode on failure |
| Charge control | `zwrt_bsp.charger set direct_power_supply_mode` (inverted: `enable` = stop) | Read-back verification and rollback on failure. Policy is persisted atomically. |
| USB mode | configfs gadget rebuild (NCM) or stock switch (ECM/RNDIS) | Global switch guard, preflight checks, restore previous composition and bridge on failure |
| TTL / HL clamp | `iptables`/`ip6tables -t mangle PREROUTING -i br-lan`, persisted to `/data/local/tmp/start_ttl.sh` | Reapplied at agent start with `-C` idempotence checks |
| SMS | `zwrt_wms` via ZTE's legacy WMS payloads | Command-status polling |
| kill-bloat | `SIGTERM` to allowlisted optional daemons | Subtracts the live `zte_topsw_daemon.conf` sync barrier; fails closed if it cannot be read |
| eSIM profiles | `esim.rs` runs `zte-esim` per operation (stdin JSON). Enable/disable with SIM refresh, nickname, operator receipts | One card session at a time (agent mutex + the helper's `flock`). Identity and state are re-checked in the same card session as the write. Writes are never retried; a lost answer is settled by read-back, and a restart reads back from `/data/local/tmp/esim_journal.json`, never replays. Locked profiles refuse activation. The helper gets SIGTERM and 5 s to close its logical channel: a killed session leaks the channel until the SIM resets. |

### Persistent state (device)

```
/data/zte-agent                          binary
/data/zte-esim                           eSIM helper binary (run on demand by the agent)
/data/local/tmp/esim_state.json          eSIM: salt, hashed card id, locked profiles, receipts in delivery
/data/local/tmp/esim_journal.json        eSIM change in flight (transient; read back after a restart)
/tmp/zte-esim.lock                       eSIM card lock (flock, released by the kernel)
/data/.zte-agent-salt                    password-hash salt
/data/local/tmp/start_zte_agent.sh       startup script (exports ZTE_AGENT_PASSWORD / PIN), run from rc.local
/data/local/tmp/charge_limit.json        charge policy
/data/local/tmp/usb_config.json          persisted USB default mode
/data/local/tmp/start_ttl.sh             TTL clamp rules
/data/local/tmp/lan_transition.json      pending LAN change journal (transient)
/data/local/tmp/{signal,connection}_log.csv   logger output (8 MiB cap each)
/data/local/tmp/zte-agent.log[.1]        agent diagnostics (256 KiB each, rotated; see diag_log.rs)
/data/local/tmp/open-u60-transactions/   deploy snapshots for rollback
```

## The dashboard

- **Shell:** `App.tsx` gates on a token in storage, then mounts five lazily
  loaded groups (Home, Signal, Network, Modem, System). Phones get a bottom
  tab bar and desktops get a sidebar.
- **Data layer** (`src/data/`):
  - `client.ts`: token handling, envelope unwrapping, timeouts, and an
    auth-expired event that returns the user to Login.
  - `api.ts`: one function per endpoint, plus **mappers** that turn raw
    firmware shapes into UI types. Examples: `lteca`/`ltecasig`/`nrca`
    carrier strings into per-carrier rows, hex band-lock masks into band
    lists, EARFCN/NR-ARFCN into MHz, UCS-2 SMS hex into text. The agent
    passes most ubus payloads through raw, so this is where firmware
    knowledge lives on the client side.
  - `poll.ts` / `pollScheduler.ts`: `usePoll(key, fn, interval)` and the
    one-shot `useResource(key, fn)` each own one resource and report
    `status` (`loading`, `ready`, `error`, `stale`). Polls never overlap,
    pause while the tab is hidden, and use a revision counter so a mutation
    discards in-flight stale reads. A key change never shows the previous
    key's data. The module-level last-good cache lets group switches render
    instantly and is cleared on logout and auth expiry.
  - `signalQuality.ts`: the single RSRP/RSRQ/SINR rating policy; classifiers
    and legends derive from it. `bands.ts`, `usage.ts`, `dates.ts`,
    `wifiWidth.ts` and `validate.ts` are pure mappers and validators.
    Unknown firmware values map to `null`/unknown, not to a default.
- **Heartbeat:** `HomeProvider` polls `/api/dashboard` every 3 s on Home
  and Signal, and every 15 s elsewhere, where it only feeds the alert banner.
  It stays the one heartbeat: Home, Signal and Modem → Data all read it;
  nothing re-polls the same data. Other resources are owned by the screen
  that shows them (`usePoll`/`useResource`); expensive views (clients,
  process list) poll slowly or load on demand.
- **Mutations:** connection-dropping actions are confirmed through one native
  `<dialog>` host (`ui/feedback.tsx`) before anything is sent. The reviewed
  payload is frozen first, and the dashboard reports "accepted" separately
  from "verified by read-back". It never retries a mutation automatically.
- **Design system:** [web-app/design.md](web-app/design.md) (tokens in
  `src/index.css`, exposed to Tailwind). Fonts are self-hosted. There are
  no runtime dependencies beyond React.

## Contract between agent and dashboard

Three copies of the API must agree: the agent route table (`server.rs`), the
client bindings (`api.ts`) and the mock agent (`web-app/tools/mock_agent.py`).
`scripts/check-api-contract.py` fails CI if they drift. It compares **method
and path**, so a GET binding for a PUT-only route is a failure
(`tests/test_api_contract.py` tests the checker). Rust unit tests pin the key
sets of the payloads the dashboard reads verbatim (speed snapshot, process
list).

The mock agent is stateful: mutations validate like the agent and change what
the next read reports. It exposes `/__mock/requests` and `/__mock/reset`, and
`MOCK_SCENARIO` (`SA`, `NSA`, `LTE`, `disconnected`) selects the radio. The
Playwright browser harness (`npm run test:browser`, in CI) answers `/api`
from an in-process synthetic agent and aborts every other host, so it never
reaches a real agent.
## Deployment model

```
developer machine                                         device
─────────────────                                         ──────
deploy.sh            cargo build (musl aarch64) ─┐
deploy-dashboard.sh  npm ci && vite build → tgz ─┤
                                                 ▼
                         scripts/deploy-components.py
                           1. validate artifacts locally (ELF arch, archive contents, sizes)
                           2. identify device (model/firmware) over SSH :2222 or ADB
                           3. preflight: tools present, rc.local parses, space for payload + rollback
                           4. snapshot → /data/local/tmp/open-u60-transactions/<id>
                           5. stage, hash-verify, activate (atomic rename / symlink switch)
                           6. restart the affected service, verify it from the device (on-device curl)
                           7. on any failure: restore the snapshot and print a recovery command
```

Everything starts from `/etc/rc.local` lines added idempotently and checked
with `sh -n`. There are no init.d or procd services. FOTA wipes `/etc`, and
so the rc.local lines; `/data` survives. Recovery after a firmware update is
to re-run the deploy sequence (see [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md)).
Locked firmware is unlocked once through the config backup/restore path
(`scripts/zunlock.py`, or the installer).

Boot order on the device: stock boot → `rc.local` → Dropbear, then the agent
(migrations, TTL rules, NCM enforcement if persisted, LAN journal recovery,
listener), then the dashboard uhttpd.

## Security model

- The LAN is trusted, and so is the firewall: the wan zone rejects input.
  The agent binds the LAN IP only; the dashboard server binds all interfaces
  but is not reachable from WAN.
- Traffic is plain HTTP. Bearer tokens and the password cross the LAN
  unencrypted. SSH is key-only.
- The AT console accepts only an exact-match list of read-only commands.
- The agent password sits in cleartext in `start_zte_agent.sh`
  (root-only, `/data`).
- Sessions record whether the password or the PIN opened them. eSIM
  activate/deactivate and profile locks need a password session. Session
  expiry uses the monotonic clock, because the wall clock jumps when the
  network sets the time after a SIM change.
- eSIM identifiers stay on the device: clients get opaque handles and the
  last four digits. The EID is persisted only as a salted hash. Receipts go
  only to DNS-named public addresses over TLS that trusts the GSMA eSIM root
  alone.

## Known limitations

- Thermal zones in `/api/device/thermal/all` are hardcoded by zone index.
- NR band locking works in SA mode only. That is a firmware limitation.
- The agent has no supervisor. If it exits, it stays down until the next
  reboot or `start_zte_agent.sh` is re-run.
