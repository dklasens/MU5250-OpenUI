// API base — same host as the dashboard, port 9090
export const API_BASE = `http://${window.location.hostname}:9090`
export const AUTH_EXPIRED_EVENT = 'zte-auth-expired'

let _token: string | null = sessionStorage.getItem('zte_token')

export function setToken(t: string) { _token = t; sessionStorage.setItem('zte_token', t) }
export function clearToken() { _token = null; sessionStorage.removeItem('zte_token') }
export function hasToken() { return !!_token }

export class ApiError extends Error {
  status?: number

  constructor(message: string, status?: number) {
    super(message)
    this.name = 'ApiError'
    this.status = status
  }
}

function emitAuthExpired() {
  clearToken()
  window.dispatchEvent(new Event(AUTH_EXPIRED_EVENT))
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
async function req(
  method: string,
  path: string,
  body?: unknown,
  extraHeaders?: Record<string, string>,
): Promise<any> {
  const controller = new AbortController()
  const timeout = setTimeout(() => controller.abort(), 15_000)
  const headers: Record<string, string> = { ...(extraHeaders ?? {}) }
  if (_token) headers['Authorization'] = `Bearer ${_token}`
  if (body !== undefined) headers['Content-Type'] = 'application/json'
  try {
    let res: Response
    try {
      res = await fetch(`${API_BASE}${path}`, {
        method,
        headers,
        body: body !== undefined ? JSON.stringify(body) : undefined,
        signal: controller.signal,
      })
    } catch (error) {
      if (error instanceof Error && error.name === 'AbortError') {
        throw new ApiError('Timed out reaching the agent')
      }
      throw new ApiError(`Failed to reach the agent at ${API_BASE}`)
    }

    let json: { ok?: boolean; data?: unknown; error?: string }
    try {
      json = await res.json()
    } catch {
      throw new ApiError(`Invalid response from agent (${res.status})`, res.status)
    }

    if (res.status === 401 && path !== '/api/auth/login') {
      emitAuthExpired()
    }
    if (!res.ok || !json.ok) {
      throw new ApiError(json.error ?? `request failed (${res.status})`, res.status)
    }
    return json.data
  } finally {
    clearTimeout(timeout)
  }
}

const get  = (path: string) => req('GET', path)
const post = (path: string, body?: unknown, extraHeaders?: Record<string, string>) =>
  req('POST', path, body, extraHeaders)
const put  = (path: string, body: unknown) => req('PUT', path, body)

// ── Auth ──────────────────────────────────────────────────────────────────────
export async function login(credentials: string | { password?: string; pin?: string }): Promise<{ token: string }> {
  const body = typeof credentials === 'string' ? { password: credentials } : credentials
  const data = await req('POST', '/api/auth/login', body)
  return { token: data.token }
}

// ── Types ─────────────────────────────────────────────────────────────────────

export interface CarrierComponent {
  label: string        // "PCC", "SCC0", "SCC1", etc.
  band: string         // "B8", "n78"
  pci: number
  earfcn: number
  bandwidth: string    // "10 MHz"
  freq?: number        // MHz, calculated from EARFCN
  rsrp?: number
  rsrq?: number
  sinr?: number
  rssi?: number
  ul_configured?: boolean
  active?: boolean
}

export interface SignalInfo {
  // Operator / network
  type?: string; carrier?: string; signal_bars?: number; cell_id?: string
  // LTE primary + all carriers
  lte_carriers: CarrierComponent[]
  // NR primary + all carriers
  nr_carriers: CarrierComponent[]
  // Network mode
  net_select?: string
  // Current band locks (from device)
  lte_band_lock?: number[]   // parsed from hex bitmask
  nr_band_lock?: number[]    // parsed from comma-separated
  // Raw lock values from firmware (for diagnostics)
  raw_lte_band_lock?: string
  raw_nr_band_lock?: string
  // Raw for backward compat
  rsrp?: number; band?: string
}

export interface BatteryInfo { percent: number; charging: boolean; voltage_mv?: number; temperature_c?: number; current_ma?: number }
export interface SpeedInfo { rx_bps: number; tx_bps: number; max_rx_bps: number; max_tx_bps: number; rx_bytes: number; tx_bytes: number; uptime_secs: number }
export interface DeviceInfo { model: string; firmware?: string; uptime_secs?: number; load_avg?: number[] }
export interface WanInfo { connected: boolean; ipv4?: string; ipv6?: string; gateway?: string; dns?: string[]; apn?: string }
export interface Wan6Info { connected: boolean; ipv6?: string; prefix?: string; dns?: string[] }
export interface Client {
  mac: string
  ip?: string
  hostname?: string
  medium?: 'wifi' | 'usb-c' | 'ethernet' | 'wired'
  medium_detail?: 'wifi_2ghz' | 'wifi_5ghz' | 'usb_c' | 'ethernet'
  interface?: string
  wifi_band?: string
  signal_dbm?: number
  tx_bitrate_mbps?: number
  rx_bitrate_mbps?: number
  expected_throughput_mbps?: number
  connected_secs?: number
  wired_link_mbps?: number
}
export interface CpuInfo { overall: number; cores: number[] }
export type UsbMode = 'ecm' | 'rndis' | 'ncm'
export interface UsbModeCapability {
  mode: UsbMode
  supported: boolean
  experimental: boolean
  function?: string
  note?: string
}
export interface UsbStatus {
  active_mode: UsbMode | null
  default_mode?: UsbMode
  ncm_persist_on_boot?: boolean
  supported_modes: string[]
  experimental_modes?: string[]
  mode_capabilities?: UsbModeCapability[]
  composition_functions?: string[]
  configfs?: {
    present?: boolean
    ncm?: boolean
    gsi_ecm?: boolean
    gsi_rndis?: boolean
  }
  bridge?: {
    name?: string
    members?: string[]
  }
  interfaces?: {
    ecm0?: boolean
    rndis0?: boolean
    ncm0?: boolean
    ncm_ifname?: string | null
  }
  usb_ids?: {
    vendor?: string | null
    product?: string | null
  }
  ncm_last_error?: string
  connect?: number
  typec_cc?: string
}
export interface MemInfo { total_kb: number; used_kb: number; free_kb: number; usage_pct: number }
export interface WifiBand {
  ssid?: string
  enabled: boolean
  channel?: number
  bandwidth?: string
  configuredChannel?: string
  configuredBandwidth?: string
  actualChannel?: number
  actualBandwidth?: string
  password?: string
  security?: string
  hidden: boolean
  clients?: number
}
export interface WifiAll {
  band_2g: WifiBand
  band_5g: WifiBand
  guest_ssid?: string
  persist_on_boot: boolean
  master_supported: boolean
  master_enabled: boolean
  wifi6_supported: boolean
  wifi6_enabled?: boolean
}
export interface DnsConfig { primary: string; secondary: string; ipv6_primary?: string; ipv6_secondary?: string }
export interface LanConfig { ip: string; netmask: string; dhcp_start: string; dhcp_end: string; dhcp_lease: string }
export interface ThermalInfo { cpu_temp_c?: number }
export interface ThermalAll {
  cpu_0?: number; cpu_1?: number; cpu_2?: number; cpu_3?: number
  modem?: number; modem_ss0?: number; modem_ss1?: number; modem_ss2?: number
  battery?: number; usb?: number; eth_phy?: number; pmic?: number
  xo_therm?: number; pa?: number; sdr?: number
}
export interface BatteryBspInfo {
  online: boolean
  low_power: boolean
  using_hw_fg_chip: boolean
  time_to_full_mins?: number
  time_to_empty_mins?: number
}
export interface BatteryDetail {
  capacity: number; status: string
  voltage_mv: number; voltage_max_mv: number; voltage_ocv_mv: number; current_ma: number
  power_mw: number; temperature_c: number
  charge_type: string; health: string; cycle_count: number
  charge_counter_mah: number; charge_full_mah: number; charge_full_design_mah: number
  time_to_full_secs: number; time_to_empty_secs: number
}
export interface ApnProfile {
  profilename: string; wanapn: string; username: string; password: string
  pdpType: number; pppAuthMode: number; profileId: string; isEnable: boolean
}
export interface SimInfo { iccid?: string; imsi?: string; state?: string; mcc?: string; mnc?: string }
export interface DataUsage {
  day: UsagePeriod
  month: UsagePeriod
  cycle?: UsagePeriod
  since_power_on?: UsagePeriod
  total: UsagePeriod
  reset_day?: number
  reset_enabled?: boolean
  clear_date_record?: string
  next_clear_date?: string
}
export interface UsagePeriod { rx_bytes: number; tx_bytes: number; time_secs: number }
export interface SmsMessage { id: number; from?: string; text?: string; date?: string; read?: boolean }
export interface SmsCapacity { inbox: number; total: number; unread: number }
export interface SpeedtestResult { status?: string; download_mbps?: number; upload_mbps?: number; ping_ms?: number; server?: string }
export interface ProcessInfo { pid?: number; name?: string; cpu_percent?: number; mem_kb?: number }

// ── EARFCN / NR-ARFCN to frequency ──────────────────────────────────────────

const LTE_BANDS: Record<number, { fdl_low: number; noffs_dl: number }> = {
  1:  { fdl_low: 2110,  noffs_dl: 0 },
  2:  { fdl_low: 1930,  noffs_dl: 600 },
  3:  { fdl_low: 1805,  noffs_dl: 1200 },
  4:  { fdl_low: 2110,  noffs_dl: 1950 },
  5:  { fdl_low: 869,   noffs_dl: 2400 },
  7:  { fdl_low: 2620,  noffs_dl: 2750 },
  8:  { fdl_low: 925,   noffs_dl: 3450 },
  12: { fdl_low: 729,   noffs_dl: 5010 },
  13: { fdl_low: 746,   noffs_dl: 5180 },
  14: { fdl_low: 758,   noffs_dl: 5280 },
  17: { fdl_low: 734,   noffs_dl: 5730 },
  18: { fdl_low: 860,   noffs_dl: 5850 },
  19: { fdl_low: 875,   noffs_dl: 6000 },
  20: { fdl_low: 791,   noffs_dl: 6150 },
  25: { fdl_low: 1930,  noffs_dl: 8040 },
  26: { fdl_low: 859,   noffs_dl: 8690 },
  28: { fdl_low: 758,   noffs_dl: 9210 },
  29: { fdl_low: 717,   noffs_dl: 9660 },
  30: { fdl_low: 2350,  noffs_dl: 9770 },
  32: { fdl_low: 1452,  noffs_dl: 9920 },
  34: { fdl_low: 2010,  noffs_dl: 36200 },
  38: { fdl_low: 2570,  noffs_dl: 37750 },
  39: { fdl_low: 1880,  noffs_dl: 38250 },
  40: { fdl_low: 2300,  noffs_dl: 38650 },
  41: { fdl_low: 2496,  noffs_dl: 39650 },
  42: { fdl_low: 3400,  noffs_dl: 41590 },
  43: { fdl_low: 3600,  noffs_dl: 43590 },
  48: { fdl_low: 3550,  noffs_dl: 55240 },
  66: { fdl_low: 2110,  noffs_dl: 66436 },
  71: { fdl_low: 617,   noffs_dl: 68586 },
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

// ── Transforms ────────────────────────────────────────────────────────────────

function parseNum(v: unknown): number | undefined {
  if (typeof v === 'number') return v
  if (typeof v === 'string') {
    const t = v.trim()
    if (!t || t === '--' || t === 'N/A') return undefined
    const n = parseFloat(t)
    return isNaN(n) ? undefined : n
  }
  return undefined
}

function parseNumAllowZero(v: unknown): number | undefined {
  if (typeof v === 'number') return v
  if (typeof v === 'string') {
    const t = v.trim()
    if (!t || t === '--' || t === 'N/A') return undefined
    const n = parseFloat(t)
    return isNaN(n) ? undefined : n
  }
  return undefined
}

function formatCellId(id?: number | string | null): string | undefined {
  if (id == null) return undefined
  const num = typeof id === 'string' ? parseInt(id, 10) : id
  if (isNaN(num) || num <= 0) return undefined
  const nodeId = num >>> 8
  const sectorId = num & 0xFF
  return `${nodeId.toString(16).toUpperCase()}|${sectorId.toString(16).toUpperCase()}`
}

function mapSignal(d: Record<string, unknown>): SignalInfo {
  const pccPci = d.lte_pci as number | undefined
  const pccEarfcn = d.wan_active_channel as number | undefined
  const pccBandStr = (d.wan_active_band as string) ?? ''
  const pccBandNum = parseInt(pccBandStr.replace(/\D/g, '')) || 0
  const pccBw = extractPccBw(d.lteca as string | undefined, pccPci, pccEarfcn)
  const snr = d.lte_snr as string | undefined

  // Build LTE PCC — skip ghost carriers (e.g. in 5G SA mode)
  const lteCarriers: CarrierComponent[] = []
  const lteRsrp = parseNum(d.lte_rsrp)
  const lteHasValidData = pccBandStr && pccBandStr !== '0' && pccBandStr !== 'B' && pccBandStr !== 'B0' && pccEarfcn != null && pccEarfcn > 0

  if (lteRsrp != null && lteHasValidData) {
    lteCarriers.push({
      label: 'PCC',
      band: pccBandNum ? `B${pccBandNum}` : pccBandStr,
      pci: pccPci ?? 0,
      earfcn: pccEarfcn,
      bandwidth: pccBw ? `${pccBw} MHz` : '—',
      freq: pccBandNum ? earfcnToFreq(pccEarfcn, pccBandNum) : undefined,
      rsrp: lteRsrp,
      rsrq: parseNum(d.lte_rsrq),
      sinr: snr ? parseFloat(snr) : undefined,
      rssi: parseNum(d.lte_rssi),
      ul_configured: true,
      active: true,
    })

    // Parse lteca SCCs
    const ltecaStr = (d.lteca as string) ?? ''
    const ltecasigStr = (d.ltecasig as string) ?? ''
    const ltecaEntries = parseLteCa(ltecaStr, pccPci, pccEarfcn)
    const ltecaSigs = parseCaSig(ltecasigStr)

    for (let i = 0; i < ltecaEntries.length; i++) {
      const e = ltecaEntries[i]
      const sig = i < ltecaSigs.length ? ltecaSigs[i] : undefined
      const bandNum = parseInt(e.band) || 0
      lteCarriers.push({
        label: `SCC${i}`,
        band: `B${e.band}`,
        pci: parseInt(e.pci) || 0,
        earfcn: parseInt(e.earfcn) || 0,
        bandwidth: `${e.bw} MHz`,
        freq: bandNum ? earfcnToFreq(parseInt(e.earfcn) || 0, bandNum) : undefined,
        rsrp: sig?.rsrp === 0 ? undefined : sig?.rsrp,
        rsrq: sig?.rsrq === 0 ? undefined : sig?.rsrq,
        sinr: sig?.sinr,
        rssi: sig?.rssi === 0 ? undefined : sig?.rssi,
        ul_configured: sig?.ul_configured,
        active: sig?.active,
      })
    }
  }

  // Build NR primary — skip ghost carriers (no valid band or ARFCN, e.g. in 4G-only mode)
  const nrCarriers: CarrierComponent[] = []
  const nrRsrp = parseNum(d.nr5g_rsrp)
  const nrBand = (d.nr5g_action_band as string) ?? ''
  const nrArfcn = (d.nr5g_action_channel as number) ?? 0
  const nrHasValidData = nrBand && nrBand !== '0' && nrBand !== 'n' && nrBand !== 'n0' && nrArfcn > 0
  if (nrRsrp != null && nrHasValidData) {
    const nrBwRaw = (d.nr5g_bandwidth as string) ?? ''
    nrCarriers.push({
      label: 'PCC',
      band: nrBand.startsWith('n') ? nrBand : `n${nrBand}`,
      pci: (d.nr5g_pci as number) ?? 0,
      earfcn: nrArfcn,
      bandwidth: nrBwRaw ? `${nrBwRaw} MHz` : '—',
      freq: nrArfcn ? nrarfcnToFreq(nrArfcn) : undefined,
      rsrp: nrRsrp,
      rsrq: parseNumAllowZero(d.nr5g_rsrq),
      sinr: parseNum(d.nr5g_snr),
      rssi: parseNumAllowZero(d.nr5g_rssi),
      ul_configured: true,
      active: true,
    })

    // Parse nrca SCCs — format: index,pci,?,band,arfcn,bw,...,rsrp,rsrq,sinr,rssi
    const nrcaStr = (d.nrca as string) ?? ''
    const nrPccPci = d.nr5g_pci as number | undefined
    const nrPccArfcn = nrArfcn
    for (const seg of nrcaStr.split(';')) {
      if (!seg.trim()) continue
      const parts = seg.split(',')
      if (parts.length < 6) continue
      const sPci = parseInt(parts[1]) || 0
      const sArfcn = parseInt(parts[4]) || 0
      // Skip if this is the PCC (same PCI + ARFCN)
      if (sPci === nrPccPci && sArfcn === nrPccArfcn) continue
      const sBand = parseInt(parts[3]) || 0
      const sBw = parts[5]
      nrCarriers.push({
        label: `SCC${nrCarriers.length - 1}`,
        band: `n${sBand}`,
        pci: sPci,
        earfcn: sArfcn,
        bandwidth: `${sBw} MHz`,
        freq: sArfcn ? nrarfcnToFreq(sArfcn) : undefined,
        rsrp: parts.length >= 8 ? parseNum(parts[7]) : undefined,
        rsrq: parts.length >= 9 ? parseNum(parts[8]) : undefined,
        sinr: parts.length >= 10 ? parseNum(parts[9]) : undefined,
        rssi: parts.length >= 11 ? parseNum(parts[10]) : undefined,
        ul_configured: parts.length > 0 ? parts[0].trim() === '1' : undefined,
        active: parts.length > 2 ? parts[2].trim() === '2' : undefined,
      })
    }
  }

  // Parse current band locks from device
  let lte_band_lock: number[] | undefined
  const lteLockStr = d.lte_band_lock as string | undefined
  if (lteLockStr && lteLockStr !== '0') {
    try {
      const mask = BigInt(lteLockStr)
      const bands: number[] = []
      for (let b = 1; b <= 71; b++) {
        if ((mask >> BigInt(b - 1)) & BigInt(1)) bands.push(b)
      }
      if (bands.length > 0) lte_band_lock = bands
    } catch { /* ignore parse errors */ }
  }

  let nr_band_lock: number[] | undefined
  const nrSaStr = (d.nr5g_sa_band_lock as string) ?? ''
  const nrNsaStr = (d.nr5g_nsa_band_lock as string) ?? ''
  // Merge SA and NSA lock lists (they should be identical, but use SA as primary)
  const nrLockStr = nrSaStr || nrNsaStr
  if (nrLockStr) {
    const bands = nrLockStr.split(',').map(s => parseInt(s.trim())).filter(n => n > 0)
    if (bands.length > 0) nr_band_lock = bands
  }

  const netType = d.network_type as string | undefined
  const is4g = netType === '4G' || netType === 'LTE' || netType === 'NSA' || netType === 'ENDC'
  const rawCellId = (is4g && d.cell_id) ? d.cell_id : (d.nr5g_cell_id || d.cell_id)

  return {
    type: netType,
    carrier: (d.network_provider_fullname || d.network_provider) as string | undefined,
    signal_bars: d.signalbar ? parseInt(d.signalbar as string) : undefined,
    cell_id: formatCellId(rawCellId as number | string | undefined),
    lte_carriers: lteCarriers,
    nr_carriers: nrCarriers,
    net_select: d.net_select as string | undefined,
    lte_band_lock,
    nr_band_lock,
    raw_lte_band_lock: String(d.lte_band_lock ?? ''),
    raw_nr_band_lock: `SA=${String(d.nr5g_sa_band_lock ?? '')} NSA=${String(d.nr5g_nsa_band_lock ?? '')}`,
    rsrp: parseNum(d.lte_rsrp) ?? parseNum(d.nr5g_rsrp),
    band: pccBandStr,
  }
}

/** Parse lteca entries, returning SCCs only (excluding PCC by PCI+EARFCN match) */
function parseLteCa(ltecaStr: string, pccPci?: number, pccEarfcn?: number) {
  const sccs: { pci: string; band: string; earfcn: string; bw: string }[] = []
  let pccFound = false
  for (const seg of ltecaStr.split(';')) {
    if (!seg.trim()) continue
    const p = seg.split(',')
    if (p.length < 5) continue
    const entryPci = parseInt(p[0]) || 0
    const entryEarfcn = parseInt(p[3]) || 0
    if (!pccFound && entryPci === pccPci && entryEarfcn === pccEarfcn) {
      pccFound = true
      continue
    }
    sccs.push({ pci: p[0], band: p[1], earfcn: p[3], bw: p[4] })
  }
  return sccs
}

/** Extract PCC bandwidth from lteca string */
function extractPccBw(ltecaStr: string | undefined, pccPci?: number, pccEarfcn?: number): string | undefined {
  if (!ltecaStr) return undefined
  for (const seg of ltecaStr.split(';')) {
    if (!seg.trim()) continue
    const p = seg.split(',')
    if (p.length < 5) continue
    if ((parseInt(p[0]) || 0) === pccPci && (parseInt(p[3]) || 0) === pccEarfcn) return p[4]
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

function mapBattery(d: Record<string, unknown>): BatteryInfo {
  return {
    percent: d.capacity as number,
    charging: d.status === 'Charging',
    voltage_mv: d.voltage_uv ? Math.round((d.voltage_uv as number) / 1000) : undefined,
    temperature_c: d.temperature ? (d.temperature as number) / 10 : undefined,
    current_ma: d.current_ua ? Math.round((d.current_ua as number) / 1000) : undefined,
  }
}

function mapSpeed(d: Record<string, unknown>): SpeedInfo {
  return {
    rx_bps: (d.real_rx_speed as number) || 0,
    tx_bps: (d.real_tx_speed as number) || 0,
    max_rx_bps: (d.real_max_rx_speed as number) || 0,
    max_tx_bps: (d.real_max_tx_speed as number) || 0,
    rx_bytes: (d.real_rx_bytes as number) || 0,
    tx_bytes: (d.real_tx_bytes as number) || 0,
    uptime_secs: (d.real_time as number) || 0,
  }
}

function mapDevice(d: Record<string, unknown>): DeviceInfo {
  const kernel = d.kernel as string | undefined
  const ver = kernel?.match(/Linux version (\S+)/)?.[1]
  return {
    model: 'ZTE U60 Pro',
    firmware: ver,
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
  // New format: { clients: [{ mac, ip, hostname, ... }] }
  const clients = d.clients as Array<Record<string, unknown>> | undefined
  if (clients) {
    return clients.map(c => ({
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
  // Legacy fallback: DHCP leases
  const leases = d.dhcp_leases as Array<{ macaddr: string; ipaddr: string; hostname: string }> | undefined
  if (!leases) return []
  return leases.map(l => ({ mac: l.macaddr, ip: l.ipaddr, hostname: l.hostname }))
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

function mapWifi(d: Record<string, unknown>): WifiAll {
  const parseBoolLike = (value: unknown, fallback: boolean): boolean => {
    if (typeof value === 'boolean') return value
    if (typeof value === 'number') return value !== 0
    if (typeof value === 'string') {
      const normalized = value.trim().toLowerCase()
      if (['1', 'true', 'on', 'yes', 'enabled'].includes(normalized)) return true
      if (['0', 'false', 'off', 'no', 'disabled'].includes(normalized)) return false
    }
    return fallback
  }
  const parseChannelNumber = (value: unknown): number | undefined => {
    const raw = String(value ?? '')
    const n = parseInt(raw, 10)
    return Number.isFinite(n) ? n : undefined
  }
  const persistRaw = d.persist_on_boot
  const persistOnBoot = parseBoolLike(persistRaw, true)
  const masterSupported = parseBoolLike(d.wifi_onoff_supported, Object.prototype.hasOwnProperty.call(d, 'wifi_onoff'))
  const masterEnabled = parseBoolLike(d.wifi_onoff, true)
  const wifi6Supported = parseBoolLike(d.wifi6_supported, Object.prototype.hasOwnProperty.call(d, 'wifi6_switch'))
  const wifi6Enabled = wifi6Supported ? parseBoolLike(d.wifi6_switch, false) : undefined
  const configuredChannel2g = String(d.channel_2g ?? 'auto') || 'auto'
  const configuredChannel5g = String(d.channel_5g ?? 'auto') || 'auto'
  const actualChannel2g = parseChannelNumber(d.actual_channel_2g)
  const actualChannel5g = parseChannelNumber(d.actual_channel_5g)
  const actualBw2g = d.actual_bw_2g as string | undefined
  const actualBw5g = d.actual_bw_5g as string | undefined
  return {
    band_2g: {
      ssid: d.ssid_2g as string | undefined,
      enabled: d.radio2_disabled !== '1',
      channel: actualChannel2g,
      bandwidth: actualBw2g,
      configuredChannel: configuredChannel2g === '0' ? 'auto' : configuredChannel2g,
      configuredBandwidth: d.htmode_2g as string | undefined,
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
      configuredBandwidth: d.htmode_5g as string | undefined,
      actualChannel: actualChannel5g,
      actualBandwidth: actualBw5g,
      password: (d.key_5g as string) || (d.has_key_5g ? '••••••••' : undefined),
      security: d.encryption_5g as string | undefined,
      hidden: d.hidden_5g === '1',
      clients: d.clients_5g as number | undefined,
    },
    guest_ssid: d.guest_ssid as string | undefined,
    persist_on_boot: persistOnBoot,
    master_supported: masterSupported,
    master_enabled: masterEnabled,
    wifi6_supported: wifi6Supported,
    wifi6_enabled: wifi6Enabled,
  }
}

function mapDns(d: Record<string, unknown>): DnsConfig {
  return {
    primary: d.prefer_dns_manual as string || '',
    secondary: d.standby_dns_manual as string || '',
    ipv6_primary: d.ipv6_wan_prefer_dns_manual as string | undefined,
    ipv6_secondary: d.ipv6_wan_standby_dns_manual as string | undefined,
  }
}

function mapLan(d: Record<string, unknown>): LanConfig {
  return {
    ip: d.lan_ipaddr as string || '',
    netmask: d.lan_netmask as string || '',
    dhcp_start: d.dhcp_start as string || '',
    dhcp_end: d.dhcp_end as string || '',
    dhcp_lease: d.dhcp_lease_time as string || '',
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

function mapDataUsage(d: Record<string, unknown>): DataUsage {
  const p = (v: Record<string, unknown>): UsagePeriod => ({
    rx_bytes: Number(v.rx_bytes ?? 0), tx_bytes: Number(v.tx_bytes ?? 0), time_secs: Number(v.time_secs ?? 0),
  })
  const cycle = d.cycle as Record<string, unknown> | undefined
  const sincePowerOn = d.since_power_on as Record<string, unknown> | undefined
  return {
    day: p(d.day as Record<string, unknown>),
    month: p(d.month as Record<string, unknown>),
    cycle: cycle ? p(cycle) : undefined,
    since_power_on: sincePowerOn ? p(sincePowerOn) : undefined,
    total: p(d.total as Record<string, unknown>),
    reset_day: d.reset_day != null ? Number(d.reset_day) : undefined,
    reset_enabled: d.reset_enabled === true || Number(d.reset_enabled) === 1,
    clear_date_record: d.clear_date_record as string | undefined,
    next_clear_date: d.next_clear_date as string | undefined,
  }
}

function mapSmsCapacity(d: Record<string, unknown>): SmsCapacity {
  return {
    inbox: d.sms_nv_rev_total as number || 0,
    total: d.sms_nv_total as number || 0,
    unread: d.sms_dev_unread_num as number || 0,
  }
}

function mapThermal(d: Record<string, unknown>): ThermalInfo {
  return { cpu_temp_c: d.cpuss_temp as number | undefined }
}

function mapBatteryBspInfo(d: Record<string, unknown>): BatteryBspInfo {
  return {
    online: d.battery_online === 1,
    low_power: d.battery_low_power === 1,
    using_hw_fg_chip: d.battery_using_hw_fg_chip === 1,
    time_to_full_mins: d.battery_time_to_full as number | undefined,
    time_to_empty_mins: d.battery_time_to_empty as number | undefined,
  }
}

// ── API ───────────────────────────────────────────────────────────────────────
export const api = {
  // Device
  device:  () => get('/api/device').then(mapDevice),
  battery: () => get('/api/battery').then(mapBattery),
  cpu:     () => get('/api/cpu').then(mapCpu),
  memory:  () => get('/api/memory').then(mapMemory),
  reboot:  () => post('/api/device/reboot', undefined, { 'X-Confirm': 'true' }),
  shutdown: () => post('/api/device/shutdown', undefined, { 'X-Confirm': 'true' }),

  // Network
  signal:   () => get('/api/network/signal').then(mapSignal),
  speed:    () => get('/api/network/speeds').then(mapSpeed),
  wan:      () => get('/api/network/wan').then(mapWan),
  wan6:     () => get('/api/network/wan6').then(mapWan6),
  clients:  () => get('/api/network/clients').then(mapClients),
  dataUsage: () => get('/api/data-usage').then(mapDataUsage),
  dataUsageResetDaySet: (reset_day: number) => put('/api/data-usage/reset-day', { reset_day }).then(mapDataUsage),

  // Modem
  simInfo:  () => get('/api/sim/info').then(mapSim),
  simImei:  () => get('/api/sim/imei'),

  // WiFi
  wifiStatus: () => get('/api/wifi/status').then(mapWifi),
  wifiSet:    (body: Record<string, unknown>) => put('/api/wifi/settings', body),

  // Router
  dnsGet:  () => get('/api/router/dns').then(mapDns),
  dnsSet:  (body: Record<string, unknown>) => put('/api/router/dns', body),
  lanGet:  () => get('/api/router/lan').then(mapLan),
  lanSet:  (body: Record<string, unknown>) => put('/api/router/lan', body),

  // Thermal / charger
  thermal: () => get('/api/device/thermal').then(mapThermal),
  thermalAll: () => get('/api/device/thermal/all') as Promise<ThermalAll>,
  batteryInfoUbus: () => get('/api/device/battery-info').then(mapBatteryBspInfo),
  batteryDetail: () => get('/api/device/battery/detail') as Promise<BatteryDetail>,

  // APN
  apnModeGet:      () => get('/api/router/apn/mode'),
  apnModeSet:      (body: Record<string, unknown>) => put('/api/router/apn/mode', body),
  apnProfiles:     () => get('/api/router/apn/profiles'),
  apnAdd:          (body: Record<string, unknown>) => post('/api/router/apn/profiles', body),
  apnModify:       (body: Record<string, unknown>) => put('/api/router/apn/profiles', body),
  apnDelete:       (body: Record<string, unknown>) => post('/api/router/apn/profiles/delete', body),
  apnActivate:     (body: Record<string, unknown>) => post('/api/router/apn/profiles/activate', body),

  // SMS
  smsList:     (box: number) => post('/api/sms/list', { cmd: 1, page: 0, data_per_page: 500, mem_store: box, tags: 0, order_by: 'date,desc' }),
  smsSend:     (to: string, text: string) => post('/api/sms/send', { to, text }),
  smsDelete:   (ids: number[]) => post('/api/sms/delete', { ids }),
  smsRead:     (ids: number[]) => post('/api/sms/read', { ids }),
  smsCapacity: () => get('/api/sms/capacity').then(mapSmsCapacity),

  // Speed test
  speedtestStart:    (server_id?: string) => post('/api/speedtest/start', { server_id }),
  speedtestProgress: () => get('/api/speedtest/progress'),
  speedtestStop:     () => post('/api/speedtest/stop', {}),

  // System
  top: () => get('/api/system/top'),
  restartAgent: () => post('/api/system/restart-agent', {}),

  // USB
  usbMode: (mode: string, options?: { confirm_experimental?: boolean }) =>
    put('/api/usb/mode', { mode, ...(options ?? {}) }),
  usbDefaultMode: (mode: UsbMode, options?: { confirm_experimental?: boolean }) =>
    put('/api/usb/default', { mode, ...(options ?? {}) }),
  usbStatus: (): Promise<UsbStatus> => get('/api/usb/status'),

  // TTL
  ttlStatus: () => get('/api/ttl/status'),
  ttlSet:    (ttl: number) => put('/api/ttl/set', { ttl }),
  ttlClear:  () => req('DELETE', '/api/ttl/clear'),

  // LAN test
  lanPing:     () => fetch(`${API_BASE}/api/lan/ping`, { headers: _token ? { Authorization: `Bearer ${_token}` } : {} }),
  lanDownload: (size: number) => `${API_BASE}/api/lan/download?size=${size}`,
  lanUpload:   () => `${API_BASE}/api/lan/upload`,

  // Network mode
  networkModeSet: (net_select: string) => put('/api/modem/network-mode', { net_select }),

  // Signal logger
  loggerSignalStart:    (duration_secs: number, interval_secs: number) => post('/api/logger/signal/start', { duration_secs, interval_secs }),
  loggerSignalStop:     () => post('/api/logger/signal/stop', {}),
  loggerSignalStatus:   () => get('/api/logger/signal/status'),
  loggerSignalDownload: () => get('/api/logger/signal/download'),

  // Connection logger
  loggerConnectionStart:    (duration_secs: number, interval_secs: number) => post('/api/logger/connection/start', { duration_secs, interval_secs }),
  loggerConnectionStop:     () => post('/api/logger/connection/stop', {}),
  loggerConnectionStatus:   () => get('/api/logger/connection/status'),
  loggerConnectionDownload: () => get('/api/logger/connection/download'),

  // AT console
  atSend: (command: string, timeout?: number) => post('/api/at/send', { command, timeout }),

  // Band lock
  // NR: per ZTE-script-NG, nr5g_type "SA" is the only working type. "NSA won't work here."
  bandLockNr:    (bands: string) => post('/api/cell/band/nr', { nr5g_type: 'SA', nr5g_band: bands }),
  // LTE: lte_band_mask must be a decimal bitmask string (band N = bit N-1)
  bandLockLte:   (bandNumbers: number[]) => {
    let mask = BigInt(0)
    for (const b of bandNumbers) mask |= BigInt(1) << BigInt(b - 1)
    return post('/api/cell/band/lte', { is_lte_band: '1', lte_band_mask: mask.toString(), is_gw_band: '0', gw_band_mask: '0' })
  },
  bandLockReset: () => post('/api/cell/band/reset'),

  // Cell lock — param names must match ZTE ubus method signatures
  cellLockNr:    (pci: string, earfcn: string, band: string) => post('/api/cell/lock/nr', { lock_nr_pci: pci, lock_nr_earfcn: earfcn, lock_nr_cell_band: band }),
  cellLockLte:   (pci: string, earfcn: string) => post('/api/cell/lock/lte', { lock_lte_pci: pci, lock_lte_earfcn: earfcn }),
  cellLockReset: () => post('/api/cell/lock/reset'),
}

// ── Helpers ───────────────────────────────────────────────────────────────────
export function formatBytes(bytes: number): string {
  if (bytes >= 1e12) return `${(bytes / 1e12).toFixed(1)} TB`
  if (bytes >= 1e9)  return `${(bytes / 1e9).toFixed(1)} GB`
  if (bytes >= 1e6)  return `${(bytes / 1e6).toFixed(1)} MB`
  if (bytes >= 1e3)  return `${(bytes / 1e3).toFixed(0)} KB`
  return `${bytes} B`
}

export function formatSpeed(bps: number): string {
  const mbps = bps * 8 / 1_000_000
  if (mbps >= 1) return `${mbps.toFixed(1)} Mbps`
  const kbps = bps * 8 / 1000
  return `${kbps.toFixed(0)} Kbps`
}

export function parseBandwidthMHz(bandwidth?: string): number {
  if (!bandwidth || bandwidth === '\u2014') return 0
  const match = bandwidth.match(/\d+(?:\.\d+)?/)
  return match ? parseFloat(match[0]) : 0
}

export function sumBandwidthMHz(carriers: { bandwidth?: string }[]): number {
  return carriers.reduce((sum, carrier) => sum + parseBandwidthMHz(carrier.bandwidth), 0)
}

export function formatBandwidthMHz(mhz: number): string {
  if (mhz <= 0) return '\u2014'
  return `${Number.isInteger(mhz) ? mhz.toFixed(0) : mhz.toFixed(1)} MHz`
}
