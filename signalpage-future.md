# Signal Page Future Enhancements

This note captures what the ZTE U60 Pro modem appears to expose today, what the current dashboard already shows, and how the Signal page could be expanded without turning it into a settings page.

The recommended direction is to keep Signal as a read-only diagnostics view, with controls for band/cell locking remaining under Band Lock.

## Current Signal Page

The current Signal page already shows:

- Connection type and provider.
- Signal bars.
- Decoded cell ID display.
- LTE and NR carrier lists.
- PCC/SCC breakdown.
- Band, PCI, EARFCN/NR-ARFCN, frequency, and bandwidth.
- Per-carrier RSRP, RSRQ, SINR, and RSSI.
- Aggregated LTE/NR bandwidth.
- Basic signal-quality reference guidance.

The current data path is:

- Frontend: `web-app/src/pages/SignalPage.tsx`
- API mapping: `web-app/src/api.ts`, `mapSignal`
- Agent endpoint: `GET /api/network/signal`
- Modem source: `ubus call zte_nwinfo_api nwinfo_get_netinfo "{}"`

## Confirmed Readable Modem Sources

### Main Network Info

Source:

```sh
ubus call zte_nwinfo_api nwinfo_get_netinfo "{}"
```

Useful fields seen on the device:

- `network_type`, for example `SA`, `ENDC`, `LTE`.
- `net_select`, for example `Only_5G`.
- `net_select_mode`, for example `auto_select`.
- `domain_stat`, for example `PS_ONLY`.
- `simcard_roam`, for example `Home`.
- `network_provider`, `network_provider_fullname`.
- `rmcc`, `rmnc`.
- `signalbar`.
- `cell_id`, `nr5g_cell_id`.
- `lac_code`, which is effectively TAC/LAC depending on RAT.
- `wan_active_band`.
- `lte_rsrp`, `lte_rsrq`, `lte_rssi`, `lte_snr`.
- `lte_pci`, `wan_active_channel`, `lteca`, `ltecasig`, `lteca_state`.
- `nr5g_rsrp`, `nr5g_rsrq`, `nr5g_rssi`, `nr5g_snr`.
- `nr5g_pci`, `nr5g_action_channel`, `nr5g_action_band`, `nr5g_bandwidth`.
- `nrca`.
- `lte_band`, `lte_band_lock`, `gw_band_lock`.
- `nr5g_sa_band_lock`, `nr5g_nsa_band_lock`, `nr5g_nrdc_band_lock`.
- `lock_lte_cell`, `lock_nr_cell`.

### UCI Radio State

Source:

```sh
uci show zte_nwinfo
```

Useful fields seen on the device:

- `zte_nwinfo.sys_info.operate_mode`
- `zte_nwinfo.sys_info.network_type`
- `zte_nwinfo.sys_info.net_select`
- `zte_nwinfo.sys_info.net_select_mode`
- `zte_nwinfo.sys_info.domain_stat`
- `zte_nwinfo.sys_info.simcard_roam`
- `zte_nwinfo.sys_info.rrc_state`
- `zte_nwinfo.sys_info.emm_state`
- `zte_nwinfo.sys_info.lteca_state`
- `zte_nwinfo.sys_info.nrca_num`
- `zte_nwinfo.sys_info.nrca_ul_state`
- `zte_nwinfo.sys_info.nrca_dl_state`
- `zte_nwinfo.sys_info.odu_nrca`
- `zte_nwinfo.sys_info.odu_lteca`
- `zte_nwinfo.signal_strength.wan_csq`
- `zte_nwinfo.cell_info.nr5g_tac`
- `zte_nwinfo.cell_info.nr5g_pci_hex`
- `zte_nwinfo.plmn_info.rplmn_mccmnc`

These are good candidates for merging into an enhanced signal response because they add context that `nwinfo_get_netinfo` does not always return directly.

### Neighbor Cell Diagnostics

Existing agent endpoints:

- `POST /api/cell/neighbors/scan`
- `GET /api/cell/neighbors/nr`
- `GET /api/cell/neighbors/lte`

Underlying ubus methods:

```sh
ubus call zte_nwinfo_api nwinfo_scan_nbr "{}"
ubus call zte_nwinfo_api nwinfo_get_nr5g_nbr_contents "{}"
ubus call zte_nwinfo_api nwinfo_get_lte_nbr_contents "{}"
```

Current live result:

- NR neighbor contents are empty.
- LTE neighbor contents are empty.

This still looks worth exposing, but only behind an explicit "Scan Neighbors" action. The UI should handle empty results as a normal outcome.

### Signal Quality Detection

Existing agent endpoints:

- `POST /api/cell/signal-detect/start`
- `POST /api/cell/signal-detect/stop`
- `GET /api/cell/signal-detect/progress`
- `GET /api/cell/signal-detect/results`

Underlying ubus methods:

```sh
ubus call zte_nwinfo_api nwinfo_start_detect_signal_quality "{}"
ubus call zte_nwinfo_api nwinfo_end_detect_signal_quality "{}"
ubus call zte_nwinfo_api nwinfo_get_progress_and_quality "{}"
ubus call zte_nwinfo_api nwinfo_get_detect_quality_recorder "{}"
```

Current live result:

- Progress returns empty `progress` and `quality`.
- Results return an empty object unless a detection run has populated records.

This is probably better as an optional diagnostics panel, not part of the primary signal summary.

### External Antenna State

Readable source:

```sh
ubus call zte_nwinfo_api nwinfo_get_ant_ext_state "{}"
```

Current live result:

- Fields exist, but values are currently `null` or empty.

Only show this if useful values are present. Otherwise hide the panel.

### Read-Only AT Commands

The agent permits a limited read-only AT console path through `POST /api/at/send`.

Commands tested:

- `AT+QNWINFO`
- `AT+QENG="servingcell"`
- `AT+QRSRP`
- `AT+QRSRQ`
- `AT+CSQ`
- `AT+CEREG?`
- `AT+CGREG?`
- `AT+COPS?`
- `AT+CGCONTRDP`

On this firmware, the Quectel-style signal commands mostly returned `ERROR`, and `AT+CSQ` returned `99,99`. `AT+CGCONTRDP` is useful for PDP/APN/session data, but that belongs more naturally under WAN/Modem than Signal.

Recommendation: do not build the Signal page around AT commands unless a future device firmware exposes richer read-only output.

## Recommended Signal Page Additions

### 1. Radio State Summary

Add a compact status strip near the top of the Signal page.

Fields:

- RAT/mode: `SA`, `ENDC`, `LTE`, with friendly labels like `5G SA`, `5G NSA`, `4G LTE`.
- Bearer preference: `net_select`, for example `Only_5G`.
- Selection mode: `net_select_mode`, for example `auto_select`.
- Packet domain: `domain_stat`, for example `PS_ONLY`.
- Roaming state: `simcard_roam`, for example `Home`.
- Operate mode: `operate_mode`, for example `ONLINE`.
- RRC state: `rrc_state`, if present.
- EMM state: `emm_state`, if present.

How:

- Extend the agent signal endpoint to merge selected `uci get zte_nwinfo.sys_info.*` fields into `/api/network/signal`.
- Extend `SignalInfo` in `web-app/src/api.ts` with a `radio_state` object or top-level optional fields.
- Render as small key/value chips, not large cards.

### 2. Serving Cell Identity

Add a serving-cell identity panel, separate from the carrier metric cards.

Fields:

- Raw LTE Cell ID or NR NCI.
- Decoded eNodeB/gNodeB.
- Decoded sector.
- TAC/LAC.
- MCC.
- MNC.
- PLMN, for example `50503`.
- PCI decimal.
- PCI hex, if present.
- Serving band and channel.

How:

- Prefer doing decoding in the agent using integer-safe Rust, then return both raw and decoded values.
- Decode `node_id = raw_cell_id >> 8`.
- Decode `sector_id = raw_cell_id & 0xff`.
- Return decimal and hex forms so the frontend does not need bitwise arithmetic.

Important:

- Avoid JavaScript bitwise operators for future-proof NR NCI decoding because JS bitwise operations are 32-bit. The current values fit, but this is fragile.

Proposed response shape:

```json
{
  "serving_cell": {
    "rat": "NR",
    "raw_id": 430473322,
    "node_id": 1681536,
    "sector_id": 106,
    "node_hex": "19A780",
    "sector_hex": "6A",
    "tac": 20284,
    "mcc": "505",
    "mnc": "03",
    "plmn": "50503",
    "pci": 431,
    "pci_hex": "1AF"
  }
}
```

### 3. Carrier Aggregation Clarity

The page should distinguish between:

- Serving carrier.
- Active aggregated carrier.
- Configured but inactive CA candidate.
- Placeholder/invalid carrier data.

Reason:

- The live `nrca` field currently includes n28 and n78 entries with placeholder signal values such as `-140.0`, `-43.0`, `-23.0`, and `-120.0`.
- These should not be treated the same as active serving carriers.
- Aggregated bandwidth should ideally separate "serving/active" from "reported/configured".

How:

- Extend `CarrierComponent` with:

```ts
status?: 'serving' | 'active' | 'configured' | 'inactive' | 'placeholder'
source?: 'primary' | 'lteca' | 'nrca'
```

