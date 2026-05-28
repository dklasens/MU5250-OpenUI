# Open U60 Pro

Credit to: https://github.com/jesther-ai/open-u60-pro (which this project is based on). 

Open U60 Pro is a custom control plane for the ZTE U60 Pro. It replaces the stock web UI's limited controls with:

- a Rust agent that runs on the modem and exposes a JSON API on `http://192.168.0.1:9090`
- a React web app that talks to that agent and is typically served from the modem at `http://192.168.0.1:8080`
- install and deploy tooling for provisioning the device and pushing updates

This README focuses on two things:

1. what the agent API actually exposes
2. what the current web app surfaces, monitors, and lets you configure

## Project Layout

- `installer/`: setup UI/data and provisioning flow
- `agent/`: Rust backend that talks to `ubus`, AT ports, sysfs, procfs, iptables, and other device services
- `web-app/`: React/Vite single-page dashboard
- `deploy.sh`: pushes the agent to the modem
- `deploy-dashboard.sh`: builds and pushes the web app

## Runtime Model

- The agent binds to `192.168.0.1:9090` by default.
- The bind address can be overridden with `ZTE_AGENT_BIND`.
- Worker count can be overridden with `ZTE_AGENT_THREADS`.
- The API password is loaded from `ZTE_AGENT_PASSWORD` or from `/data/local/tmp/start_zte_agent.sh`.
- An optional 6 digit dashboard PIN can be loaded from `ZTE_AGENT_PIN` or from `/data/local/tmp/start_zte_agent.sh`.
- On startup the agent also:
  - starts the DoH proxy subsystem
  - starts the scheduler
  - starts SMS forwarding event handling
  - reapplies persisted TTL rules from `/data/local/tmp/start_ttl.sh`
  - reapplies Wi-Fi state persistence logic

## API Contract

### Authentication and safety

- `POST /api/auth/login` is the only unauthenticated endpoint.
- Login accepts either the agent password or, from mobile user agents only, the configured dashboard PIN.
- All other endpoints require `Authorization: Bearer <token>`.
- Tokens are in-memory session tokens with a 1 hour TTL.
- Up to 10 tokens are retained at once.
- Login attempts are rate limited per client IP after 5 failed attempts, with a 30 second lockout.
- CORS is only opened for LAN-style origins (`localhost`, `127.0.0.1`, `::1`, `10.x.x.x`, `172.16-31.x.x`, `192.168.x.x`).
- Destructive endpoints currently require `X-Confirm: true`:
  - `POST /api/device/reboot`
  - `POST /api/device/shutdown`
  - `POST /api/device/factory-reset`

### Response shape

The agent returns JSON in a consistent envelope:

```json
{ "ok": true, "data": { "...": "..." } }
```

or:

```json
{ "ok": false, "error": "..." }
```

### Example login

```bash
curl -sS http://192.168.0.1:9090/api/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"password":"your-agent-password"}'
```

Optional mobile PIN login:

```bash
curl -sS http://192.168.0.1:9090/api/auth/login \
  -A 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) Mobile/15E148' \
  -H 'Content-Type: application/json' \
  -d '{"pin":"123456"}'
```

## Agent API Surface

The canonical routing table lives in `agent/src/server.rs`. The API is broader than the current UI; the sections below describe the exposed endpoint families.

### Auth

- `POST /api/auth/login`

### Dashboard, device, and system

Read-only and aggregated status:

- `GET /api/dashboard`
- `GET /api/device`
- `GET /api/battery`
- `GET /api/cpu`
- `GET /api/memory`
- `GET /api/device/battery-info`
- `GET /api/device/thermal`
- `GET /api/device/thermal/all`
- `GET /api/device/battery/detail`
- `GET /api/device/charger`
- `GET /api/device/system`
- `GET /api/system/top`

Device-control and service actions:

- `POST /api/device/reboot`
- `POST /api/device/shutdown`
- `POST /api/device/factory-reset`
- `POST /api/device/power-save`
- `PUT /api/device/power-save`
- `GET /api/device/fast-boot`
- `PUT /api/device/fast-boot`
- `POST /api/system/restart-agent`
- `POST /api/system/kill-bloat`

