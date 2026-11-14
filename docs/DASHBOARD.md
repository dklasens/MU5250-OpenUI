# Dashboard

React 19 + Vite + Tailwind SPA served from the device itself (isolated
upstream OpenWrt uhttpd on port 8080, release files selected by `/data/www.current` (legacy `/data/www` fallback)), talking to the
agent on port 9090. It is kept separate from ZTE's patched stock-UI uhttpd,
whose singleton ubus object makes a second UCI instance unreliable.

## Layout

Five navigation groups — bottom tab bar on phones, sidebar on desktop,
light/dark theme (auto + manual), each group lazy-loaded:

| Group | Contents |
|---|---|
| **Home** | signal, modem mode, throughput, battery, connection, device, data usage — backed by the single batched `/api/dashboard` poll |
| **Signal** | per-carrier LTE/NR detail (PCI, ARFCN, RSRP/RSRQ/SINR) + network mode, band lock, one-tap cell lock from live cells |
| **Network** | clients by Wi-Fi/USB-C/Ethernet with link details, per-band Wi-Fi configuration, LAN/DHCP and DNS |
| **Modem** | APN profiles (with carrier presets), data usage + reset day, TTL clamping, SMS (inbox/sent, compose, delete) |
| **System** | thermals, battery health, charge control, signal/connection loggers, AT console, on-demand process list, device/SIM info, USB mode + powerbank, power actions |

The agent exposes exactly what these screens use and nothing else — the
unsurfaced extras (DoH proxy, speed test, scheduler, SMS forwarding, SIM PIN
flows, calls/USSD/STK) were removed rather than left dangling. See
[AGENT.md](AGENT.md) for the route table, and
`scripts/check-api-contract.py`, which fails if the two drift apart.

## Source layout

```
web-app/src/
  App.tsx            auth gate + group switching (lazy-loaded)
  app/               shell (sidebar/bottom tabs), login, theme, home poll context
  data/
    client.ts        token handling, envelope unwrapping, timeouts
    api.ts           endpoint bindings + firmware response mappers
    poll.ts          usePoll / useResource: one owner per resource (see Resource model)
    pollScheduler.ts non-overlapping, visibility-aware scheduler behind poll.ts
    signalQuality.ts the single RSRP/RSRQ/SINR rating policy
    bands.ts         band-lock parsing (LTE mask, NR SA / NSA lists) and "All bands"
    usage.ts, dates.ts, wifiWidth.ts, validate.ts   pure mappers and validators
  ui/                design-system primitives (cards, controls, toast, native-dialog confirm)
  icons.tsx          inline SVG icon set (no icon dependency)
  features/
    home/            Overview — single batched /api/dashboard poll
    signal/          Overview + Mode & Locking
    network/         Clients + Wi-Fi + Router
    modem/           APN + Data + TTL + SMS
    system/          Metrics + Tools + Settings
```

## Conventions that keep the device happy

- Home is one batched request (`/api/dashboard`), not nine calls. It is the
  only heartbeat: every 3 s on Home, Signal and System → Metrics (which reads
  CPU and memory from it via `useFastHeartbeat`), every 15 s elsewhere (where
  it only feeds the alert banner). Nothing re-polls data it already carries.
- Pollers never overlap (next poll scheduled after the previous completes)
  and pause while the browser tab is hidden.
- Expensive endpoints (`/api/network/clients`, `/api/system/top`) poll
  slowly (15 s) or load on demand.

## Resource model

`usePoll(key, fn, intervalMs)` in `src/data/poll.ts` owns one resource's reads.
`useResource(key, fn)` is the same with a one-shot read (`intervalMs = null`).
Both return `{ data, error, status, refreshing, refresh, mutate }`.

- `status` is `loading` (no data, no failure yet), `ready`, `error` (the first
  read failed, nothing to show) or `stale` (a later read failed; `data` is the
  last good value). Screens render these states instead of defaulting to zero.
- `key` must include every parameter that changes the result. On a key change
  the hook shows that key's cached value, or `loading`, never the previous
  key's data or error.
