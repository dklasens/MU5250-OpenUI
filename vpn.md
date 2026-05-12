# VPN Page Implementation Plan

This document outlines a proposed implementation for adding a NordVPN client feature to the ZTE U60 Pro dashboard.

The target user experience is a new dashboard page called `VPN` where a user can select a NordVPN endpoint, connect the router, and route client traffic through that VPN until disconnected.

No credentials, Nord tokens, passwords, or PINs should be committed to the repository.

## Current Device Findings

Read-only checks against the device showed:

- OS: OpenWrt `23.05.4`
- Kernel: `5.15.170`
- Architecture: `aarch64_cortex-a53`
- Target: `sdx75/generic`
- `/dev/net/tun` exists
- `opkg` exists
- `curl` exists
- `ip`, `iptables`, `nft`, `uci`, and `service` exist
- `nordvpnlite` is not currently installed
- `openvpn` is not currently installed
- `wg` / `wireguard-tools` are not currently installed
- `pbr-iptables` is installed but disabled
- Device has enough storage headroom for a small VPN package:
  - `/` had roughly `192 MB` free
  - `/data` had roughly `1.7 GB` free
  - `/etc` overlay had roughly `90 MB` free
- Device had enough RAM headroom at the time checked:
  - about `681 MB` available

The device is a reasonable candidate for a VPN client, but the first implementation should be treated as a controlled prototype because ZTE's OpenWrt build includes custom networking, firewall, and modem integration.

## Recommended Approach

Use Nord's `nordvpnlite` package first.

Reasons:

- NordVPN publishes an OpenWrt-specific Lite client.
- NordVPN publishes an `aarch64_cortex-a53` package, matching this device architecture.
- It should provide a simpler control surface than hand-building NordLynx/WireGuard config.
- It should perform better than OpenVPN.
- It avoids us having to reverse-engineer Nord's server discovery and authentication flow up front.

Alternatives:

- OpenVPN:
  - Easier to understand and widely supported.
  - Likely slower and more CPU-heavy.
  - Not recommended as the first implementation.
- Manual WireGuard/NordLynx:
  - Potentially fastest.
  - More complex because Nord's credential/key and server-selection flow would need careful handling.
  - Not recommended until `nordvpnlite` has been evaluated.

Official reference links:

- NordVPN Lite OpenWrt guide: <https://support.nordvpn.com/hc/en-us/articles/41793554983953-How-to-set-up-NordVPN-on-an-OpenWRT-router-using-NordVPN-Lite>
- NordVPN Lite downloads: <https://downloads.nordcdn.com/nordvpnlite/>
- NordVPN manual OpenWrt OpenVPN guide: <https://support.nordvpn.com/hc/en-us/articles/20340177222289-How-to-manually-set-up-NordVPN-on-an-OpenWRT-router>

## Product Goals

The VPN page should provide:

- VPN install readiness/status.
- Nord token setup without committing secrets.
- Country or server selection.
- Connect/disconnect controls.
- Clear connection status.
- Current VPN endpoint details.
- Public IP before and after VPN connection.
- DNS leak/IPv6 leak awareness.
- Kill-switch option.
- Logs and troubleshooting information.

The first version should prioritise reliability over advanced routing features.

## Non-Goals For Initial Version

Do not implement these in the first pass:

- Per-client VPN routing.
- Split tunnelling by domain.
- Split tunnelling by device.
- Multi-provider VPN support.
- Custom WireGuard config import.
- Automatic "fastest server" benchmarking across many regions.
- Account login via username/password.
- Storing Nord account credentials.

Those can be added later after whole-router NordVPN routing is stable.

## Security Requirements

### Secret Handling

Nord token handling must follow these rules:

- Never commit the Nord token.
- Never write the token into repo files.
- Never print the token in logs.
- Never return the token from API responses.
- Store the token only on the device.
- Store device-side config with restrictive permissions.

Recommended device paths:

- Runtime/config directory: `/data/zte-agent/vpn`
- Nord token file: `/data/zte-agent/vpn/nord_token`
- VPN state file: `/data/zte-agent/vpn/state.json`
- Logs: `/data/zte-agent/vpn/logs`

Recommended permissions:

```sh
chmod 700 /data/zte-agent/vpn
chmod 600 /data/zte-agent/vpn/nord_token
```

If `nordvpnlite` requires its own config path, the agent should still own token provisioning and ensure permissions are sane.

### API Redaction

API responses should redact:

- Nord token.
- Private keys.
- Any generated WireGuard private key.
- Session secrets.
- Full command lines containing secrets.