Note: the `power-save` pair is slightly unconventional: `POST` reads device-manager state and `PUT` writes it.

### Network status and clients

- `GET /api/network/signal`
- `GET /api/network/traffic`
- `GET /api/network/speed`
- `GET /api/network/wan`
- `GET /api/network/wan6`
- `GET /api/network/lan-status`
- `GET /api/network/clients`
- `GET /api/network/speeds`
- `GET /api/network/rmnet`

### Wi-Fi

- `GET /api/wifi/status`
- `PUT /api/wifi/settings`
- `GET /api/wifi/guest`
- `PUT /api/wifi/guest`

These endpoints back both status reporting and band-specific configuration such as SSID, key, hidden mode, channel, bandwidth, radio enable state, persistence flags, and related Wi-Fi toggles.

### Modem and mobile data

- `GET /api/data-usage`
- `PUT /api/data-usage/reset-day`
- `GET /api/modem/status`
- `POST /api/modem/online`
- `GET /api/modem/data`
- `PUT /api/modem/data`
- `POST /api/modem/airplane`
- `PUT /api/modem/network-mode`
- `POST /api/modem/scan`
- `GET /api/modem/scan/status`
- `GET /api/modem/scan/results`
- `POST /api/modem/register`
- `GET /api/modem/register/result`

### Cell, band lock, and radio diagnostics

- `POST /api/cell/lock/nr`
- `POST /api/cell/lock/lte`
- `POST /api/cell/lock/reset`
- `POST /api/cell/neighbors/scan`
- `GET /api/cell/neighbors/nr`
- `GET /api/cell/neighbors/lte`
- `POST /api/cell/band/nr`
- `POST /api/cell/band/lte`
- `POST /api/cell/band/reset`
- `GET /api/cell/stc/params`
- `PUT /api/cell/stc/params`
- `GET /api/cell/stc/status`
- `POST /api/cell/stc/enable`
- `POST /api/cell/stc/disable`
- `POST /api/cell/stc/reset`
- `POST /api/cell/signal-detect/start`
- `POST /api/cell/signal-detect/stop`
- `GET /api/cell/signal-detect/results`
- `GET /api/cell/signal-detect/progress`

### Router and LAN services

- `GET /api/router/dns`
- `PUT /api/router/dns`
- `GET /api/router/lan`
- `PUT /api/router/lan`
- `GET /api/router/firewall`
- `PUT /api/router/firewall/switch`
- `PUT /api/router/firewall/level`
- `PUT /api/router/firewall/nat`
- `PUT /api/router/firewall/dmz`
- `GET /api/router/firewall/upnp`
- `PUT /api/router/firewall/upnp`
- `GET /api/router/firewall/port-forward`
- `POST /api/router/firewall/port-forward`
- `PUT /api/router/firewall/port-forward/switch`
- `GET /api/router/firewall/filter-rules`
- `GET /api/router/vpn`
- `PUT /api/router/vpn`
- `GET /api/router/qos`
- `PUT /api/router/qos`
- `GET /api/router/domain-filter`
- `PUT /api/router/domain-filter`
- `GET /api/router/apn/mode`
- `PUT /api/router/apn/mode`
- `GET /api/router/apn/profiles`
- `POST /api/router/apn/profiles`
- `PUT /api/router/apn/profiles`
- `GET /api/router/apn/auto-profiles`
- `POST /api/router/apn/profiles/delete`
- `POST /api/router/apn/profiles/activate`

### USB

- `GET /api/usb/status`
- `PUT /api/usb/mode`
- `PUT /api/usb/powerbank`

### SMS and SMS forwarding

SMS mailbox operations:

- `POST /api/sms/list`
- `GET /api/sms/capacity`
- `POST /api/sms/send`
- `POST /api/sms/delete`
- `POST /api/sms/read`

Forwarding and rules:

- `GET /api/sms/forward/config`
- `PUT /api/sms/forward/config`
- `POST /api/sms/forward/rules`
- `PUT /api/sms/forward/rules`
- `DELETE /api/sms/forward/rules`
- `PUT /api/sms/forward/rules/toggle`
- `POST /api/sms/forward/test`
- `GET /api/sms/forward/log`
- `POST /api/sms/forward/log/clear`
- `POST /api/sms/forward/retry`