- A one-shot read runs again only on `refresh()` (the Retry buttons); returning
  to a hidden tab does not re-read it.
- `refresh()` and `mutate()` discard older in-flight reads, so a slow read
  cannot overwrite a mutation's reply.
- The last-good cache is module-level so group switches render instantly. It is
  cleared (`clearPollCache`) on logout and when the session expires.

## Displayed values

- **Signal ratings.** One policy in `src/data/signalQuality.ts` drives every
  rating, legend and colour. Lower bounds are inclusive:

  | Metric | Excellent | Good | Fair | Poor |
  |---|---|---|---|---|
  | RSRP, dBm | >= -80 | -90 to -80 | -100 to -90 | < -100 |
  | RSRQ, dB | >= -10 | -15 to -10 | -20 to -15 | < -20 |
  | SINR, dB | >= 20 | 10 to 20 | 0 to 10 | < 0 |

  RSSI is shown but not rated. Missing or non-numeric values show
  "Unavailable". Ratings are product heuristics, not a standard or a
  throughput guarantee. See [web-app/design.md](../web-app/design.md).
- **Primary carrier.** Home and Signal use the mapper's validated primary
  carrier: SA uses the NR PCC; LTE and NSA use the LTE anchor PCC. Unknown bars
  and measurements show Unavailable rather than a guessed value.
- **Data usage.** Counters are nullable; an unknown counter is never shown as
  0. The cycle block is titled "Current cycle". Reset day, enablement, cycle
  start and next reset come from the device. The firmware reports dates in two
  formats (`YYYY/MM/DD` and `YYYYMMDD`); both are parsed as calendar dates
  without a timezone conversion. `reset_enabled: null` reads "unknown", not
  "disabled". `since_power_on` is shown as "Connection counters", because its
  `real_*` values reset with the data connection. Saving a reset day also
  enables automatic reset, and the button says so.
- **Band locks.** Each lock is `unknown` (not reported), `automatic` (`0` or
  empty) or `locked`. LTE is a bitmask; NR SA and NSA are separate lists and
  are read separately. A lock that covers every supported band shows
  "All bands".
  - Limitation: the modem's default LTE mask covers 22 bands, but the agent
    lists 24 lockable LTE bands (`LTE_BANDS` in `agent/src/cell.rs`; B66 and
    B71 are the extras). On the device's default LTE state the dashboard
    therefore shows 22 bands selected, not "All bands".
- **Cell locks.** `lock_nr_cell` (`pci,nr-arfcn,band`) and `lock_lte_cell`
  (`pci,earfcn`) from the netinfo payload are each `unknown` (missing or
  malformed), `none` (empty, or all zeros as left by a cell-lock reset) or
  `locked`. Each cell-lock card shows the current lock and compares it with
  that RAT's serving primary cell (PCI, ARFCN and, for NR, band). A lock to a
  cell the modem is not on shows a warning with "Reset cell locks" beside it.
  The state shown is always the device's last read, never the request just sent.
- **Wi-Fi.** TX power is the real percentage (1-100). Channel widths are
  compared as MHz, so `VHT80` and `80 MHz` agree, and an unknown width never
  raises a mismatch. Channel advice is limited to configuration facts; no
  neighbouring-network scan exists, so interference is not claimed.
- **USB.** The Debug (ADB) option is gone; the agent rejects it. The
  `mode_capabilities` list is authoritative, and `supported_modes` is only the
  fallback when it is absent. The active mode, a scheduled (accepted, not yet
  confirmed) mode and the boot default are shown separately.
- **SMS.** The page holds one validated message collection; Inbox and Sent are
  derived from it, and the unread count always comes from the inbox.
- **Alerts.** Alerts derived from the heartbeat have stable ids per condition.
  Dismissing ends at the condition's recovery (an "episode"); a recurrence
  shows again, and a worse level (warning to error) shows again. Assistive
  technology is told only about new episodes and escalations; changing detail
  text updates silently.

## Editing and confirming changes

- **Drafts survive polling.** Band selections, per-band Wi-Fi fields, the
  charge limit (Apply / Cancel), TTL, network mode and APN mode keep the
  user's edits across heartbeats. On the locking page an untouched draft
  follows the device; an edited draft is kept and a different device value is
  flagged as a conflict.
