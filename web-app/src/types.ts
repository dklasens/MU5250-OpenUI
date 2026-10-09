// Shared domain types for the agent API.

export interface CarrierComponent {
  label: string // "PCC", "SCC0", "SCC1", etc.
  band: string // "B8", "n78"
  /** Physical cell ID. 0 is a valid PCI; undefined = the firmware did not report a valid one. */
  pci?: number
  earfcn: number
  bandwidth: string // "10 MHz"
  freq?: number // MHz, calculated from EARFCN
  rsrp?: number
  rsrq?: number
  sinr?: number
  rssi?: number
  ul_configured?: boolean
  active?: boolean
}

/**
 * Observed band-lock state. `unknown` (missing/unparseable) is not `automatic`
 * (a known absence of any explicit restriction); `locked` carries sorted,
 * deduplicated band numbers.
 */
export type BandLockState =
  | { kind: 'unknown' }
  | { kind: 'automatic' }
  | { kind: 'locked'; bands: number[] }

/** The carrier Home/Signal should treat as the serving cell, picked from validated carriers. */
export interface PrimaryCarrier {
  rat: 'lte' | 'nr'
  carrier: CarrierComponent
}

export interface SignalInfo {
  type?: string
  carrier?: string
  /** 0–5. undefined = not reported; a genuine 0 stays 0. */
  signal_bars?: number
  cell_id?: string
  lte_carriers: CarrierComponent[]
  nr_carriers: CarrierComponent[]
  /**
   * Serving carrier chosen by network mode from validated carriers only:
   * SA -> NR PCC; LTE/4G -> LTE PCC; NSA/ENDC -> the LTE anchor PCC (the NR leg
   * is an SCG shown in nr_carriers); nothing valid -> undefined (never a raw
   * fallback, and never LTE while in SA even though the firmware populates
   * LTE fields there).
   */
  primary?: PrimaryCarrier
  net_select?: string
  lte_band_lock_state: BandLockState
  nr_sa_band_lock_state: BandLockState
  nr_nsa_band_lock_state: BandLockState
  raw_lte_band_lock?: string
  /** Display string "SA=<raw> NSA=<raw>" (diagnostics only). */
  raw_nr_band_lock?: string
  raw_nr_sa_band_lock?: string
  raw_nr_nsa_band_lock?: string
  band?: string
}

export interface BatteryInfo {
  percent: number
  /** Raw kernel status: Charging, Full, Not charging, Discharging. */
  status?: string
  charging: boolean
  /** On external power, whether or not the battery is charging. */
  plugged: boolean
  voltage_mv?: number
  temperature_c?: number
  current_ma?: number
}

/** Live WAN throughput, in **bytes** per second (`formatSpeed` converts to bits). */
export interface SpeedInfo {
  rx_bps: number
  tx_bps: number
  max_rx_bps: number
  max_tx_bps: number
}

export interface DeviceInfo {
  model: string
  /** ZTE build, e.g. XCBZ_HK_MU5250V1.0.0B04. */
  firmware?: string
  hardware?: string
  /** Kernel release from /proc/version. */
  kernel?: string
  uptime_secs?: number
  load_avg?: number[]
}

export interface WanInfo {
  connected: boolean
  ipv4?: string
  ipv6?: string
  gateway?: string
  dns?: string[]
  apn?: string
}

export interface Wan6Info {
  connected: boolean
  ipv6?: string
  prefix?: string
  dns?: string[]
}

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

export interface CpuInfo {
  overall: number
  cores: number[]
}

export type UsbMode = 'ecm' | 'rndis' | 'ncm'

export interface UsbModeCapability {
  mode: UsbMode
  supported: boolean
  experimental: boolean
  function?: string
  note?: string
}

export interface UsbLink {
  negotiated?: string
  negotiated_label?: string
  negotiated_mbps?: number
  max?: string
  max_label?: string
  max_mbps?: number
  at_full_speed?: boolean
}

/**
 * Result of `PUT /api/usb/mode`. NCM (and ECM rollback from NCM) are only
 * *scheduled* (HTTP 202, `status: "scheduled"`): the active mode has not
 * changed yet and must be verified by re-reading status. ECM/RNDIS go through
 * ZTE's ubus and return its payload unchanged ('applied' = accepted by ubus;
 * the firmware's own reboot semantics apply).
 */
export type UsbModeResult =
  | { state: 'scheduled'; mode: UsbMode | null; experimental: boolean; delayMs: number | null; rollback?: string }
  | { state: 'applied'; raw: Record<string, unknown> }

