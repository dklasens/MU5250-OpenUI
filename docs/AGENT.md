# Agent — `zte-agent`

Rust HTTP backend that runs on the modem (`agent/`), talking to ubus, AT
ports, sysfs/procfs and device services. `agent/src/server.rs` is the
canonical routing table; this document summarizes it.

- Binds `192.168.0.1:9090` (override `ZTE_AGENT_BIND`, `ZTE_AGENT_THREADS`).
  Because it does not listen on device loopback, ADB setup verification runs
  an on-device curl against this LAN address; forwarding to `tcp:9090` and
  changing the HTTP Host header would not reach the listener.
- Auth: `POST /api/auth/login` (password from `ZTE_AGENT_PASSWORD`, or an
  optional 6-digit mobile PIN); bearer tokens with a sliding 1 h expiry so a
  dashboard left open stays logged in, rate-limited login, LAN-only CORS
- JSON envelope: `{ "ok": true, "data": … }` / `{ "ok": false, "error": … }`
- Destructive actions (`/api/device/reboot|shutdown`) require the
  `X-Confirm: true` header

## Endpoint reference

Every route below has a dashboard consumer, and every call the dashboard makes
is a route below — `scripts/check-api-contract.py` enforces both directions
(plus that the mock agent stays in step). 58 paths / 63 method+path pairs.

| Family | Endpoints |
|---|---|
| Auth | `POST /api/auth/login` — bearer token, sliding 1 h expiry |
| Batch | `GET /api/dashboard` — device, battery, cpu, memory, speed, data usage, signal, wan, wan6, thermal in one request. The app's heartbeat: Home, Signal and Modem/Data all read it instead of polling their own endpoints |
| Status | `GET /api/device`, `/api/cpu`, `/api/memory`, `/api/system/top` |
| Network | `GET /api/network/clients` |
| Device | `GET /api/device/battery-info`, `/api/device/thermal/all`, `/api/device/battery/detail`, `/api/device/charger`; `POST /api/device/reboot`, `/api/device/shutdown` |
| System | `POST /api/system/restart-agent`, `/api/system/kill-bloat` |
| Wi-Fi | `GET /api/wifi/status`, `PUT /api/wifi/settings` |
| Modem | `PUT /api/data-usage/reset-day`, `PUT /api/modem/network-mode` |
| Cell/band lock | `POST /api/cell/lock/nr`, `/api/cell/lock/lte`, `/api/cell/lock/reset`, `/api/cell/band/nr`, `/api/cell/band/lte`, `/api/cell/band/reset` |
| Router | `GET+PUT /api/router/dns`, `/api/router/lan`, `/api/router/apn/mode`; `GET+POST /api/router/apn/profiles`; `POST /api/router/apn/profiles/delete`, `/api/router/apn/profiles/activate` |
| SMS | `POST /api/sms/list`, `/api/sms/send`, `/api/sms/delete`, `/api/sms/read` (delete falls back to direct SQLite for SIM-stored rows the firmware refuses) |
| SIM | `GET /api/sim/info`, `/api/sim/imei` |
| USB | `GET /api/usb/status`, `PUT /api/usb/mode`, `/api/usb/default`, `/api/usb/powerbank` |
| Power | `GET+PUT /api/device/charge-control` — manual stop/resume + limit enforcer with hysteresis, event-driven off `BSP_CHARGER_EVENT` |
| Extras | TTL clamping (`GET /api/ttl/status`, `PUT /api/ttl/set`, `DELETE /api/ttl/clear`), AT console (`POST /api/at/send`, `GET /api/at/port`), signal/connection CSV loggers (`/api/logger/*`) |

## Architecture notes

- **Transport**: `tiny_http` thread pool; no async runtime (small binary,
  small footprint). The listener is supervised — tiny_http's accept thread
  exits permanently on its first `accept()` error, so `server::start` watches
  for that, drains the workers and rebuilds rather than sitting alive serving
  nothing.
- **Dependencies**: `serde`, `serde_json`, `tiny_http`, `sha2`, `libc`. No TLS
  stack and no HTTP client — removing the DoH proxy, SMS forwarder and speed
  test dropped `ureq`, and with it rustls/ring/ICU.
- **Subprocess cost**: every `ubus`/`uci` read is a fork+exec, which dominates
  the agent's CPU (about 4–5 ms per `ubus call` on-device). `cache.rs` gives
  each dashboard source its own TTL (signal 1 s, WAN throughput 1 s, thermal 10 s, wan/wan6 30 s, data usage 30 s, cycle dates 300 s), so
  the client's poll rate is decoupled from the refresh rate and concurrent
  clients collapse onto one refresh. `wifi_status` dumps whole configs with
  `ubus::uci_show` instead of issuing one `uci get` per key.
- **Event bus**: one `ubus listen` process dispatches to subscribers over
  bounded channels (`BSP_CHARGER_EVENT` → charge enforcer).
- **State files** (all under `/data/local/tmp/`): `charge_limit.json`,
  `usb_config.json`, signal/connection CSV logs.
- **Boot behavior**: `main.rs` runs a one-shot migration that undoes the
  removed DoH proxy's dnsmasq rewiring (otherwise a device that had DoH
  enabled would come back up forwarding DNS to a dead port), applies
  `start_ttl.sh` if present, and re-applies persisted NCM only if explicitly
  enabled — see [SAFETY.md](SAFETY.md) §2 for why the latter two are acceptable.