### Authentication

All VPN endpoints must require dashboard auth.

State-changing endpoints should require:

- Existing bearer token auth.
- Confirmation header for destructive/high-impact actions if consistent with existing app patterns.

Examples:

- Connect: normal authenticated POST.
- Disconnect: normal authenticated POST.
- Kill-switch enable: authenticated PUT.
- Credential/token update: authenticated PUT and never echo value back.

## Proposed Dashboard UX

Add a new top-level page:

- Sidebar label: `VPN`
- Route/page id: `vpn`
- Component: `web-app/src/pages/VpnPage.tsx`

### Page Sections

#### 1. Status Header

Show:

- Connection state:
  - `Not installed`
  - `Disconnected`
  - `Connecting`
  - `Connected`
  - `Disconnecting`
  - `Error`
- Connected country/server.
- VPN interface name.
- VPN public IP.
- Uptime.
- Current route mode:
  - `All LAN traffic`
  - `Disconnected`
  - later: `Policy based`

#### 2. Setup / Readiness

Show if not installed or not configured:

- `nordvpnlite` package present: yes/no.
- Token configured: yes/no.
- TUN support: yes/no.
- Firewall route support: yes/no.
- DNS control support: yes/no.

Actions:

- Install package, if we decide to automate installation.
- Upload/set Nord token.
- Test Nord connectivity.

For the safest first version, package installation can remain manual, while the dashboard reports readiness. Once stable, installation can be automated.

#### 3. Server Selection

Initial version:

- Country selector.
- Optional city/region if `nordvpnlite` exposes it.

Later version:

- Server selector.
- Recommended/fastest server.
- Recent selections.
- Favourites.

The page should not assume exact server selection is possible until `nordvpnlite` command support is verified on-device.

#### 4. Controls

Buttons:

- Connect.
- Disconnect.
- Reconnect.
- Refresh status.

Optional toggles:

- Kill switch.
- Block IPv6 when VPN is active.
- Use VPN DNS.
- Auto-connect on boot.

#### 5. Traffic Routing Summary

Show:

- LAN subnet being routed.
- WAN interface used to reach VPN server.
- VPN interface.
- Default route state.
- DNS mode.
- IPv6 leak protection state.

#### 6. Logs / Diagnostics

Show:

- Last connection attempt.
- Last error.
- Recent logs.
- Public IP check result.
- DNS check result if implemented.

Keep logs concise by default and offer an expanded view.

## Proposed Agent API

Add a new Rust module:

- `agent/src/vpn.rs`

Register routes in:

- `agent/src/server.rs`

Expose frontend methods in:

- `web-app/src/api.ts`

### Endpoint List

```http
GET /api/vpn/status
GET /api/vpn/readiness
GET /api/vpn/countries
GET /api/vpn/servers
PUT /api/vpn/token
PUT /api/vpn/settings
POST /api/vpn/connect
POST /api/vpn/disconnect
POST /api/vpn/reconnect
GET /api/vpn/logs
GET /api/vpn/public-ip
```

### `GET /api/vpn/status`

Purpose:

- Return current VPN state.

Example response:

```json
{
  "installed": true,
  "configured": true,
  "state": "connected",
  "provider": "nordvpn",
  "client": "nordvpnlite",
  "country": "Australia",
  "server": "au123.nordvpn.com",
  "interface": "nordlynx",
  "vpn_ip": "10.5.0.2",
  "public_ip": "203.0.113.10",
  "uptime_secs": 1205,
  "kill_switch": true,
  "ipv6_block": true,
  "dns_mode": "vpn",
  "last_error": null
}
```

### `GET /api/vpn/readiness`

Purpose:

- Report whether the device can support VPN.

Checks:

- `nordvpnlite` binary exists.
- `/dev/net/tun` exists.
- `ip` exists.
- `iptables` or `nft` exists.
- `uci` exists.
- enough free space.
- token configured.
- VPN service/config present.

Example response:

```json
{
  "architecture": "aarch64_cortex-a53",
  "tun_available": true,
  "nordvpnlite_installed": false,
  "opkg_available": true,
  "iptables_available": true,
  "nft_available": true,
  "token_configured": false,
  "warnings": [
    "nordvpnlite is not installed",
    "IPv6 leak protection is not configured"
  ]
}
```

### `PUT /api/vpn/token`

Purpose:

- Store or update the Nord token on-device.

Request:

```json
{
  "token": "..."
}
```

Response:

```json
{
  "token_configured": true
}
```

Important:

- Do not return the token.
- Do not log the token.
- Validate only basic shape/length.
- Write atomically:
  - create temp file
  - chmod
  - rename into place

### `PUT /api/vpn/settings`

Purpose:

- Store VPN preferences.

Request:

```json
{
  "country": "Australia",
  "server": null,
  "kill_switch": true,
  "block_ipv6": true,
  "vpn_dns": true,
  "auto_connect": false
}
```

Store in:

```text
/data/zte-agent/vpn/settings.json
```

### `POST /api/vpn/connect`

Purpose:

- Connect to the selected VPN endpoint.

Request:

```json
{
  "country": "Australia",
  "server": null
}
```

Flow:

1. Validate token exists.
2. Validate `nordvpnlite` is installed.
3. Resolve selected country/server if needed.
4. Start `nordvpnlite`.
5. Wait for VPN interface or connected status.
6. Apply routing/firewall/DNS policy.
7. Verify public IP changed if possible.
8. Return status.

### `POST /api/vpn/disconnect`

Purpose:

- Disconnect VPN and restore normal routing.

Flow:

1. Stop/disconnect `nordvpnlite`.
2. Remove VPN routes/rules.
3. Remove VPN DNS override.
4. Remove kill-switch rules, unless configured to remain strict.
5. Verify default route is back through cellular WAN.
6. Return status.

### `GET /api/vpn/logs`

Purpose:

- Return recent VPN logs.

Rules:

- Limit output.
- Redact tokens and keys.
- Include command result summaries, not full secret-bearing command lines.

## `nordvpnlite` Integration Plan

### Phase 0: Manual Probe

Before writing dashboard automation, test on-device manually in a controlled branch/task.

Read-only preparation:

- Confirm architecture.
- Confirm free space.
- Confirm `/dev/net/tun`.
- Confirm package source.

Install test, only when explicitly approved:

```sh
opkg update
opkg install /path/to/nordvpnlite_latest_aarch64_cortex-a53.ipk
```

Then inspect:

```sh
command -v nordvpnlite
nordvpnlite --help
nordvpnlite status
```

The actual command names and output format need to be confirmed on the device before the agent wraps them.

### Phase 1: Status-Only Support

Implement:

- readiness endpoint.
- status endpoint.
- frontend status page.

Do not connect/disconnect yet.

Goal:

- Establish how to detect install/config/running state safely.

### Phase 2: Token Setup

Implement:

- `PUT /api/vpn/token`.
- local secret storage.
- readiness reflects configured token.

Goal:

- Token can be configured from dashboard without repo or log exposure.

### Phase 3: Connect/Disconnect

Implement:

- connect endpoint.
- disconnect endpoint.
- minimal UI controls.
- status polling.

Goal:

- VPN client can connect and disconnect reliably.

### Phase 4: Whole-Router Routing

Implement routing so LAN traffic uses VPN.

Possible approaches:

1. Use `nordvpnlite` route management if it handles default route and DNS correctly.
2. Add OpenWrt interface/firewall integration around the VPN interface.
3. Use explicit `ip route` / `ip rule` / `iptables` rules controlled by the agent.

Preferred:

- Use the native OpenWrt network/firewall model where possible.
- Avoid fragile one-off shell routing unless necessary.

### Phase 5: Kill Switch

Implement kill switch after routing is stable.

Goal:

- If VPN is configured/enabled but disconnected unexpectedly, LAN clients cannot leak to cellular WAN.

Required behavior:

- Permit router-to-VPN-server traffic over WAN.
- Permit LAN-to-router traffic for dashboard/DHCP/DNS.
- Block LAN-to-WAN forwarding unless VPN is connected.
- Allow LAN-to-VPN forwarding when VPN is connected.

Careful:

- A bad kill switch can lock clients out of internet access.
- It must never block dashboard access at `192.168.0.1`.
- It must be recoverable after reboot.

### Phase 6: DNS and IPv6 Leak Handling

DNS:

- Force LAN DNS to the router.
- Router forwards DNS through VPN or uses VPN-provided DNS.
- Prevent LAN clients from bypassing DNS to WAN while VPN is active if kill switch is on.

IPv6:

- Current device has IPv6 on cellular WAN.
- If VPN does not tunnel IPv6, IPv6 must either be disabled for LAN while VPN is active or explicitly blocked from forwarding to WAN.

Recommendation for initial version:

- Add a `Block IPv6 while VPN is active` setting.
- Default it to enabled.

## Routing Design Notes

Current WAN:

- Main cellular interface appears as `rmnet_data0`.
- OpenWrt interface is `zte_wan`.
- Cellular MTU is currently around `1428`.