### SIM and telephony

SIM and lock management:

- `GET /api/sim/info`
- `GET /api/sim/imei`
- `POST /api/sim/pin/verify`
- `POST /api/sim/pin/change`
- `POST /api/sim/pin/mode`
- `POST /api/sim/unlock`
- `GET /api/sim/lock-trials`

Calls, USSD, and STK:

- `POST /api/call/dial`
- `POST /api/call/hangup`
- `POST /api/call/answer`
- `GET /api/call/status`
- `POST /api/call/dtmf`
- `POST /api/call/mute`
- `POST /api/ussd/send`
- `POST /api/ussd/respond`
- `POST /api/ussd/cancel`
- `GET /api/stk/menu`
- `POST /api/stk/select`

### DNS-over-HTTPS, testing, automation, and logging

DoH proxy:

- `GET /api/doh/status`
- `PUT /api/doh/config`
- `POST /api/doh/enable`
- `POST /api/doh/disable`
- `GET /api/doh/cache`
- `POST /api/doh/cache/clear`

LAN tests:

- `GET /api/lan/ping`
- `GET /api/lan/download`
- `POST /api/lan/upload`

Speed test:

- `GET /api/speedtest/servers`
- `POST /api/speedtest/start`
- `GET /api/speedtest/progress`
- `POST /api/speedtest/stop`

Scheduler:

- `GET /api/scheduler/jobs`
- `POST /api/scheduler/jobs`
- `PUT /api/scheduler/jobs`
- `DELETE /api/scheduler/jobs`
- `PUT /api/scheduler/jobs/toggle`

TTL clamping:

- `GET /api/ttl/status`
- `PUT /api/ttl/set`
- `DELETE /api/ttl/clear`

AT console:

- `POST /api/at/send`

The AT console is intentionally read-only. The agent blocks destructive prefixes such as `AT+CFUN`, `AT^`, SIM lock changes, and PDP activation commands.

Signal and connection loggers:

- `POST /api/logger/signal/start`
- `POST /api/logger/signal/stop`
- `GET /api/logger/signal/status`
- `GET /api/logger/signal/download`
- `POST /api/logger/connection/start`
- `POST /api/logger/connection/stop`
- `GET /api/logger/connection/status`
- `GET /api/logger/connection/download`

## What the Web App Exposes Today

The current SPA navigation is defined in `web-app/src/App.tsx` and `web-app/src/components/Sidebar.tsx`. These are the pages that are actually exposed in the shipped UI today.

### Dashboard

Monitoring only:

- signal summary, signal bars, active RAT and band
- battery percentage, charging state, voltage, temperature
- live download and upload rates with peaks
- device model, firmware, uptime, CPU, memory
- WAN IPv4/IPv6, gateway, DNS
- daily, monthly, and lifetime data usage

### Signal

Monitoring only:

- LTE and NR carrier decomposition
- PCC/SCC breakdown
- PCI, EARFCN/NR-ARFCN, frequency, bandwidth
- per-carrier RSRP, RSRQ, SINR, RSSI
- simple inline explanations for signal quality metrics

### Connected

Monitoring only:

- connected client count by Wi-Fi, USB-C, Ethernet, and other
- Wi-Fi hostname/IP/MAC/radio/signal/link rates
- wired and USB-C client interface and link speed details

### Wi-Fi

Configurable in the current UI:

- global Wi-Fi master switch when firmware exposes it reliably
- reboot persistence for Wi-Fi enabled/disabled state
- copy 2.4 GHz settings to 5 GHz, or the reverse
- per-band enable/disable for 2.4 GHz and 5 GHz
- per-band SSID
- per-band password
- per-band hidden SSID flag
- per-band configured channel
- per-band configured bandwidth
- per-band TX power preset

Visible status in the current UI:

- actual channel vs configured channel
- actual bandwidth vs configured bandwidth
- client count per band
- security mode
- Wi-Fi 6 support/status when exposed
- guest SSID display
- channel/bandwidth advisory text

### Router

Configurable in the current UI:

- LAN IP address
- LAN netmask
- DHCP start address
- DHCP end address
- DHCP lease time
- manual IPv4 DNS servers
- manual IPv6 DNS servers
- quick DNS presets for Cloudflare, Google, and Quad9

### Modem

Configurable in the current UI:

- APN mode: automatic or manual
- add manual APN profiles
- delete APN profiles
- activate a specific APN profile
- quick-fill APN presets for common carriers
- enable TTL clamping
- update TTL/Hop Limit value
- disable TTL clamping

Visible in the current UI:

- current TTL clamp status, including IPv6 state
- current reset-day billing cycle usage
- upload, download, and total usage for the current cycle
- data since power on
- daily and device lifetime counters

### Band & Cell Locking

Configurable in the current UI:

- network mode selection:
  - `5G + 4G`
  - `5G SA`
  - `5G NSA`
  - `4G LTE`
  - `3G`
- NR band locks
- LTE band locks
- NR cell lock by PCI + NR-ARFCN + band
- LTE cell lock by PCI + EARFCN
- one-click lock actions from currently active serving carriers
- reset band locks
- reset cell locks

Visible in the current UI:

- parsed and raw LTE/NR band lock state
- current network mode reported by the modem

### Metrics

Monitoring only:

- thermal sensors for CPU, modem, PA, SDR, battery, USB, Ethernet PHY, PMIC, XO
- per-core CPU temperatures
- battery capacity, status, power, voltage, current, charge type, time to full/empty
- charge counter, remaining capacity to full, voltage headroom, loaded-vs-OCV delta, C-rate
- battery health, design capacity vs current full capacity, cycle count
- hardware vs software fuel-gauge flag

### Advanced

Configurable in the current UI:

- signal logger start/stop with selectable duration and sample interval
- signal logger CSV download
- connection event logger start/stop with selectable duration and poll interval
- connection event logger CSV download
- AT command console with configurable timeout

Visible in the current UI:

- logger run state, elapsed time, duration, sample/event counts
- AT command history and responses

### Settings

Visible in the current UI:

- device metadata, firmware, uptime, IMEI
- SIM state, ICCID, IMSI, MCC/MNC
- memory usage
- top processes
- current API base URL and dashboard URL

Configurable in the current UI:

- restart the backend agent
- reload the dashboard
- reboot the device with confirmation
- shut down the device with confirmation
- USB mode switch:
  - `RNDIS`
  - `ECM`
  - `NCM` (experimental configfs path, not exposed by the stock ZTE switch)
  - `DEBUG`
- optional persisted NCM-after-boot default, applied by the agent after the
  stock USB stack has settled
- sign out

## API Features Not Yet Surfaced In The Current Navigation

The backend already exposes more than the current SPA navigation uses. At the time of writing, the API includes capabilities that are not currently represented as top-level UI pages or active controls in `App.tsx`, including:

- SMS mailbox operations and SMS forwarding workflows
- modem airplane/data toggles, operator scan, and manual registration
- DoH proxy configuration and cache inspection
- speedtest server selection and execution
- scheduler jobs
- SIM PIN/PUK flows
- voice call, USSD, and STK actions
- router firewall, NAT, DMZ, UPnP, port-forward, QoS, VPN passthrough, and domain-filter controls
- USB power-bank mode

There is also an `SmsPage.tsx` in the web app source tree, but it is not currently mounted in the main app navigation.

## Development And Deployment

### Agent

- source: `agent/`
- default target runtime: native on-device
- default bind: `192.168.0.1:9090`

Useful env vars:

- `ZTE_AGENT_PASSWORD`
- `ZTE_AGENT_BIND`
- `ZTE_AGENT_THREADS`

### Web app

- source: `web-app/`
- API base logic: `http://<current-host>:9090`
- session token storage: `sessionStorage`

### Deploy scripts

- `./deploy.sh`: build and push the Rust agent, then restart it on-device
- `./deploy-dashboard.sh`: build and push the web dashboard assets

## Source Of Truth

If this README and the code ever disagree, use these files as the authoritative references:

- `agent/src/server.rs`: HTTP routing table
- `agent/src/auth.rs`: auth and token behavior
- `web-app/src/App.tsx`: pages actually mounted in the UI
- `web-app/src/api.ts`: client-side API bindings and payload shapes