- Treat PCC as `serving`.
- For LTE SCCs, use `ltecasig` active state where available.
- For NR SCCs, parse `nrca` flags carefully:
  - field 0 appears to be UL configured.
  - field 2 appears to indicate activity/state.
  - current frontend treats `2` as active.
  - live data has `1`, which should not be displayed as fully active.
- Mark obviously bogus signal values as `placeholder`.

Suggested UI:

- Top summary: active serving bandwidth only.
- Details table: include all reported carriers with a clear `Status` column.
- Optional subtext: "Configured CA candidates are reported by the modem but are not necessarily carrying traffic."

### 4. Lock State Read-Only Summary

Add a read-only "Current Locks" section.

Fields:

- LTE band lock parsed list.
- NR SA band lock parsed list.
- NR NSA band lock parsed list.
- LTE cell lock raw value.
- NR cell lock raw value.
- Whether each lock appears active or unset.

How:

- The frontend already parses some lock state in `mapSignal`.
- Expand this into a clearer `lock_state` object.
- Render this on Signal as status only.
- Keep all lock controls in Band Lock.

Proposed response shape:

```json
{
  "lock_state": {
    "lte_bands": [1, 3, 5],
    "nr_sa_bands": [1, 28, 78],
    "nr_nsa_bands": [1, 28, 78],
    "lte_cell": null,
    "nr_cell": null,
    "raw_lte_cell": "0,0",
    "raw_nr_cell": "0,0,0"
  }
}
```

### 5. Supported and Allowed Bands

Add a collapsible "Band Capability" or "Allowed Bands" panel.

Fields:

- LTE supported/allowed bands from `lte_band`.
- LTE current lock mask from `lte_band_lock`.
- NR SA allowed bands from `nr5g_sa_band_lock`.
- NR NSA allowed bands from `nr5g_nsa_band_lock`.
- NRDC bands from `nr5g_nrdc_band_lock`.

How:

- Agent can return raw lists and parsed arrays.
- Frontend renders grouped chips.
- Keep collapsed by default because these lists are long.

### 6. Neighbor Cells

Add a "Neighbor Cells" panel with a manual scan button.

Flow:

1. User clicks "Scan".
2. Frontend calls `POST /api/cell/neighbors/scan`.
3. Poll `GET /api/cell/neighbors/nr` and `GET /api/cell/neighbors/lte`.
4. Parse contents if populated.
5. Show empty-state text if the modem returns no neighbors.

How:

- Add `api.cellNeighborsScan`, `api.cellNeighborsNr`, and `api.cellNeighborsLte`.
- Add parsers only after capturing real non-empty output, because the delimiter format is firmware-specific.
- Avoid auto-scanning on page load. Neighbor scans can be slower and may affect modem behavior.

### 7. Signal Quality Capture

Add an optional diagnostics panel for the built-in signal quality detector.

Fields/actions:

- Start detection.
- Stop detection.
- Progress.
- Latest quality/result records.

How:

- Add frontend API methods for existing endpoints.
- Hide or de-emphasize the panel when progress/result fields are empty.
- Keep this below the live signal cards.

### 8. Signal Trend Snapshot

There is already a signal logger under Advanced.

Potential Signal page enhancement:

- Show a small "Recent signal trend" widget if a signal log is active.
- Include last few RSRP/SINR samples, current capture state, and a link/button to download the CSV.

How:

- Reuse existing logger endpoints:
  - `GET /api/logger/signal/status`
  - `GET /api/logger/signal/download`
- If adding charting, keep it very small and avoid pulling in a heavy dependency unless already present.

### 9. External Antenna Status

Add a conditional panel only when values are meaningful.

Fields:

- Internal antenna RSRP values.
- External antenna RSRP values.
- Current antenna state.
- Internal/external flag.

How:

- Add agent endpoint `GET /api/cell/antenna` or fold this into enhanced signal diagnostics.
- Source from `nwinfo_get_ant_ext_state`.
- If all values are `null`, empty, or zero, return `available: false` and do not show the panel.

### 10. Raw Diagnostics Drawer

Add a collapsible raw diagnostics drawer for troubleshooting.

Include:

- Raw `nwinfo_get_netinfo`.
- Selected raw UCI radio fields.
- Raw neighbor contents.
- Raw antenna state.

How:

- Either expose a dedicated `GET /api/network/signal/raw` endpoint or include `raw` only when a query param is supplied.
- Keep hidden by default.
- Redact identifiers if needed before export. MCC/MNC and cell IDs are fine for local diagnostics, but SIM identifiers such as IMSI/ICCID should not be included here.

## Suggested Agent API Changes