Current LAN:

- LAN bridge is `br-lan`.
- LAN subnet is `192.168.0.0/24`.

Potential VPN interface:

- Must be discovered after `nordvpnlite` runs.
- Could be `nordlynx`, `wg0`, `tun0`, or another name.
- Do not hard-code until verified.

### Firewall Zone Design

If integrating with UCI firewall:

- Add a `vpn` firewall zone.
- Attach VPN network/interface to that zone.
- Add forwarding from `lan` to `vpn`.
- Remove or disable forwarding from `lan` to `wan` while kill switch is active.
- Enable masquerading on `vpn` zone if required.
- Apply MSS clamping.

Example conceptual firewall model:

```text
lan -> vpn: allowed
lan -> wan: blocked when kill switch is enabled and VPN is active/configured
router -> wan: allowed for VPN tunnel establishment
router -> vpn: allowed
```

### Route Preservation

When a full-tunnel VPN is active, the router must still know how to reach the VPN server over cellular WAN.

The implementation must ensure:

- default route can move to VPN.
- explicit route to VPN endpoint remains via `zte_wan`.
- DNS bootstrap for resolving VPN server is handled before route switch.

If `nordvpnlite` handles this internally, prefer its behavior.

## Performance Expectations

### NordVPN Lite / NordLynx Style

Expected:

- Best performance option.
- Lower CPU than OpenVPN.
- Lower battery and heat impact than OpenVPN.
- Still slower than direct cellular.
- Added latency due to VPN server path.

Unknown until tested:

- Maximum sustained throughput on SDX75 CPU under encryption.
- Thermal behavior under long downloads.
- Whether ZTE packet acceleration/offload is bypassed by VPN tunnel routing.

### OpenVPN

Expected:

- Significantly lower throughput.
- Higher CPU load.
- Higher heat and battery impact.
- More likely to become the bottleneck before 5G does.

Recommendation:

- Avoid OpenVPN unless `nordvpnlite` fails completely.

## Risks

### Package Compatibility

Risk:

- Nord's package may expect standard OpenWrt behavior that ZTE's customised firmware changes.

Mitigation:

- Install only with explicit approval.
- Back up configs first.
- Test status-only.
- Test connect/disconnect before modifying router-wide forwarding.

### Kernel Module Compatibility

Risk:

- If `nordvpnlite` depends on kernel WireGuard support and the matching module is absent, installation or runtime may fail.

Current finding:

- `/dev/net/tun` exists.
- No installed `wg` binary was found.
- No obvious WireGuard module was found in read-only checks.

Mitigation:

- Confirm `nordvpnlite` dependency behavior before install.
- Prefer Nord's package if it brings the required userspace/runtime.
- Avoid installing mismatched kernel modules.

### Routing Lockout

Risk:

- A bad route/firewall change could break internet access for clients.

Mitigation:

- Keep SSH/dashboard access on LAN untouched.
- Keep changes reversible.
- Add rollback endpoint/script.
- Apply kill switch only after route verification works.

### DNS Leaks

Risk:

- LAN clients may continue using WAN DNS.

Mitigation:

- Force DNS through router.
- Update DNS forwarding while VPN is active.
- Optionally block outbound port 53/853 from LAN to WAN.

### IPv6 Leaks

Risk:

- Cellular IPv6 remains active while VPN only covers IPv4.

Mitigation:

- Default to blocking IPv6 forwarding while VPN is active.
- Only permit IPv6 through VPN if verified.

### Boot Persistence

Risk:

- VPN starts before cellular WAN is ready.
- ZTE startup scripts overwrite routes/firewall.

Mitigation:

- Use procd service with delayed/retry startup.
- Wait for `zte_wan` connectivity.
- Re-apply route/firewall policy after WAN events if needed.

## Rollback Plan

Before any real install/config test:

Back up:

```sh
uci export network > /data/zte-agent/vpn/backup-network.uci
uci export firewall > /data/zte-agent/vpn/backup-firewall.uci
uci export dhcp > /data/zte-agent/vpn/backup-dhcp.uci
uci export pbr > /data/zte-agent/vpn/backup-pbr.uci
```

Rollback should:

1. Stop VPN client.
2. Remove temporary routes/rules.
3. Restore DNS behavior.
4. Restore firewall forwarding.
5. Restart network/firewall services if needed.
6. Confirm default route through `zte_wan`.

Add an emergency shell script:

```text
/data/zte-agent/vpn/disable-vpn.sh
```

The script should not require the dashboard to be working.

