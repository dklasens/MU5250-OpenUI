// Agent API bindings + response mappers.
//
// The mappers encode hard-won knowledge of the firmware's response shapes
// (netinfo carrier strings, CA formats, band-lock bitmasks) — treat with care.
// Validation lives at this boundary (see ./validate): a malformed field becomes
// unknown/unavailable, never a fabricated 0 or a crash, and unknown extra
// fields are ignored.

import { get, post, put, readCsv, req } from './client'
import { normaliseBands, parseLteBandLock, parseNrBandLock } from './bands'
import { mapDataUsage } from './usage'
import { boolLike, finiteNumber, intInRange, isObj, nonEmptyStr, nonNegative, nonNegativeInt, obj, str, strList, arr } from './validate'
import { widthMhz } from './wifiWidth'
import type {
  ApnModeState,
  ApnProfile,
  AtSendResult,
  BatteryBspInfo,
  BatteryDetail,
  BatteryInfo,
  CarrierComponent,
  ChargeControlState,
  Client,
  CpuInfo,
  DeviceInfo,
  DnsConfig,
  HomeData,
  KillBloatResult,
  LanConfig,
  LoggerStatus,
  MemInfo,
  ModemCapabilities,
  PrimaryCarrier,
  ProcessListResult,
  SignalInfo,
  SimInfo,
  SmsMessage,
  SmsCapabilities,
  SpeedInfo,
  ThermalAll,
  ThermalInfo,
  TtlStatus,
  UsbMode,
  UsbModeCapability,
  UsbModeResult,
  UsbStatus,
  Wan6Info,
  WanInfo,
  WifiAll,
} from '../types'

export { mapDataUsage }

// ── EARFCN / NR-ARFCN to frequency ──────────────────────────────────────────

const LTE_BANDS: Record<number, { fdl_low: number; noffs_dl: number }> = {
  1: { fdl_low: 2110, noffs_dl: 0 },
  2: { fdl_low: 1930, noffs_dl: 600 },
  3: { fdl_low: 1805, noffs_dl: 1200 },
  4: { fdl_low: 2110, noffs_dl: 1950 },
  5: { fdl_low: 869, noffs_dl: 2400 },
  7: { fdl_low: 2620, noffs_dl: 2750 },
  8: { fdl_low: 925, noffs_dl: 3450 },
  12: { fdl_low: 729, noffs_dl: 5010 },
  13: { fdl_low: 746, noffs_dl: 5180 },
  14: { fdl_low: 758, noffs_dl: 5280 },
  17: { fdl_low: 734, noffs_dl: 5730 },
  18: { fdl_low: 860, noffs_dl: 5850 },
  19: { fdl_low: 875, noffs_dl: 6000 },
  20: { fdl_low: 791, noffs_dl: 6150 },
  25: { fdl_low: 1930, noffs_dl: 8040 },
  26: { fdl_low: 859, noffs_dl: 8690 },
  28: { fdl_low: 758, noffs_dl: 9210 },
  29: { fdl_low: 717, noffs_dl: 9660 },
  30: { fdl_low: 2350, noffs_dl: 9770 },
  32: { fdl_low: 1452, noffs_dl: 9920 },
  34: { fdl_low: 2010, noffs_dl: 36200 },
  38: { fdl_low: 2570, noffs_dl: 37750 },
  39: { fdl_low: 1880, noffs_dl: 38250 },
  40: { fdl_low: 2300, noffs_dl: 38650 },
  41: { fdl_low: 2496, noffs_dl: 39650 },
  42: { fdl_low: 3400, noffs_dl: 41590 },
  43: { fdl_low: 3600, noffs_dl: 43590 },
  48: { fdl_low: 3550, noffs_dl: 55240 },
  66: { fdl_low: 2110, noffs_dl: 66436 },
  71: { fdl_low: 617, noffs_dl: 68586 },
}

function earfcnToFreq(earfcn: number, bandNum: number): number | undefined {
  const band = LTE_BANDS[bandNum]
  if (!band) return undefined
  return band.fdl_low + 0.1 * (earfcn - band.noffs_dl)
}

function nrarfcnToFreq(arfcn: number): number | undefined {
  if (arfcn <= 599999) return 0.005 * arfcn
  if (arfcn <= 2016666) return 3000 + 0.015 * (arfcn - 600000)
  if (arfcn <= 3279165) return 24250 + 0.06 * (arfcn - 2016667)
  return undefined
}

// ── Parse helpers ─────────────────────────────────────────────────────────────

/** Finite number from a number or strictly numeric string; '', '--', 'N/A', junk -> undefined. */
const parseNum = finiteNumber

function parseInteger(v: unknown): number | undefined {
  const n = finiteNumber(v)
  return n !== undefined && Number.isInteger(n) ? n : undefined
}

function parseCellId(id?: unknown): number | undefined {
  const num = typeof id === 'string' ? Number(id.trim()) : typeof id === 'number' ? id : NaN
  return Number.isSafeInteger(num) && num > 0 ? num : undefined
}

/**
 * LTE ECI (28 bits) splits at a fixed point: eNB ID (20 bits) | cell (8 bits).
 * Plain arithmetic, not bit operators, which truncate to 32 bits.
 */
function formatEci(id?: unknown): string | undefined {
  const eci = parseCellId(id)
  if (eci == null) return undefined
  const hex = (n: number) => n.toString(16).toUpperCase()
  return `${hex(Math.floor(eci / 256))}|${hex(eci % 256)}`
}

/**
 * NR NCI is 36 bits and the gNB/cell split is operator-configured (22–32 bit
 * gNB ID), so no split can be derived from the NCI alone. Show it whole.
 */
function formatNci(id?: unknown): string | undefined {
  const nci = parseCellId(id)
  return nci == null ? undefined : nci.toString(16).toUpperCase()
}

/** Valid physical cell IDs: LTE 0–503, NR 0–1007. 0 is valid; absent/garbage is undefined. */
const lteSignalPci = (v: unknown) => intInRange(v, 0, 503)
const nrSignalPci = (v: unknown) => intInRange(v, 0, 1007)