- **Confirm before submit.** Actions that can drop the connection are
  confirmed before anything is sent: Wi-Fi off, per-band changes, save and
  sync; APN mode and activation; network mode; band lock and reset; cell lock
  and reset; USB mode; LAN. The reviewed payload is frozen before the dialog
  opens and exactly that payload is sent after Confirm. Cancel sends nothing.
- **Accepted is not verified.** After a request the UI says it was accepted,
  then reads back and reports verified, mismatch or unverified. Verification is
  bounded, and the dashboard never retries a mutation automatically. A reply
  that never arrives offers a read-only check, not a resend.
- Cell-lock requests are unchanged apart from the confirmation and layout;
  the observed lock is read back from the heartbeat (see "Cell locks" above).

## Accessibility

- Confirmations use one native `<dialog>` host. Cancel has initial focus for
  `danger` and `connection` kinds.
- Tabs use roving focus with manual activation (arrows move focus, Enter or
  Space activates). Exclusive choices are radiogroups; multi-select chips use
  `aria-pressed`.
- Every field has a visible label. Toasts go to live regions; error toasts
  persist until dismissed.
- Focus ring is 2 px accent with a 2 px offset, at least 3:1 against its
  surface. Under a coarse pointer every control is at least 44 x 44 px, and
  rows stack on phones.
- The rules live in [web-app/design.md](../web-app/design.md).

## Develop

Requires Node.js `^20.19.0 || >=22.12.0` and npm. Node 18 is unsupported by
the locked Vite toolchain.

```sh
cd web-app
npm ci
npm run dev       # local dev server (expects agent at <hostname>:9090)
npm run build     # tsc + vite build -> dist/
npm run lint
npm test                 # Node suites: mappers, poller, pure view logic
npm run test:browser     # Playwright browser harness (below)
```

Deploy to the device with `./deploy-dashboard.sh` from the repo root. The
script checks Node/npm, runs `npm ci`, builds and uploads the SPA, restarts the
isolated dashboard server, and verifies the page from the device before
reporting success. Run `scripts/zharden.sh` first if that server is not yet
installed.

### Local demo without the device

```sh
cd web-app
bash tools/demo.sh        # dashboard on :8080 + mock agent on :9090
bash tools/demo.sh stop
```

The mock agent (`tools/mock_agent.py`, stdlib-only) serves realistic U60 Pro
data — Telstra ENDC with LTE anchor + n78 NR, live-jittering throughput,
battery, clients, thermals — so every screen can be reviewed without
hardware. Sign in with any password.

The mock is stateful: mutations validate like the agent and change what the
next read reports (an APN activation flips APN mode, a Wi-Fi TX write shows in
`/api/wifi/status`, a band lock shows in the netinfo fields). Test hooks outside
the agent contract: `GET /__mock/requests` lists recorded non-GET requests,
`POST /__mock/reset` restores initial state (optionally `{"scenario": ...}`).
`MOCK_SCENARIO` (or `--scenario`) picks the radio: `SA`, `NSA`, `LTE` or
`disconnected`. Its behaviour is covered by `tests/test_mock_agent.py`.

### Browser tests

`npm run test:browser` runs Playwright (Chromium only, `@playwright/test`
1.63.0) against the Vite dev server. Every `/api` request is answered by a
synthetic agent in the test process; requests to any other host, including
`192.168.0.1`, are aborted and fail the test, so it cannot reach a real agent.
Set `PW_PORT` to choose the dev-server port (default 5199); with it set,
failure artefacts go to `test-results/port-<PW_PORT>` so parallel runs do not
collide. The suite runs in CI
(`.github/workflows/checks.yml`). Details:
[web-app/tests/browser/README.md](../web-app/tests/browser/README.md).

### API contract check

`python3 scripts/check-api-contract.py` compares the agent route table, the
dashboard bindings and the mock agent as **method + path** pairs, so a GET
binding for a PUT-only route fails. `tests/test_api_contract.py` tests the
checker itself.