### Enhanced Signal Endpoint

Keep:

```http
GET /api/network/signal
```

Enhance it server-side by combining:

- `zte_nwinfo_api.nwinfo_get_netinfo`
- selected `uci get zte_nwinfo.sys_info.*`
- selected `uci get zte_nwinfo.cell_info.*`
- selected `uci get zte_nwinfo.plmn_info.*`
- selected `uci get zte_nwinfo.signal_strength.*`

Return a richer normalized payload while preserving existing raw fields so the current frontend remains compatible.

### New Read-Only Endpoints

Recommended:

```http
GET /api/cell/antenna
GET /api/cell/locks
GET /api/cell/bands
GET /api/network/signal/raw
```

Already exists and can be surfaced:

```http
POST /api/cell/neighbors/scan
GET /api/cell/neighbors/nr
GET /api/cell/neighbors/lte
POST /api/cell/signal-detect/start
POST /api/cell/signal-detect/stop
GET /api/cell/signal-detect/progress
GET /api/cell/signal-detect/results
```

## Suggested Frontend Model Changes

Extend `SignalInfo` with normalized groups:

```ts
interface SignalInfo {
  type?: string
  carrier?: string
  signal_bars?: number
  lte_carriers: CarrierComponent[]
  nr_carriers: CarrierComponent[]
  radio_state?: RadioState
  serving_cell?: ServingCell
  lock_state?: LockState
  band_capability?: BandCapability
  antenna?: AntennaState
}
```

Potential new types:

```ts
interface RadioState {
  mode?: string
  friendly_mode?: string
  net_select?: string
  net_select_mode?: string
  domain_stat?: string
  roaming?: string
  operate_mode?: string
  rrc_state?: string
  emm_state?: string
}

interface ServingCell {
  rat?: 'LTE' | 'NR'
  raw_id?: number
  node_id?: number
  sector_id?: number
  node_hex?: string
  sector_hex?: string
  tac?: number
  lac?: number
  mcc?: string
  mnc?: string
  plmn?: string
  pci?: number
  pci_hex?: string
}

interface LockState {
  lte_bands?: number[]
  nr_sa_bands?: number[]
  nr_nsa_bands?: number[]
  raw_lte_band_lock?: string
  raw_nr_sa_band_lock?: string
  raw_nr_nsa_band_lock?: string
  lte_cell?: string | null
  nr_cell?: string | null
}

interface BandCapability {
  lte_allowed?: number[]
  nr_sa_allowed?: number[]
  nr_nsa_allowed?: number[]
  nr_nrdc_allowed?: number[]
}

interface AntennaState {
  available: boolean
  internal_rsrp?: number[]
  external_rsrp?: number[]
  state?: string
}
```

## Suggested UI Layout

Keep the first viewport focused:

1. Header: mode, provider, bars.
2. At-a-glance cards:
   - Serving signal strength.
   - Active mode.
   - Active/serving bandwidth.
   - Serving cell identity.
3. Carrier tables/cards:
   - NR carriers.
   - LTE carriers.
   - Include `Status` and avoid implying inactive CA is live capacity.
4. Diagnostics:
   - Radio state.
   - Lock state.
   - Allowed bands.
   - Neighbor scan.
   - Signal quality capture.
   - Raw details drawer.

## Implementation Order

Recommended sequence:

1. Normalize enhanced signal data in the agent.
2. Update `SignalInfo` and `mapSignal` without changing UI behavior.
3. Add Serving Cell Identity and Radio State panels.
4. Fix carrier status classification and active/configured bandwidth display.
5. Add read-only Lock State and Allowed Bands panels.
6. Add Neighbor Scan as an explicit user action.
7. Add optional Signal Quality Capture or trend widgets.
8. Add Raw Diagnostics drawer.

## Verification Plan

After implementation:

- Run `cargo check` in `agent`.
- Run `npm run build` in `web-app`.
- Verify `/api/network/signal` remains backwards compatible.
- Verify Signal page in:
  - SA mode.
  - ENDC/NSA mode, if available.
  - LTE-only mode, if available.
- Confirm placeholder CA values are not counted as active serving bandwidth.
- Confirm neighbor scan empty results render cleanly.
- Confirm no SIM secrets such as IMSI/ICCID are exposed on Signal or in raw export.

## Open Questions

- Should Signal show only radio-layer data, or also WAN/session context such as APN and PDP address?
- Should inactive CA candidates count toward any "reported bandwidth" total, or should the dashboard only count serving/active bandwidth?
- Should neighbor scanning be available on mobile, or hidden behind Advanced because it may be slow?
- Should raw diagnostics be exportable, and if so, what identifiers should be redacted?