export interface UsbStatus {
  active_mode: UsbMode | null
  default_mode?: UsbMode
  link?: UsbLink
  ncm_persist_on_boot?: boolean
  supported_modes: string[]
  experimental_modes?: string[]
  mode_capabilities?: UsbModeCapability[]
  composition_functions?: string[]
  configfs?: { present?: boolean; ncm?: boolean; gsi_ecm?: boolean; gsi_rndis?: boolean }
  bridge?: { name?: string; members?: string[] }
  interfaces?: { ecm0?: boolean; rndis0?: boolean; ncm0?: boolean; ncm_ifname?: string | null }
  usb_ids?: { vendor?: string | null; product?: string | null }
  ncm_last_error?: string
  connect?: number
  typec_cc?: string
}

export interface MemInfo {
  total_kb: number
  used_kb: number
  free_kb: number
  usage_pct: number
}

export interface WifiBand {
  ssid?: string
  enabled: boolean
  channel?: number
  bandwidth?: string
  configuredChannel?: string
  /** Raw UCI htmode, PHY mode + width, e.g. "EHT80". */
  configuredBandwidth?: string
  /** configuredBandwidth normalised to MHz (R13); undefined if unrecognised. */
  configuredWidthMhz?: number
  /** Observed runtime width normalised to MHz (R13); undefined if missing/unrecognised. */
  actualWidthMhz?: number
  /** Configured TX power as a percentage, validated integer 1–100 (R09). undefined = unknown. */
  txpowerPercent?: number
  bandwidthOptions?: string[]
  supportedStandards?: string
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
  master_supported: boolean
  master_enabled: boolean
  wifi6_supported: boolean
  wifi6_enabled?: boolean
  wifi7_supported: boolean
}

export interface DnsConfig {
  primary: string
  secondary: string
  ipv6_primary?: string
  ipv6_secondary?: string
}

export interface LanConfig {
  ipaddr: string
  netmask: string
  dhcp_enabled: boolean
  dhcp_start: string
  dhcp_end: string
  lease_seconds: number
}

export interface ModemCapabilities {
  network_modes: { value: string; label: string }[]
  lte_bands: number[]
  nr_sa_bands: number[]
  nr_nsa_band_lock_supported: boolean
}

export interface ThermalInfo {
  cpu_temp_c?: number
}

export interface ThermalAll {
  available: boolean
  cpu_0?: number
  cpu_1?: number
  cpu_2?: number
  cpu_3?: number
  modem?: number
  modem_ss0?: number
  modem_ss1?: number
  modem_ss2?: number
  battery?: number
  usb?: number
  eth_phy?: number
  pmic?: number
  xo_therm?: number
  pa?: number
  sdr?: number
}

export interface BatteryBspInfo {
  available: boolean
  online: boolean | null
  low_power: boolean | null
  using_hw_fg_chip: boolean | null
  time_to_full_mins: number | null
  time_to_empty_mins: number | null
}

export interface BatteryDetail {
  available: boolean
  capacity: number | null
  status: string | null
  voltage_mv: number | null
  voltage_max_mv: number | null
  voltage_ocv_mv: number | null
  current_ma: number | null
  power_mw: number | null
  temperature_c: number | null
  charge_type: string | null
  health: string | null
  cycle_count: number | null
  charge_counter_mah: number | null
  charge_full_mah: number | null
  charge_full_design_mah: number | null
  time_to_full_secs: number | null
  time_to_empty_secs: number | null
}

export interface ChargeControlState {
  last_error?: string | null
  available: boolean
  battery_available: boolean
  charger_available: boolean
  charging_stopped: boolean | null
  battery_status: string | null
  capacity: number | null
  charge_limit_enabled: boolean
  charge_limit: number
  hysteresis: number
  manual_override: boolean
}

export interface ApnProfile {
  profilename: string
  wanapn: string
  username: string
  password: string
  /** 1=IPv4, 2=IPv6, 3=IPv4v6; null = missing/unrecognised. */
  pdpType: number | null
  /** 0=None, 1=PAP, 2=CHAP, 3=PAP/CHAP; null = missing/unrecognised. */
  pppAuthMode: number | null
  profileId: string
  isEnable: boolean
}

/** Validated `apn_mode` (0 = automatic, 1 = manual). Missing/other values are 'unknown', never 'auto'. */
export interface ApnModeState {
  mode: 'auto' | 'manual' | 'unknown'
  raw: unknown
}