- **Logging**: the agent redirects its own stdout/stderr (`diag_log.rs`) to
  `/data/local/tmp/zte-agent.log`. Lines carry the device's local time and uptime.
  Each file is capped at 256 KiB, one rotated `.1` copy is kept, and identical
  consecutive lines are collapsed into a count. Syslog isn't usable here: the
  stock busybox `syslogd` runs with `-l 1` (emergency only) and has no `logread`
  backend. The startup script sends the agent's output to `/dev/null`, and the
  agent redirects its own stdout/stderr at startup. Timestamps in this log and
  in the CSV logs are device-local time without a `Z`. The firmware keeps local
  time in the system clock with `TZ=UTC`, so epoch values from the device
  differ from true UTC by the local offset.

## Safety constraints built into the agent

- **AT console is allowlisted** (`server.rs`): read-only commands only;
  `AT+CFUN`, `AT^…`, `AT+CMGD`, `AT$QCRMCALL`, `AT+CLCK`, `AT+CGDCONT=`,
  `AT+CGACT=` are blocked.
- **kill-bloat only kills daemons that are safe to kill** — never the
  `zte_topsw_daemon.conf` sync-barrier set (see SAFETY.md).
- **Destructive endpoints require `X-Confirm: true`** (`/api/device/reboot`,
  `/api/device/shutdown`).
- **Login is rate-limited**: 5 failures per client IP arms a 30 s lockout.
- **LAN-only bind + LAN-origin CORS** by default.
- ubus inputs passed through from HTTP are size/depth-validated
  (`validate.rs`) before forwarding.

## USB modes

See [reference/usb-modes.md](reference/usb-modes.md) for the live-device
findings: only ECM/RNDIS are exposed by the stock switch; NCM exists in
configfs and is agent-managed (experimental, gated behind
`confirm_experimental`), and the ubus `mode` field is not a reliable
detector of the active composition.

CN firmware B31 removed the stock switch itself: `zwrt_bsp.usb` exposes only
`list`. The agent checks for `set` once (`ubus -v list`), reports
`mode_switch: false`, marks RNDIS unsupported and refuses ECM/RNDIS requests
with 409 instead of failing with "Method not found". The configfs NCM path
does not depend on it.

## Building

```sh
cargo build --release --target aarch64-unknown-linux-musl -p zte-agent
```

Cross-linker config lives in `.cargo/config.toml`
(`aarch64-linux-musl-gcc`). `cargo test` runs the unit tests (auth lockout
and token expiry, dashboard payload shapes, TTL cache, UCI value unquoting,
USB boot guards, WiFi sanitizers).

`python3 scripts/check-api-contract.py` asserts the agent route table, the
dashboard's calls and the mock agent's fixtures all agree — run it after
touching any of the three.

## Recovery, freshness and bounded logging

The default listener reads the configured LAN IPv4 address at startup. A LAN
change returns HTTP 202 with a reconnect address and a confirmation token. The
dashboard confirms connectivity within 120 seconds. Until confirmation, a
private recovery record lets the agent restore the previous settings after a
timeout or restart. A fixed `ZTE_AGENT_BIND` override blocks IP changes. The
confirmation endpoint accepts only the token scoped to that pending change;
it does not require forwarding the general session token to the new address.

Dashboard `sources` metadata reports the last successful sample time, age,
refresh interval, staleness and collection error for signal, WAN, IPv6 WAN,
thermal and data-usage sources. Failed refreshes retain the last successful
reading and mark it stale. The UI shows source failures and charge-policy
errors, and warns when its entire dashboard refresh fails.

`data_usage` (in `/api/dashboard` and the `PUT /api/data-usage/reset-day`
reply) reports counters as numbers, or `null` when the firmware omits or
garbles one. `reset_day` (1–31) and `reset_enabled` (0/1) come from
`zwrt_data get_wwandst_clearday`, falling back to the persisted
`zwrt_data_commit.wwancid1dst` UCI values; each is `null` when neither source
supplies a valid value. `clear_date_record` (`YYYY/MM/DD`) and
`next_clear_date` (`YYYYMMDD`) are passed through as the firmware stores them.
`since_power_on` holds the firmware's `real_*` counters; on HK B04 these reset
with the data connection, not only at power-on, so the dashboard labels them
connection counters. Saving a reset day also enables the automatic reset.

Signal and connection loggers share the dashboard's radio source (one-second
minimum refresh interval). CSV files have an 8 MiB cap per logger, buffered
writes, a 30-second maximum flush interval and an error field in logger status.
Downloads stream a fixed-length snapshot as `text/csv`, retaining the same
endpoint paths. The updated dashboard also understands older JSON-wrapped CSV
responses. A logger stops on write/flush failure rather than counting failed
writes as successful samples.

Charge policy reconciles periodically even when charger events are lost. USB
switches are serialised, abort when boot readiness cannot be established, and
attempt to restore the previous composition and bridge after a failed switch.
Physical USB and charging behaviour must still be checked on each supported
firmware; local failure tests do not establish hardware compatibility.