/** RSRP is a negative dBm value; 0 (or positive) is the firmware's "no measurement" placeholder. */
function validRsrp(v: unknown): number | undefined {
  const n = parseNum(v)
  return n !== undefined && n < 0 ? n : undefined
}

/** RSSI likewise reports 0 when unmeasured. */
function validRssi(v: unknown): number | undefined {
  const n = parseNum(v)
  return n !== undefined && n < 0 ? n : undefined
}

/** Parse lteca entries, returning SCCs only (excluding PCC by PCI+EARFCN match). */
function parseLteCa(ltecaStr: string, pccPci?: number, pccEarfcn?: number) {
  const sccs: { pci?: number; band: string; earfcn: number; bw: string }[] = []
  let pccFound = false
  for (const seg of ltecaStr.split(';')) {
    if (!seg.trim()) continue
    const p = seg.split(',')
    if (p.length < 5) continue
    const entryPci = parseInteger(p[0])
    const entryEarfcn = parseInteger(p[3])
    if (
      !pccFound && pccPci !== undefined && pccEarfcn !== undefined &&
      entryPci === pccPci && entryEarfcn === pccEarfcn
    ) {
      pccFound = true
      continue
    }
    sccs.push({ pci: entryPci === undefined ? undefined : lteSignalPci(entryPci), band: p[1].trim(), earfcn: entryEarfcn ?? 0, bw: p[4].trim() })
  }
  return sccs
}

/** Extract PCC bandwidth from lteca string. */
function extractPccBw(ltecaStr: string | undefined, pccPci?: number, pccEarfcn?: number): string | undefined {
  if (!ltecaStr || pccPci === undefined || pccEarfcn === undefined) return undefined
  for (const seg of ltecaStr.split(';')) {
    if (!seg.trim()) continue
    const p = seg.split(',')
    if (p.length < 5) continue
    if (parseInteger(p[0]) === pccPci && parseInteger(p[3]) === pccEarfcn) return p[4].trim()
  }
  return undefined
}

/** Parse ltecasig / nrcasig: "rsrp,rsrq,sinr,rssi,ul_configured,active;..." */
function parseCaSig(sigStr: string) {
  const sigs: { rsrp?: number; rsrq?: number; sinr?: number; rssi?: number; ul_configured?: boolean; active?: boolean }[] = []
  for (const seg of sigStr.split(';')) {
    if (!seg.trim()) continue
    const p = seg.split(',')
    if (p.length < 4) continue
    sigs.push({
      rsrp: parseNum(p[0]),
      rsrq: parseNum(p[1]),
      sinr: parseNum(p[2]),
      rssi: parseNum(p[3]),
      ul_configured: p.length > 4 ? p[4].trim() === '1' : undefined,
      active: p.length > 5 ? p[5].trim() === '2' : undefined,
    })
  }
  return sigs
}

// ── Mappers ───────────────────────────────────────────────────────────────────

/** Lowest reportable NR RSRP; the modem reports it for unmeasured carriers. */
const NR_RSRP_FLOOR = -140

type NetworkMode = 'sa' | 'nsa' | 'lte' | 'unknown'

/** Classify the firmware `network_type` string. Bare "5G"/"NR" is ambiguous -> unknown. */
export function classifyNetworkType(type: unknown): NetworkMode {
  const t = (typeof type === 'string' ? type : '').trim().toUpperCase()
  if (!t) return 'unknown'
  if (t.includes('ENDC') || t.includes('EN-DC') || t.includes('NSA')) return 'nsa'
  if (/\bSA\b/.test(t)) return 'sa'
  if (t === '4G' || t.includes('LTE')) return 'lte'
  return 'unknown'
}

/**
 * Choose the serving carrier from *validated* carriers by network mode.
 * SA -> NR PCC. LTE -> LTE PCC. NSA/ENDC -> the LTE anchor PCC (UEs camp on the
 * LTE anchor; the NR leg is secondary-cell-group and stays in nr_carriers).
 * Unrecognised type -> only when exactly one RAT has a valid PCC.
 * No fallback across RATs in a known mode: SA never picks LTE, even though the
 * firmware leaves LTE fields populated there.
 */
function pickPrimary(mode: NetworkMode, lte: CarrierComponent[], nr: CarrierComponent[]): PrimaryCarrier | undefined {
  const lteP = lte.find((c) => c.label === 'PCC')
  const nrP = nr.find((c) => c.label === 'PCC')
  const pick = (rat: 'lte' | 'nr', carrier: CarrierComponent | undefined): PrimaryCarrier | undefined =>
    carrier ? { rat, carrier } : undefined
  switch (mode) {
    case 'sa':
      return pick('nr', nrP)
    case 'lte':
    case 'nsa':
      return pick('lte', lteP)
    default:
      if (lteP && !nrP) return pick('lte', lteP)
      if (nrP && !lteP) return pick('nr', nrP)
      return undefined
  }
}