## Implementation Steps

### Step 1: Document and Probe

- Keep this `vpn.md` as the design reference.
- Manually confirm `nordvpnlite` commands on device after explicit approval.
- Capture command output formats.
- Decide if package installation will be manual or dashboard-driven.

### Step 2: Add Agent VPN Module

Files:

- `agent/src/vpn.rs`
- `agent/src/server.rs`
- possibly `agent/src/main.rs` if shared state is needed.

Responsibilities:

- Run safe status commands.
- Read/write VPN settings.
- Store token securely.
- Redact secrets.
- Wrap connect/disconnect operations.
- Manage route/firewall helpers if needed.

### Step 3: Add Frontend API Types

File:

- `web-app/src/api.ts`

Add:

- `VpnStatus`
- `VpnReadiness`
- `VpnSettings`
- `VpnCountry`
- API functions for the endpoints.

### Step 4: Add VPN Page

File:

- `web-app/src/pages/VpnPage.tsx`

Add route/page:

- `web-app/src/App.tsx`
- `web-app/src/components/Sidebar.tsx`

Initial page:

- readiness card.
- status card.
- token setup form.
- country selector.
- connect/disconnect controls.
- logs panel.

### Step 5: Package/Install Strategy

Option A: Manual install first.

- Safer.
- Less code.
- Lets us validate compatibility before automating.

Option B: Dashboard install.

- More user-friendly.
- Requires download/install handling.
- Must handle package verification and failure modes.

Recommendation:

- Start with manual install instructions/status detection.
- Add dashboard-driven install later.

### Step 6: Connect/Disconnect Prototype

- Implement connect/disconnect using confirmed `nordvpnlite` commands.
- Poll status every 2 seconds while connecting.
- Timeout cleanly.
- Store last error.
- Do not touch kill switch yet.

### Step 7: Whole-Router Route Enforcement

- Confirm whether `nordvpnlite` routes LAN traffic automatically.
- If it only protects router-originated traffic, add firewall/route integration.
- Verify from a LAN client, not only from the router itself.

### Step 8: DNS/IPv6 Protection

- Add VPN DNS mode.
- Add IPv6 block mode.
- Verify with external leak tests from a LAN client.

### Step 9: Kill Switch

- Add opt-in kill switch.
- Test disconnect and tunnel failure scenarios.
- Ensure dashboard remains reachable.
- Ensure emergency disable works.

### Step 10: Persistence

- Add auto-connect setting.
- Add boot service or use `nordvpnlite` persistence if provided.
- Wait for WAN before connecting.
- Re-apply policy after reconnect.

## Testing Plan

### Build Tests

Run:

```sh
cargo check
npm run build
git diff --check
```

### Device Tests

Readiness:

- status when package missing.
- status when token missing.
- status when configured.

Connection:

- connect to selected country.
- disconnect.
- reconnect.
- bad token behavior.
- no WAN behavior.
- VPN server unreachable behavior.

Routing:

- router public IP direct.
- router public IP over VPN.
- LAN client public IP direct.
- LAN client public IP over VPN.
- DNS server observed from LAN client.
- IPv6 leak check from LAN client.

Performance:

- speed test direct.
- speed test via VPN.
- CPU load direct.
- CPU load via VPN.
- battery drain/temperature during sustained traffic.

Resilience:

- reboot with auto-connect off.
- reboot with auto-connect on.
- cellular reconnect while VPN active.
- VPN process crash.
- forced disconnect.
- kill switch recovery.

## Success Criteria

MVP success:

- VPN page shows readiness and status accurately.
- Token can be configured without being exposed.
- Router can connect/disconnect NordVPN from the dashboard.
- LAN client traffic routes through VPN while connected.
- Disconnect restores normal internet.
- Dashboard remains reachable at `192.168.0.1`.
- No token or private key is committed or logged.

Production-ready success:

- Kill switch works.
- IPv6 leak protection works.
- DNS leak protection works.
- Auto-connect works after reboot.
- Failure states are recoverable from the dashboard or emergency script.
- Performance impact is documented.

## Open Questions

- Does `nordvpnlite` on this device expose country/server listing commands suitable for UI selection?
- Does it manage default routes itself, or only provide a tunnel?
- Does it support WireGuard/NordLynx entirely in userspace, or does it require kernel WireGuard support?
- What interface name does it create on this firmware?
- Does it tunnel IPv6?
- Does it provide DNS settings, or do we need to enforce DNS separately?
- Should initial version require manual package installation?
- Should VPN be all-or-nothing, or should per-client routing be planned for a later version?

