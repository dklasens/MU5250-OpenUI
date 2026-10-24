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
  dashboard left open stays logged in, rate-limited login, LAN-only CORS.
  Token expiry runs on the monotonic clock, so a network time change after a
  SIM switch neither ends nor extends a session. A session remembers whether
  the password or the PIN opened it: eSIM activate, deactivate, lock,
  download (and its confirm/cancel), delete and receipts/send need a password
  session (403 for a PIN session)
- JSON envelope: `{ "ok": true, "data": … }` / `{ "ok": false, "error": … }`
- Destructive actions (`/api/device/reboot|shutdown`, `/api/system/kill-bloat`,
  `/api/esim/enable|disable|delete`) require the `X-Confirm: true` header

## Endpoint reference

Every route below has a dashboard consumer, and every call the dashboard makes
is a route below — `scripts/check-api-contract.py` enforces both directions
(plus that the mock agent stays in step). 69 paths / 74 method+path pairs.

| Family | Endpoints |
|---|---|
| Auth | `POST /api/auth/login` — bearer token, sliding 1 h expiry |
| Batch | `GET /api/dashboard` — device, battery, cpu, memory, speed, data usage, signal, wan, wan6, thermal in one request, plus `esim: {status, available}` (capability only; see eSIM). The app's heartbeat: Home, Signal and Modem/Data all read it instead of polling their own endpoints |
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
| eSIM | `GET /api/esim/profiles`; `POST /api/esim/refresh`, `/api/esim/enable`, `/api/esim/disable`, `/api/esim/nickname`, `/api/esim/lock`, `/api/esim/download`, `/api/esim/download/confirm`, `/api/esim/download/cancel`, `/api/esim/delete`, `/api/esim/receipts/send` — see [eSIM profiles](#esim-profiles) |
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

## eSIM profiles

Profiles on a removable eUICC (for example a 9eSIM card) in the SIM slot. The
card work runs in `zte-esim`, an on-demand helper (`/data/zte-esim`) that
carries the SGP.22 card code and its TLS stack; the agent carries neither
(`agent/src/esim.rs`). Activation and deactivation were validated on a 9eSIM
card (U60 Pro, HK B04) on 2026-10-24, and so was a profile download; the view
reports `hardware_validated.switching: true`.

**Heartbeat.** `/api/dashboard` carries `esim: {status, available}`. `status` is
`detecting`, `ready`, `unsupported` (an ordinary SIM), `unavailable` (the
modem's SIM service did not answer) or `helper_missing`. Detection runs in the
background at most once a minute and is not repeated for an ordinary SIM until
the SIM changes. A card's management application is unreachable both for an
ordinary SIM and for a few seconds while an eSIM card reloads after a switch
(the same QMI error 80). So the status stays `ready` with a stale list only
during an eSIM change, for 2 minutes after one, or while the SIM is not yet
ready; otherwise a ready SIM without it is `unsupported`. The tab then hides,
and the remembered card and its locks return when the card is reinserted. After
a restart the tab appears only once the card has been read.

**The view.** Every eSIM route answers with the same object:

| Field | Meaning |
|---|---|
| `status`, `available`, `diagnostic` | as in the heartbeat, plus a reason when not ready |
| `card` | `id` (masked, `…` + last 4), `firmware`, `sgp22_version`, `profile_version`, `free_nvm_bytes`, `free_ram_bytes`, `pending_receipts` (receipts on the card) — or `null` |
| `profiles[]` | `handle` (opaque, stable), `display_name` (nickname, else name, else provider; never an identifier-like name), `nickname`, `name`, `provider`, `iccid_masked`, `state` (`enabled`/`disabled`), `class`, `locked`, `warnings` (`manual_apn`, `roaming_off`; on inactive operational profiles) |
| `revision` | changes whenever the card or its profiles change; writes must quote it |
| `freshness` | `age_ms`, `stale`, `error: {kind, message}` (`sim_reloading`, `busy`, `management_unavailable`, `read_failed`, …). A failed read keeps the last good list |
| `operation` | the latest change: `id`, `kind` (`enable`/`disable`/`nickname`/`download`/`delete`), `target` (a handle; `null` for a download until the installed profile has been read back), `target_name`, `previous`, `state`, `resolved`, `message`, and for downloads `step` (the helper's step name), `preview` (`{provider, name, class}` from the server; never an ICCID), `can_cancel`, `preview_expires_in_ms`. These four are `null`/`false` for other kinds |
| `receipts` | operator receipts queued by this agent (its own changes, or a Send receipts run): `pending`, `last_error`, `gave_up`, `sending` (a run is in progress) |
| `actions` | `enabled`, `blocked_by` (`helper_missing`, `no_inventory`, `operation_in_progress`) |
| `hardware_validated` | `switching: true` once switching was tested on the device (it was, 2026-10-24) |

Raw ICCIDs and the EID never leave the agent.

**Requests.**

| Route | Body | Answers |
|---|---|---|
| `GET /api/esim/profiles` | — | 200 view; re-reads the card at most every 20 s, never behind another card session |
| `POST /api/esim/refresh` | — | 200 view after a serialized fresh read (skipped while a change is settling) |
| `POST /api/esim/enable` | `handle`, `revision`, `operation_id` | 202 view, settled in the background. `X-Confirm: true` and a password session required |
| `POST /api/esim/disable` | same | same; leaves no profile active |
| `POST /api/esim/nickname` | `handle`, `revision`, `operation_id`, `nickname` (≤ 64 bytes UTF-8, no control or invisible formatting characters; empty clears) | 200 view, or 409 with the reason |
| `POST /api/esim/lock` | `handle`, `locked` | 200 view. An agent-side guard: a locked profile cannot be activated or deleted. Password session required |
| `POST /api/esim/download` | `activation_code` (`LPA:1$<server>$<matching ID>[$<OID>[$1]]`), `confirmation_code` (required when the code ends `$1`), `operation_id` | 202 view; the download runs in the background. 400 invalid code or missing confirmation code; 409 a change in flight, "the router has no mobile data connection", or no card read yet; 503 helper missing. Password session required |
| `POST /api/esim/download/confirm` | `operation_id`, `install` (bool), optional `nickname` (Rename rules) | 200 view. Only while `awaiting_confirmation` for that `operation_id` (else 409). `install: false` declines (`endUserRejection`). Password session required |
| `POST /api/esim/download/cancel` | `operation_id` | 200 view; postpones the order with the operator, so a single-use QR stays usable. 409 once `installing` or when no download is running. Password session required |
| `POST /api/esim/delete` | `handle`, `revision`, `operation_id` | 202 view, settled in the background. Inactive, unlocked profiles only (409 active or stale, 423 locked, 404 unknown). `X-Confirm: true` and a password session required |
| `POST /api/esim/receipts/send` | `{}` | 202 view: delivers every receipt on the card in the background, removing each only after its server accepts it. 200 with nothing to send; 409 while a change is in flight. Never runs on its own. Password session required |

**Codes are secrets.** Activation and confirmation codes are accepted once and
never appear in any answer, log, journal or error text. The dashboard keeps
them only in the open form; the mock agent redacts them in its request log.

`operation_id` is 8–64 of `[A-Za-z0-9_-]`, chosen by the client. Repeating it
answers 200 with the operation it already started; nothing is sent twice.

Errors: 400 malformed body or missing `X-Confirm`; 403 PIN session on a
password-only route; 404 unknown handle; 409 stale `revision`, a change still
in progress, a reused `operation_id`, a non-operational profile or the wrong
state; 423 locked profile; 503 helper missing or no profile list yet.

**Operation states.** `pending` → `reloading` (accepted; the modem reloads the
SIM, during which the card cannot be read for 5–25 s) or `verifying` (the
answer was lost, or the agent restarted: reading back) → `succeeded`, `busy`
(card refused, SIM Toolkit busy; nothing changed; never retried
automatically), `refused`, `failed` or `unknown` (could not be read back).
A download goes `connecting` (authenticating with the operator's server) →
`awaiting_confirmation` (the preview; nothing written yet; cancelled after 2
minutes without an answer) → `installing` (cannot be cancelled) →
`succeeded`, `cancelled`, `failed` or `unknown`.
A write is never retried and a restart reads back, never replays. "Active"
means selected on the card, not connected.

## Safety constraints built into the agent

- **AT console is allowlisted** (`server.rs`): read-only commands only;
  `AT+CFUN`, `AT^…`, `AT+CMGD`, `AT$QCRMCALL`, `AT+CLCK`, `AT+CGDCONT=`,
  `AT+CGACT=` are blocked.
- **kill-bloat only kills daemons that are safe to kill** — never the
  `zte_topsw_daemon.conf` sync-barrier set (see SAFETY.md).
- **Destructive endpoints require `X-Confirm: true`** (`/api/device/reboot`,
  `/api/device/shutdown`, `/api/system/kill-bloat`, `/api/esim/enable`,
  `/api/esim/disable`, `/api/esim/delete`).
- **eSIM switching, locks, downloads, deletes and receipt runs need a password
  session**; the eSIM helper may only send the ES10 commands each operation
  needs (no APDU passthrough).
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