export function mapSignal(d: Record<string, unknown>): SignalInfo {
  const pccPci = lteSignalPci(d.lte_pci)
  const pccEarfcn = nonNegativeInt(d.wan_active_channel)
  const pccBandStr = str(d.wan_active_band) ?? ''
  const pccBandNum = parseInt(pccBandStr.replace(/\D/g, '')) || 0
  const ltecaStr = str(d.lteca) ?? ''
  const pccBw = extractPccBw(ltecaStr, pccPci, pccEarfcn)

  // Build LTE PCC — skip ghost carriers (e.g. in 5G SA mode, where the active
  // band is an NR band like "n78" and the LTE fields hold stale values).
  const lteCarriers: CarrierComponent[] = []
  const lteHasValidData =
    pccBandStr !== '' && pccBandStr !== '0' && pccBandStr !== 'B' && pccBandStr !== 'B0' &&
    !/^n/i.test(pccBandStr) && pccEarfcn !== undefined && pccEarfcn > 0

  if (lteHasValidData && pccEarfcn !== undefined) {
    lteCarriers.push({
      label: 'PCC',
      band: pccBandNum ? `B${pccBandNum}` : pccBandStr,
      pci: pccPci,
      earfcn: pccEarfcn,
      bandwidth: pccBw ? `${pccBw} MHz` : '—',
      freq: pccBandNum ? earfcnToFreq(pccEarfcn, pccBandNum) : undefined,
      rsrp: validRsrp(d.lte_rsrp),
      rsrq: parseNum(d.lte_rsrq),
      sinr: parseNum(d.lte_snr),
      rssi: validRssi(d.lte_rssi),
      ul_configured: true,
      active: true,
    })

    const ltecasigStr = str(d.ltecasig) ?? ''
    const ltecaEntries = parseLteCa(ltecaStr, pccPci, pccEarfcn)
    const ltecaSigs = parseCaSig(ltecasigStr)

    for (let i = 0; i < ltecaEntries.length; i++) {
      const e = ltecaEntries[i]
      const sig = i < ltecaSigs.length ? ltecaSigs[i] : undefined
      const bandNum = parseInt(e.band) || 0
      lteCarriers.push({
        label: `SCC${i}`,
        band: `B${e.band}`,
        pci: e.pci,
        earfcn: e.earfcn,
        bandwidth: `${e.bw} MHz`,
        freq: bandNum ? earfcnToFreq(e.earfcn, bandNum) : undefined,
        rsrp: sig?.rsrp === 0 ? undefined : sig?.rsrp,
        rsrq: sig?.rsrq === 0 ? undefined : sig?.rsrq,
        sinr: sig?.sinr,
        rssi: sig?.rssi === 0 ? undefined : sig?.rssi,
        ul_configured: sig?.ul_configured,
        active: sig?.active,
      })
    }
  }

  // Build NR primary — skip ghost carriers (no valid band or ARFCN, e.g. 4G-only)
  const nrCarriers: CarrierComponent[] = []
  const nrBand = str(d.nr5g_action_band) ?? ''
  const nrArfcn = nonNegativeInt(d.nr5g_action_channel) ?? 0
  const nrHasValidData = nrBand !== '' && nrBand !== '0' && nrBand !== 'n' && nrBand !== 'n0' && nrArfcn > 0
  if (nrHasValidData) {
    const nrBw = parseNum(d.nr5g_bandwidth)
    const nrPccPci = nrSignalPci(d.nr5g_pci)
    nrCarriers.push({
      label: 'PCC',
      band: nrBand.startsWith('n') ? nrBand : `n${nrBand}`,
      pci: nrPccPci,
      earfcn: nrArfcn,
      bandwidth: nrBw !== undefined ? `${nrBw} MHz` : '—',
      freq: nrarfcnToFreq(nrArfcn),
      rsrp: validRsrp(d.nr5g_rsrp),
      rsrq: parseNum(d.nr5g_rsrq),
      sinr: parseNum(d.nr5g_snr),
      rssi: validRssi(d.nr5g_rssi),
      ul_configured: true,
      active: true,
    })

    // nrca SCCs — format: index,pci,?,band,arfcn,bw,...,rsrp,rsrq,sinr,rssi
    const nrcaStr = str(d.nrca) ?? ''
    for (const seg of nrcaStr.split(';')) {
      if (!seg.trim()) continue
      const parts = seg.split(',')
      if (parts.length < 6) continue
      const sPci = nrSignalPci(parts[1])
      const sArfcn = parseInteger(parts[4]) ?? 0
      if (sPci !== undefined && nrPccPci !== undefined && sPci === nrPccPci && sArfcn === nrArfcn) continue
      const sBand = parseInteger(parts[3]) ?? 0
      const sBw = parts[5].trim()
      // A configured-but-unmeasured SCC reports the 3GPP reporting floors
      // (RSRP -140, RSRQ -43, SINR -23): no measurement, not a real reading.
      const sRsrp = parts.length >= 8 ? parseNum(parts[7]) : undefined
      const measured = sRsrp != null && sRsrp > NR_RSRP_FLOOR
      nrCarriers.push({
        label: `SCC${nrCarriers.length - 1}`,
        band: `n${sBand}`,
        pci: sPci,
        earfcn: sArfcn,
        bandwidth: `${sBw} MHz`,
        freq: sArfcn ? nrarfcnToFreq(sArfcn) : undefined,
        rsrp: measured ? sRsrp : undefined,
        rsrq: measured && parts.length >= 9 ? parseNum(parts[8]) : undefined,
        sinr: measured && parts.length >= 10 ? parseNum(parts[9]) : undefined,
        rssi: measured && parts.length >= 11 ? parseNum(parts[10]) : undefined,
        ul_configured: parts.length > 0 ? parts[0].trim() === '1' : undefined,
        active: parts.length > 2 ? parts[2].trim() === '2' : undefined,
      })
    }
  }

  const mode = classifyNetworkType(d.network_type)
  // NSA/ENDC camps on the LTE anchor, so its LTE cell is the serving cell.
  const is4g = mode === 'lte' || mode === 'nsa'
  const cellId = is4g && parseCellId(d.cell_id) != null
    ? formatEci(d.cell_id)
    : formatNci(d.nr5g_cell_id) ?? formatEci(d.cell_id)

  // SA and NSA locks are separate observations; never substitute one for the other.
  const rawSa = d.nr5g_sa_band_lock
  const rawNsa = d.nr5g_nsa_band_lock

  return {
    type: str(d.network_type),
    carrier: nonEmptyStr(d.network_provider_fullname) ?? nonEmptyStr(d.network_provider),
    signal_bars: intInRange(d.signalbar, 0, 5),
    cell_id: cellId,
    lte_carriers: lteCarriers,
    nr_carriers: nrCarriers,
    primary: pickPrimary(mode, lteCarriers, nrCarriers),
    net_select: str(d.net_select),
    lte_band_lock_state: parseLteBandLock(d.lte_band_lock),
    nr_sa_band_lock_state: parseNrBandLock(rawSa),
    nr_nsa_band_lock_state: parseNrBandLock(rawNsa),
    raw_lte_band_lock: String(d.lte_band_lock ?? ''),
    raw_nr_band_lock: `SA=${String(rawSa ?? '')} NSA=${String(rawNsa ?? '')}`,
    raw_nr_sa_band_lock: str(rawSa),
    raw_nr_nsa_band_lock: str(rawNsa),
    band: pccBandStr,
  }
}