export interface SimInfo {
  iccid?: string
  imsi?: string
  state?: string
  mcc?: string
  mnc?: string
}

/** A date with no time or zone, as the router reports it. Never a UTC instant. */
export interface CalendarDate {
  year: number
  month: number // 1–12
  day: number // 1–31
}

/** null = the agent could not read the counter (unknown); 0 is a real measured zero. */
export interface UsagePeriod {
  rx_bytes: number | null
  tx_bytes: number | null
  time_secs: number | null
}

export interface DataUsage {
  day: UsagePeriod
  month: UsagePeriod
  /** Same firmware month counters as `month`. Missing in the payload = undefined. */
  cycle?: UsagePeriod
  /**
   * Firmware `real_*` counters. The API name is historical: these are NOT
   * proven to be "since power on" (observed counter time 15.8 h vs uptime
   * 20.5 h); they behave like connection-scoped counters. Keep the field name
   * for contract compatibility; do not word the UI as "since power on".
   */
  since_power_on?: UsagePeriod
  total: UsagePeriod
  /** 1–31, or null when unknown (agent emits null when neither ubus nor UCI could be read). */
  reset_day: number | null
  /** null = unknown (not "disabled"). */
  reset_enabled: boolean | null
  /** Raw firmware strings, kept for display/debugging. */
  clear_date_record?: string
  next_clear_date?: string
  /** `clear_date_record` parsed as a calendar date; null if missing/invalid. */
  cycle_start?: CalendarDate | null
  /** `next_clear_date` parsed as a calendar date; null if missing/invalid. */
  next_reset?: CalendarDate | null
}

export interface SmsMessage {
  id: number
  /** Sender (inbox) or recipient (sent) number. */
  number: string
  content: string
  date?: string
  /** Firmware tags: 0=received/read, 1=received/unread, 2=sent, 3=failed, 4=draft. */
  tag: number
  /** Firmware storage: 1=native/NV device storage. */
  mem_store?: number
}

export interface SmsCapabilities {
  available: boolean
  ready: boolean
  object: string
  storage?: string
  reason?: string
}

/** One row of `GET /api/system/top` — mirrors agent/src/system.rs::ProcessEntry. */
export interface ProcessInfo {
  pid: number
  name: string
  cpu_pct: number
  rss_kb: number
  state: string
  /** True for daemons on the agent's kill-safe bloat allowlist. */
  is_bloat: boolean
}

/** `GET /api/system/top` — mirrors agent/src/system.rs::ProcessListResult. */
export interface ProcessListResult {
  processes: ProcessInfo[]
  total_count: number
  bloat_count: number
  bloat_cpu_pct: number
  bloat_rss_kb: number
}

export interface KilledProcess {
  pid: number
  name: string
}

/** `POST /api/system/kill-bloat` — mirrors agent/src/system.rs::KillBloatResult. */
export interface KillBloatResult {
  killed: KilledProcess[]
  skipped: KilledProcess[]
  freed_rss_kb: number
}

export interface LoggerStatus {
  last_error?: string | null
  max_bytes?: number
  flush_interval_secs?: number
  running: boolean
  samples?: number
  events?: number
  elapsed_secs: number
  duration_secs: number
  interval_secs: number
}

export interface LoggerDownload {
  csv: string
}

export interface TtlStatus {
  active?: boolean
  ipv6_active?: boolean
  ttl_value?: number
}

export interface AtSendResult {
  command?: string
  response: string
  port?: string
  elapsed_ms?: number
}

/** One merged poll of /api/dashboard — the home screen's single request. */
export interface SourceFreshness {
  sampled_at_ms: number | null
  age_ms: number | null
  ttl_ms: number
  stale: boolean
  error: string | null
}
/** QoS the network granted on the data session, as logged by the stock data daemon. */
export interface BearerInfo {
  qci: number | null
  ambr_dl_mbps: number | null
  ambr_ul_mbps: number | null
}
export interface HomeData {
  sources?: Record<string, SourceFreshness>
  charge_control_error?: string | null
  signal: SignalInfo | null
  bearer: BearerInfo | null
  battery: BatteryInfo | null
  speed: SpeedInfo | null
  device: DeviceInfo | null
  wan: WanInfo | null
  wan6: Wan6Info | null
  cpu: CpuInfo | null
  memory: MemInfo | null
  usage: DataUsage | null
  thermal: ThermalInfo | null
}