function mapBattery(d: Record<string, unknown>): BatteryInfo {
  const status = typeof d.status === 'string' ? d.status : undefined
  return {
    percent: d.capacity as number,
    status,
    charging: status === 'Charging',
    // USB supply present and not feeding a powerbank load. Covers "Full" and
    // "Not charging" (e.g. the charge limit paused charging while plugged in).
    plugged: status === 'Charging' || (d.external_power === true && status !== 'Discharging'),
    voltage_mv: d.voltage_uv ? Math.round((d.voltage_uv as number) / 1000) : undefined,
    temperature_c: d.temperature ? (d.temperature as number) / 10 : undefined,
    current_ma: d.current_ua ? Math.round((d.current_ua as number) / 1000) : undefined,
  }
}

/**
 * Map the agent's `SpeedSnapshot` (agent/src/system.rs) — the modem's own WAN
 * counters (including IPA-offloaded traffic), averaged between agent samples.
 * Rates are bytes/sec.
 */
function mapSpeed(d: Record<string, unknown>): SpeedInfo {
  return {
    rx_bps: (d.rx_speed as number) || 0,
    tx_bps: (d.tx_speed as number) || 0,
    max_rx_bps: (d.max_rx_speed as number) || 0,
    max_tx_bps: (d.max_tx_speed as number) || 0,
  }
}

function mapDevice(d: Record<string, unknown>): DeviceInfo {
  const kernel = d.kernel as string | undefined
  return {
    model: 'ZTE U60 Pro',
    firmware: typeof d.firmware === 'string' ? d.firmware : undefined,
    hardware: typeof d.hardware === 'string' ? d.hardware : undefined,
    kernel: kernel?.match(/Linux version (\S+)/)?.[1],
    uptime_secs: d.uptime_secs as number | undefined,
    load_avg: d.load_avg as number[] | undefined,
  }
}

function mapWan(d: Record<string, unknown>): WanInfo {
  const addrs = d['ipv4-address'] as Array<{ address: string }> | undefined
  const v6addrs = d['ipv6-address'] as Array<{ address: string }> | undefined
  const routes = d.route as Array<{ nexthop: string }> | undefined
  return {
    connected: d.up as boolean,
    ipv4: addrs?.[0]?.address,
    ipv6: v6addrs?.[0]?.address,
    gateway: routes?.[0]?.nexthop,
    dns: d['dns-server'] as string[] | undefined,
    apn: d.proto as string | undefined,
  }
}

function mapWan6(d: Record<string, unknown>): Wan6Info {
  const v6addrs = d['ipv6-address'] as Array<{ address: string; mask?: number }> | undefined
  const v6prefix = d['ipv6-prefix'] as Array<{ address: string; mask?: number }> | undefined
  return {
    connected: d.up as boolean,
    ipv6: v6addrs?.[0]?.address,
    prefix: v6prefix?.[0] ? `${v6prefix[0].address}/${v6prefix[0].mask}` : undefined,
    dns: d['dns-server'] as string[] | undefined,
  }
}

function mapClients(d: Record<string, unknown>): Client[] {
  const clients = d.clients as Array<Record<string, unknown>> | undefined
  if (clients) {
    return clients.map((c) => ({
      mac: c.mac as string,
      ip: c.ip as string | undefined,
      hostname: c.hostname as string | undefined,
      medium: c.medium as Client['medium'],
      medium_detail: c.medium_detail as Client['medium_detail'],
      interface: c.interface as string | undefined,
      wifi_band: c.wifi_band as string | undefined,
      signal_dbm: c.signal_dbm as number | undefined,
      tx_bitrate_mbps: c.tx_bitrate_mbps as number | undefined,
      rx_bitrate_mbps: c.rx_bitrate_mbps as number | undefined,
      expected_throughput_mbps: c.expected_throughput_mbps as number | undefined,
      connected_secs: c.connected_secs as number | undefined,
      wired_link_mbps: c.wired_link_mbps as number | undefined,
    }))
  }
  const leases = d.dhcp_leases as Array<{ macaddr: string; ipaddr: string; hostname: string }> | undefined
  if (!leases) return []
  return leases.map((l) => ({ mac: l.macaddr, ip: l.ipaddr, hostname: l.hostname }))
}

function mapCpu(d: Record<string, unknown>): CpuInfo {
  return { overall: d.overall as number, cores: d.cores as number[] }
}

function mapMemory(d: Record<string, unknown>): MemInfo {
  return {
    total_kb: d.total_kb as number,
    used_kb: d.used_kb as number,
    free_kb: d.free_kb as number,
    usage_pct: d.usage_pct as number,
  }
}

export function mapWifi(d: Record<string, unknown>): WifiAll {
  const parseBoolLike = (value: unknown, fallback: boolean): boolean => boolLike(value) ?? fallback
  const parseChannelNumber = (value: unknown): number | undefined => {
    const raw = String(value ?? '')
    const n = parseInt(raw, 10)
    return Number.isFinite(n) ? n : undefined
  }
  const masterSupported = parseBoolLike(d.wifi_onoff_supported, Object.prototype.hasOwnProperty.call(d, 'wifi_onoff'))
  const masterEnabled = parseBoolLike(d.wifi_onoff, true)
  const wifi6Supported = parseBoolLike(d.wifi6_supported, Object.prototype.hasOwnProperty.call(d, 'wifi6_switch'))
  const wifi6Enabled = wifi6Supported ? parseBoolLike(d.wifi6_switch, false) : undefined
  const configuredChannel2g = String(d.channel_2g ?? 'auto') || 'auto'
  const configuredChannel5g = String(d.channel_5g ?? 'auto') || 'auto'
  const actualChannel2g = parseChannelNumber(d.actual_channel_2g)
  const actualChannel5g = parseChannelNumber(d.actual_channel_5g)
  const actualBw2g = str(d.actual_bw_2g)
  const actualBw5g = str(d.actual_bw_5g)
  return {
    band_2g: {
      ssid: d.ssid_2g as string | undefined,
      enabled: d.radio2_disabled !== '1',
      channel: actualChannel2g,
      bandwidth: actualBw2g,
      configuredChannel: configuredChannel2g === '0' ? 'auto' : configuredChannel2g,
      configuredBandwidth: str(d.htmode_2g),
      configuredWidthMhz: widthMhz(d.htmode_2g),
      actualWidthMhz: widthMhz(actualBw2g),
      txpowerPercent: intInRange(d.txpower_2g, 1, 100),
      bandwidthOptions: d.bandwidth_options_2g as string[] | undefined,
      supportedStandards: d.supported_standards_2g as string | undefined,
      actualChannel: actualChannel2g,
      actualBandwidth: actualBw2g,
      password: (d.key_2g as string) || (d.has_key_2g ? '••••••••' : undefined),
      security: d.encryption_2g as string | undefined,
      hidden: d.hidden_2g === '1',
      clients: d.clients_2g as number | undefined,
    },
    band_5g: {
      ssid: d.ssid_5g as string | undefined,
      enabled: d.radio5_disabled !== '1',
      channel: actualChannel5g,
      bandwidth: actualBw5g,
      configuredChannel: configuredChannel5g === '0' ? 'auto' : configuredChannel5g,
      configuredBandwidth: str(d.htmode_5g),
      configuredWidthMhz: widthMhz(d.htmode_5g),
      actualWidthMhz: widthMhz(actualBw5g),
      txpowerPercent: intInRange(d.txpower_5g, 1, 100),
      bandwidthOptions: d.bandwidth_options_5g as string[] | undefined,
      supportedStandards: d.supported_standards_5g as string | undefined,
      actualChannel: actualChannel5g,
      actualBandwidth: actualBw5g,
      password: (d.key_5g as string) || (d.has_key_5g ? '••••••••' : undefined),
      security: d.encryption_5g as string | undefined,
      hidden: d.hidden_5g === '1',
      clients: d.clients_5g as number | undefined,
    },
    guest_ssid: d.guest_ssid as string | undefined,
    master_supported: masterSupported,
    master_enabled: masterEnabled,
    wifi6_supported: wifi6Supported,
    wifi6_enabled: wifi6Enabled,
    wifi7_supported: parseBoolLike(d.wifi7_supported, false),
  }
}

function mapDns(d: Record<string, unknown>): DnsConfig {
  return {
    primary: (d.prefer_dns_manual as string) || '',
    secondary: (d.standby_dns_manual as string) || '',
    ipv6_primary: d.ipv6_wan_prefer_dns_manual as string | undefined,
    ipv6_secondary: d.ipv6_wan_standby_dns_manual as string | undefined,
  }
}

function mapLan(d: Record<string, unknown>): LanConfig {
  return {
    ipaddr: (d.ipaddr as string) || '',
    netmask: (d.netmask as string) || '',
    dhcp_enabled: d.dhcp_enabled === true,
    dhcp_start: (d.dhcp_start as string) || '',
    dhcp_end: (d.dhcp_end as string) || '',
    lease_seconds: Number(d.lease_seconds) || 0,
  }
}

function mapSim(d: Record<string, unknown>): SimInfo {
  return {
    iccid: d.sim_iccid as string | undefined,
    imsi: d.sim_imsi as string | undefined,
    state: d.sim_states as string | undefined,
    mcc: d.mdm_mcc as string | undefined,
    mnc: d.mdm_mnc as string | undefined,
  }
}

/**
 * Map the raw `zte_libwms_get_sms_data` response. The firmware returns
 * `{messages: [{id, number, content, date, tag, mem_store}, ...]}`; UCS-2
 * hex-encoded content/numbers are decoded.
 *
 * Entries without a valid id (never defaulted to 0), without a recognised tag
 * (0–4), that are not objects, or that repeat an earlier id are dropped and
 * counted so callers can flag them. A payload that is neither a list nor an
 * object with a list/absent `messages` throws instead of reading as "empty".
 */
export function mapSmsListResult(d: unknown): { messages: SmsMessage[]; dropped: number } {
  let raw: unknown[]
  if (Array.isArray(d)) raw = d
  else if (isObj(d)) {
    if (d.messages == null) raw = []
    else if (Array.isArray(d.messages)) raw = d.messages
    else throw new Error('Malformed SMS list response')
  } else throw new Error('Malformed SMS list response')

  const messages: SmsMessage[] = []
  const seen = new Set<number>()
  let dropped = 0
  const text = (v: unknown): string =>
    typeof v === 'string' ? v : typeof v === 'number' && Number.isFinite(v) ? String(v) : ''
  for (const m of raw) {
    if (!isObj(m)) { dropped++; continue }
    const id = nonNegativeInt(m.id)
    const tag = intInRange(m.tag, 0, 4)
    if (id === undefined || tag === undefined || seen.has(id)) { dropped++; continue }
    seen.add(id)
    const number = text(m.number)
    const content = text(m.content)
    const date = text(m.date)
    messages.push({
      id,
      number: isUcs2Hex(number) ? decodeUcs2Hex(number) : number,
      content: isUcs2Hex(content) ? decodeUcs2Hex(content) : content,
      date: date !== '' ? date : undefined,
      tag,
      mem_store: nonNegativeInt(m.mem_store),
    })
  }
  return { messages, dropped }
}

export function mapSmsList(d: unknown): SmsMessage[] {
  return mapSmsListResult(d).messages
}

function isUcs2Hex(s: string): boolean {
  return s.length > 0 && s.length % 4 === 0 && /^[0-9A-Fa-f]+$/.test(s)
}

function decodeUcs2Hex(hex: string): string {
  try {
    const units: number[] = []
    for (let i = 0; i < hex.length; i += 4) {
      units.push(parseInt(hex.substring(i, i + 4), 16))
    }
    return String.fromCodePoint(...units)
  } catch {
    return hex
  }
}

function mapThermal(d: Record<string, unknown>): ThermalInfo {
  return { cpu_temp_c: d.cpuss_temp as number | undefined }
}

function mapBatteryBspInfo(d: Record<string, unknown>): BatteryBspInfo {
  return {
    available: d.available === true,
    online: typeof d.online === 'boolean' ? d.online : null,
    low_power: typeof d.low_power === 'boolean' ? d.low_power : null,
    using_hw_fg_chip: typeof d.using_hw_fg_chip === 'boolean' ? d.using_hw_fg_chip : null,
    time_to_full_mins: typeof d.time_to_full_mins === 'number' ? d.time_to_full_mins : null,
    time_to_empty_mins: typeof d.time_to_empty_mins === 'number' ? d.time_to_empty_mins : null,
  }
}

/** Map the /api/dashboard batch response into the Home screen shape. */
function mapHome(d: Record<string, unknown>): HomeData {
  return {
    signal: isObj(d.signal) && Object.keys(d.signal).length > 0 ? mapSignal(d.signal) : null,
    bearer: isObj(d.bearer)
      ? { qci: parseNum(d.bearer.qci) ?? null, ambr_dl_mbps: parseNum(d.bearer.ambr_dl_mbps) ?? null, ambr_ul_mbps: parseNum(d.bearer.ambr_ul_mbps) ?? null }
      : null,
    battery: isObj(d.battery) && Object.keys(d.battery).length > 0 ? mapBattery(d.battery) : null,
    speed: isObj(d.speed) ? mapSpeed(d.speed) : null,
    device: isObj(d.device) ? mapDevice(d.device) : null,
    wan: isObj(d.wan) && Object.keys(d.wan).length > 0 ? mapWan(d.wan) : null,
    wan6: isObj(d.wan6) && Object.keys(d.wan6).length > 0 ? mapWan6(d.wan6) : null,
    cpu: isObj(d.cpu) ? mapCpu(d.cpu) : null,
    memory: isObj(d.memory) ? mapMemory(d.memory) : null,
    usage: isObj(d.data_usage) && !('error' in d.data_usage) ? mapDataUsage(d.data_usage) : null,
    thermal: isObj(d.thermal) ? mapThermal(d.thermal) : null,
    sources: isObj(d.sources) ? d.sources as unknown as HomeData['sources'] : {},
    charge_control_error: typeof d.charge_control_error === 'string' ? d.charge_control_error : null,
  }
}

// ── USB ───────────────────────────────────────────────────────────────────────

const USB_MODE_VALUES: readonly UsbMode[] = ['ecm', 'rndis', 'ncm']

function parseUsbMode(v: unknown): UsbMode | null {
  const t = typeof v === 'string' ? v.trim().toLowerCase() : ''
  return USB_MODE_VALUES.find((m) => m === t) ?? null
}

function mapUsbCapabilities(v: unknown): UsbModeCapability[] | undefined {
  const list = arr(v)
  if (!list) return undefined
  const out: UsbModeCapability[] = []
  for (const entry of list) {
    if (!isObj(entry)) continue
    const mode = parseUsbMode(entry.mode)
    const supported = boolLike(entry.supported)
    // Never invent support: an entry we cannot read is simply absent.
    if (mode === null || supported === undefined || out.some((c) => c.mode === mode)) continue
    out.push({
      mode,
      supported,
      // Missing flag: NCM is treated as experimental (requires confirmation), the safe default.
      experimental: boolLike(entry.experimental) ?? mode === 'ncm',
      function: str(entry.function),
      note: str(entry.note),
    })
  }
  return out
}

/**
 * Map `GET /api/usb/status`. `mode_capabilities` stays authoritative: absent or
 * unreadable -> undefined (callers apply a documented fallback); entries are
 * never synthesised. Active mode, default mode and the NCM persistence flag are
 * kept separate; unknown extra fields are ignored.
 */
export function mapUsbStatus(d: Record<string, unknown>): UsbStatus {
  const link = obj(d.link)
  const configfs = obj(d.configfs)
  const bridge = obj(d.bridge)
  const interfaces = obj(d.interfaces)
  const usbIds = obj(d.usb_ids)
  const defaultMode = parseUsbMode(d.default_mode)
  return {
    active_mode: parseUsbMode(d.active_mode),
    default_mode: defaultMode ?? undefined,
    link: link && {
      negotiated: str(link.negotiated),
      negotiated_label: str(link.negotiated_label),
      negotiated_mbps: nonNegative(link.negotiated_mbps),
      max: str(link.max),
      max_label: str(link.max_label),
      max_mbps: nonNegative(link.max_mbps),
      at_full_speed: boolLike(link.at_full_speed),
    },
    ncm_persist_on_boot: boolLike(d.ncm_persist_on_boot),
    supported_modes: strList(d.supported_modes) ?? [],
    experimental_modes: strList(d.experimental_modes),
    mode_capabilities: mapUsbCapabilities(d.mode_capabilities),
    composition_functions: strList(d.composition_functions),
    configfs: configfs && {
      present: boolLike(configfs.present),
      ncm: boolLike(configfs.ncm),
      gsi_ecm: boolLike(configfs.gsi_ecm),
      gsi_rndis: boolLike(configfs.gsi_rndis),
    },
    bridge: bridge && { name: str(bridge.name), members: strList(bridge.members) },
    interfaces: interfaces && {
      ecm0: boolLike(interfaces.ecm0),
      rndis0: boolLike(interfaces.rndis0),
      ncm0: boolLike(interfaces.ncm0),
      ncm_ifname: str(interfaces.ncm_ifname) ?? null,
    },
    usb_ids: usbIds && { vendor: str(usbIds.vendor) ?? null, product: str(usbIds.product) ?? null },
    ncm_last_error: str(d.ncm_last_error),
    connect: finiteNumber(d.connect),
    typec_cc: str(d.typec_cc),
  }
}

/**
 * Map the `PUT /api/usb/mode` answer: `status: "scheduled"` (NCM, ECM rollback
 * from NCM; HTTP 202) vs anything else (ubus passthrough for ECM/RNDIS).
 * A scheduled answer never throws, even with an unreadable mode: the switch
 * has been accepted and the caller must go and verify it.
 */
export function mapUsbModeResult(d: Record<string, unknown>): UsbModeResult {
  if (d.status === 'scheduled') {
    const rollback = str(d.rollback)
    return {
      state: 'scheduled',
      mode: parseUsbMode(d.mode),
      experimental: boolLike(d.experimental) ?? false,
      delayMs: nonNegative(d.delay_ms) ?? null,
      ...(rollback !== undefined ? { rollback } : {}),
    }
  }
  return { state: 'applied', raw: d }
}

// ── APN ───────────────────────────────────────────────────────────────────────

/** `apn_mode`: 0 = automatic, 1 = manual. Missing or any other value is 'unknown', not automatic. */
export function mapApnMode(d: Record<string, unknown>): ApnModeState {
  const raw = d.apn_mode
  const n = finiteNumber(raw)
  return { mode: n === 0 ? 'auto' : n === 1 ? 'manual' : 'unknown', raw }
}

/**
 * Map `apnListArray`. Missing/null list = no profiles; a non-array throws.
 * Entries without a usable profileId (string or number) are dropped; the
 * firmware's mixed number/string/boolean encodings are normalised here.
 */
export function mapApnProfiles(d: Record<string, unknown>): ApnProfile[] {
  const list = d.apnListArray
  if (list == null) return []
  if (!Array.isArray(list)) throw new Error('Malformed APN profile list')
  const out: ApnProfile[] = []
  for (const p of list) {
    if (!isObj(p)) continue
    const profileId =
      nonEmptyStr(p.profileId) !== undefined
        ? (p.profileId as string).trim()
        : typeof p.profileId === 'number' && Number.isFinite(p.profileId)
          ? String(p.profileId)
          : undefined
    if (profileId === undefined || out.some((x) => x.profileId === profileId)) continue
    out.push({
      profilename: str(p.profilename) ?? '',
      wanapn: str(p.wanapn) ?? '',
      username: str(p.username) ?? '',
      password: str(p.password) ?? '',
      pdpType: intInRange(p.pdpType, 0, 255) ?? null,
      pppAuthMode: intInRange(p.pppAuthMode, 0, 255) ?? null,
      profileId,
      isEnable: boolLike(p.isEnable) ?? false,
    })
  }
  return out
}

// ── Capabilities / charge control / TTL ───────────────────────────────────────

function bandList(v: unknown): number[] {
  return normaliseBands((arr(v) ?? []).flatMap((x) => {
    const n = finiteNumber(x)
    return n === undefined ? [] : [n]
  }))
}

/** Modem capabilities. Missing lists mean "no support claimed", never a guessed default. */
export function mapModemCapabilities(d: Record<string, unknown>): ModemCapabilities {
  const network_modes: ModemCapabilities['network_modes'] = []
  for (const m of arr(d.network_modes) ?? []) {
    if (!isObj(m)) continue
    const value = nonEmptyStr(m.value)
    if (value !== undefined) network_modes.push({ value, label: nonEmptyStr(m.label) ?? value })
  }
  return {
    network_modes,
    lte_bands: bandList(d.lte_bands),
    nr_sa_bands: bandList(d.nr_sa_bands),
    nr_nsa_band_lock_supported: boolLike(d.nr_nsa_band_lock_supported) ?? false,
  }
}

/**
 * Charge-control state. The limit/hysteresis/enabled fields drive the slider
 * and toggle, so a payload where they are unreadable is rejected (callers see
 * an error/unavailable state) rather than rendered with invented values.
 * Availability flags default to false: unreadable never means available.
 */
export function mapChargeControl(d: Record<string, unknown>): ChargeControlState {
  const charge_limit = intInRange(d.charge_limit, 0, 100)
  const hysteresis = nonNegative(d.hysteresis)
  const charge_limit_enabled = boolLike(d.charge_limit_enabled)
  if (charge_limit === undefined || hysteresis === undefined || charge_limit_enabled === undefined) {
    throw new Error('Malformed charge control response')
  }
  return {
    last_error: str(d.last_error) ?? null,
    available: boolLike(d.available) ?? false,
    battery_available: boolLike(d.battery_available) ?? false,
    charger_available: boolLike(d.charger_available) ?? false,
    charging_stopped: boolLike(d.charging_stopped) ?? null,
    battery_status: str(d.battery_status) ?? null,
    capacity: intInRange(d.capacity, 0, 100) ?? null,
    charge_limit_enabled,
    charge_limit,
    hysteresis,
    manual_override: boolLike(d.manual_override) ?? false,
  }
}

/** TTL status; unreadable fields are undefined (unknown), never "disabled". */
export function mapTtlStatus(d: Record<string, unknown>): TtlStatus {
  return {
    active: boolLike(d.active),
    ipv6_active: boolLike(d.ipv6_active),
    ttl_value: intInRange(d.ttl_value, 0, 255),
  }
}

// ── API surface ───────────────────────────────────────────────────────────────

export const api = {
  // Home — single batched request
  home: () => get('/api/dashboard').then(mapHome),

  // Device
  device: () => get('/api/device').then(mapDevice),
  cpu: () => get('/api/cpu').then(mapCpu),
  memory: () => get('/api/memory').then(mapMemory),
  reboot: () => post('/api/device/reboot', undefined, { 'X-Confirm': 'true' }),
  shutdown: () => post('/api/device/shutdown', undefined, { 'X-Confirm': 'true' }),

  // Network
  clients: () => get('/api/network/clients').then(mapClients),
  dataUsageResetDaySet: (reset_day: number) => put('/api/data-usage/reset-day', { reset_day }).then(mapDataUsage),

  // Modem / SIM
  simInfo: () => get('/api/sim/info').then(mapSim),
  simImei: () => get('/api/sim/imei'),
  modemCapabilities: () => get('/api/modem/capabilities').then(mapModemCapabilities),
  networkModeSet: (net_select: string) => put('/api/modem/network-mode', { net_select }),

  // WiFi
  wifiStatus: () => get('/api/wifi/status').then(mapWifi),
  wifiSet: (body: Record<string, unknown>) => put('/api/wifi/settings', body),

  // Router
  dnsGet: () => get('/api/router/dns').then(mapDns),
  dnsSet: (body: Record<string, unknown>) => put('/api/router/dns', body),
  lanGet: () => get('/api/router/lan').then(mapLan),
  lanSet: (body: Record<string, unknown>) => put('/api/router/lan', body),

  // Thermal / charger / power
  thermalAll: () => get('/api/device/thermal/all').then((d) => d as unknown as ThermalAll),
  batteryInfoUbus: () => get('/api/device/battery-info').then(mapBatteryBspInfo),
  batteryDetail: () => get('/api/device/battery/detail').then((d) => d as unknown as BatteryDetail),
  chargerInfo: () => get('/api/device/charger'),
  chargeControl: () => get('/api/device/charge-control').then(mapChargeControl),
  // The agent answers the PUT with the full updated state (device_ext.rs ends
  // in charge_control_get), so re-fetching it would be a wasted round trip.
  chargeControlSet: (body: Partial<ChargeControlState>) =>
    put('/api/device/charge-control', body).then(mapChargeControl),

  // APN
  apnModeGet: () => get('/api/router/apn/mode').then(mapApnMode),
  apnModeSet: (body: Record<string, unknown>) => put('/api/router/apn/mode', body),
  apnProfiles: () => get('/api/router/apn/profiles').then(mapApnProfiles),
  apnAdd: (body: Record<string, unknown>) => post('/api/router/apn/profiles', body),
  apnDelete: (body: Record<string, unknown>) => post('/api/router/apn/profiles/delete', body),
  apnActivate: (body: Record<string, unknown>) => post('/api/router/apn/profiles/activate', body),

  // SMS: the agent owns translation to ZTE's legacy WMS payloads.
  smsCapabilities: () => get('/api/sms/capabilities').then((d) => d as unknown as SmsCapabilities),
  smsList: () => post('/api/sms/list', { page: 0, per_page: 500 }).then(mapSmsList),
  /** Same request, also reporting how many malformed/duplicate entries were dropped. */
  smsListChecked: () => post('/api/sms/list', { page: 0, per_page: 500 }).then(mapSmsListResult),
  smsSend: (number: string, message: string) => post('/api/sms/send', { number, message }),
  smsDelete: (ids: number[]) => post('/api/sms/delete', { ids }),
  smsRead: (ids: number[]) => post('/api/sms/read', { ids }),

  // System
  top: () => get('/api/system/top').then((d) => d as unknown as ProcessListResult),
  killBloat: () =>
    post('/api/system/kill-bloat', { all: true }, { 'X-Confirm': 'true' }).then(
      (d) => d as unknown as KillBloatResult,
    ),
  restartAgent: () => post('/api/system/restart-agent', {}),

  // USB
  usbMode: (mode: string, options?: { confirm_experimental?: boolean }) =>
    put('/api/usb/mode', { mode, ...(options ?? {}) }).then(mapUsbModeResult),
  usbDefaultMode: (mode: UsbMode, options?: { confirm_experimental?: boolean }) =>
    put('/api/usb/default', { mode, ...(options ?? {}) }),
  usbStatus: () => get('/api/usb/status').then(mapUsbStatus),
  usbPowerbank: (on: boolean) => put('/api/usb/powerbank', { state: on ? 1 : 0 }),

  // TTL
  ttlStatus: () => get('/api/ttl/status').then(mapTtlStatus),
  ttlSet: (ttl: number) => put('/api/ttl/set', { ttl }),
  ttlClear: () => req('DELETE', '/api/ttl/clear'),

  // Signal logger
  loggerSignalStart: (duration_secs: number, interval_secs: number) =>
    post('/api/logger/signal/start', { duration_secs, interval_secs }),
  loggerSignalStop: () => post('/api/logger/signal/stop', {}),
  loggerSignalStatus: () => get('/api/logger/signal/status').then((d) => d as unknown as LoggerStatus),
  loggerSignalDownload: () => readCsv('/api/logger/signal/download'),

  // Connection logger
  loggerConnectionStart: (duration_secs: number, interval_secs: number) =>
    post('/api/logger/connection/start', { duration_secs, interval_secs }),
  loggerConnectionStop: () => post('/api/logger/connection/stop', {}),
  loggerConnectionStatus: () => get('/api/logger/connection/status').then((d) => d as unknown as LoggerStatus),
  loggerConnectionDownload: () =>
    readCsv('/api/logger/connection/download'),

  // AT console
  atSend: (command: string, timeout?: number) =>
    post('/api/at/send', { command, timeout }).then((d) => d as unknown as AtSendResult),
  atPort: () => get('/api/at/port').then((d) => d as unknown as { port: string | null; available: boolean }),

  // Band lock
  // NR: per ZTE-script-NG, nr5g_type "SA" is the only working type. "NSA won't work here."
  bandLockNr: (bands: string) => post('/api/cell/band/nr', { nr5g_type: 'SA', nr5g_band: bands }),
  // LTE: lte_band_mask must be a decimal bitmask string (band N = bit N-1)
  bandLockLte: (bandNumbers: number[]) => {
    let mask = BigInt(0)
    for (const b of bandNumbers) mask |= BigInt(1) << BigInt(b - 1)
    return post('/api/cell/band/lte', {
      is_lte_band: '1',
      lte_band_mask: mask.toString(),
      is_gw_band: '0',
      gw_band_mask: '0',
    })
  },
  bandLockReset: () => post('/api/cell/band/reset'),

  // Cell lock — param names must match ZTE ubus method signatures
  cellLockNr: (pci: string, earfcn: string, band: string) =>
    post('/api/cell/lock/nr', { lock_nr_pci: pci, lock_nr_earfcn: earfcn, lock_nr_cell_band: band }),
  cellLockLte: (pci: string, earfcn: string) =>
    post('/api/cell/lock/lte', { lock_lte_pci: pci, lock_lte_earfcn: earfcn }),
  cellLockReset: () => post('/api/cell/lock/reset'),
}
