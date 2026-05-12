import { Activity, BatteryCharging, BatteryMedium, Download, Radio, Signal, Upload } from 'lucide-react'
import {
  api, formatSpeed, formatBytes, formatBandwidthMHz, sumBandwidthMHz,
} from '../api'
import Card, { Stat } from '../components/Card'
import { usePolling } from '../hooks/usePolling'

function signalColor(rsrp?: number) {
  if (rsrp == null) return 'text-gray-500'
  if (rsrp > -80) return 'text-green-500'
  if (rsrp > -100) return 'text-yellow-500'
  return 'text-red-500'
}

function signalBg(rsrp?: number) {
  if (rsrp == null) return 'bg-gray-400'
  if (rsrp > -80) return 'bg-green-500'
  if (rsrp > -100) return 'bg-yellow-500'
  return 'bg-red-500'
}

function signalQuality(rsrp?: number) {
  if (rsrp == null) return '\u2014'
  if (rsrp > -80) return 'Excellent'
  if (rsrp > -90) return 'Good'
  if (rsrp > -100) return 'Fair'
  return 'Weak'
}

function SignalBars({ bars, large = false }: { bars?: number; large?: boolean }) {
  const n = bars ?? 0
  const color = n >= 4 ? '#22c55e' : n >= 2 ? '#eab308' : '#ef4444'
  const heights = large ? [12, 20, 28, 36, 44] : [4, 7, 10, 13, 16]
  const width = large ? 'w-4' : 'w-2.5'
  return (
    <div className="flex items-end gap-1">
      {heights.map((h, i) => (
        <div
          key={i}
          className={`${width} rounded-sm transition-colors`}
          style={{ height: `${h}px`, backgroundColor: i < n ? color : '#cbd5e1' }}
        />
      ))}
    </div>
  )
}

function BatteryIcon({ percent, charging, large = false }: { percent: number; charging: boolean; large?: boolean }) {
  const fill = percent > 20 ? (percent > 50 ? '#22c55e' : '#eab308') : '#ef4444'
  return (
    <div className={`relative flex items-center ${large ? 'h-9 w-16' : 'h-6 w-10'}`}>
      <div className={`${large ? 'h-8 w-14' : 'h-5 w-9'} rounded border-2 border-gray-500 bg-white`}>
        <div className="h-full rounded-sm transition-all" style={{ width: `${percent}%`, backgroundColor: fill }} />
      </div>
      <div className={`absolute ${large ? '-right-1.5 h-3 w-2' : '-right-1 h-2 w-1.5'} rounded-r bg-gray-500`} />
      {charging && <span className="absolute inset-0 flex items-center justify-center text-[10px] font-bold text-white">&#x26A1;</span>}
    </div>
  )
}

function formatUptime(secs?: number) {
  if (!secs) return '\u2014'
  const d = Math.floor(secs / 86400)
  const h = Math.floor((secs % 86400) / 3600)
  const m = Math.floor((secs % 3600) / 60)
  return [d && `${d}d`, (d || h) && `${h}h`, `${m}m`].filter(Boolean).join(' ')
}

function modemMode(type?: string) {
  const raw = (type ?? '').toUpperCase()
  if (!raw) return '\u2014'
  if (raw.includes('ENDC') || raw.includes('NSA')) return 'ENDC'
  if (raw.includes('SA')) return 'SA'
  if (raw.includes('LTE') || raw === '4G') return 'LTE'
  if (raw.includes('NR') || raw.includes('5G')) return '5G'
  return raw
}

function modeTone(mode: string) {
  if (mode === 'SA') return 'bg-purple-100 text-purple-700 border-purple-200'
  if (mode === 'ENDC') return 'bg-indigo-100 text-indigo-700 border-indigo-200'
  if (mode === 'LTE') return 'bg-slds-blue/10 text-slds-blueHover border-slds-blue/30'
  return 'bg-gray-100 text-gray-600 border-gray-200'
}

function modeSubtext(mode: string, hasNR: boolean, hasLTE: boolean) {
  if (mode === 'ENDC') return 'LTE anchor + NR carrier'
  if (mode === 'SA') return 'Standalone NR'
  if (mode === 'LTE') return 'LTE only'
  if (hasNR && hasLTE) return 'NR + LTE'
  if (hasNR) return 'NR'
  if (hasLTE) return 'LTE'
  return 'No active carriers'
}

function Row({ label, value, wrap }: { label: string; value: string; wrap?: boolean }) {
  return (
    <div className="flex justify-between gap-2">
      <span className="shrink-0 text-gray-500">{label}</span>
      <span className={`text-right font-medium text-gray-900 ${wrap ? 'break-all' : 'truncate'}`}>{value}</span>
    </div>
  )
}

function Chip({ children, tone = 'default' }: { children: React.ReactNode; tone?: 'default' | 'lte' | 'nr' }) {
  const cls = tone === 'nr'
    ? 'bg-purple-100 text-purple-700 border-purple-200'
    : tone === 'lte'
      ? 'bg-slds-blue/10 text-slds-blueHover border-slds-blue/30'
      : 'bg-gray-100 text-gray-600 border-gray-200'
  return <span className={`rounded-lg border px-2 py-0.5 text-[11px] font-bold ${cls}`}>{children}</span>
}

export default function DashboardPage() {
  const { data, error } = usePolling(async () => {
    const results = await Promise.allSettled([
      api.signal(), api.battery(), api.speed(), api.device(),
      api.wan(), api.wan6(), api.cpu(), api.memory(), api.dataUsage(),
    ])
    const [sig, bat, spd, dev, w, w6, c, m, u] = results
    return {
      signal: sig.status === 'fulfilled' ? sig.value : null,
      battery: bat.status === 'fulfilled' ? bat.value : null,
      speed: spd.status === 'fulfilled' ? spd.value : null,
      device: dev.status === 'fulfilled' ? dev.value : null,
      wan: w.status === 'fulfilled' ? w.value : null,
      wan6: w6.status === 'fulfilled' ? w6.value : null,
      cpu: c.status === 'fulfilled' ? c.value : null,
      mem: m.status === 'fulfilled' ? m.value : null,
      usage: u.status === 'fulfilled' ? u.value : null,
    }
  }, 3000)

  const signal = data?.signal ?? null
  const battery = data?.battery ?? null
  const speed = data?.speed ?? null
  const device = data?.device ?? null
  const wan = data?.wan ?? null
  const wan6 = data?.wan6 ?? null
  const cpu = data?.cpu ?? null
  const mem = data?.mem ?? null
  const usage = data?.usage ?? null

  const primaryCarrier = signal?.lte_carriers?.[0] || signal?.nr_carriers?.[0]
  const pccRsrp = primaryCarrier?.rsrp ?? signal?.rsrp
  const lteBandwidth = signal ? sumBandwidthMHz(signal.lte_carriers) : 0
  const nrBandwidth = signal ? sumBandwidthMHz(signal.nr_carriers) : 0
  const totalBandwidth = lteBandwidth + nrBandwidth
  const hasLTE = Boolean(signal?.lte_carriers.length)
  const hasNR = Boolean(signal?.nr_carriers.length)
  const mode = modemMode(signal?.type)
  const carrierCount = (signal?.lte_carriers.length ?? 0) + (signal?.nr_carriers.length ?? 0)

  return (
    <div className="space-y-5">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl sm:text-3xl font-bold text-gray-900">Dashboard</h1>
          <p className="mt-0.5 text-sm text-gray-500">{signal?.carrier ?? 'Mobile broadband status'}</p>
        </div>
        {error && <span className="text-xs text-red-500">{error}</span>}
      </div>

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-5">
        <Card className="sm:col-span-2 xl:col-span-2">
          <div className="flex min-h-[174px] flex-col justify-between gap-5">
            <div className="flex items-start justify-between gap-4">
              <div>
                <div className="flex items-center gap-2 text-gray-500">
                  <Signal className="h-4 w-4" />
                  <p className="text-xs font-bold uppercase">Signal strength</p>
                </div>
                <div className="mt-3 flex items-end gap-3">
                  <p className={`text-5xl font-bold leading-none ${signalColor(pccRsrp)}`}>
                    {pccRsrp != null ? pccRsrp : '\u2014'}
                  </p>
                  <div className="pb-1">
                    <p className="text-sm font-bold text-gray-900">dBm RSRP</p>
                    <p className="text-xs text-gray-500">{signalQuality(pccRsrp)}</p>
                  </div>
                </div>
              </div>
              <SignalBars bars={signal?.signal_bars} large />
            </div>
            <div className="flex flex-wrap items-center gap-2 border-t border-gray-200/60 pt-3">
              <span className={`h-2.5 w-2.5 rounded-full ${signalBg(pccRsrp)}`} />
              <span className="text-sm text-gray-600">{signal?.signal_bars ?? 0}/5 bars</span>
              {primaryCarrier?.band && <Chip tone={primaryCarrier.band.startsWith('n') ? 'nr' : 'lte'}>{primaryCarrier.band}</Chip>}
              {primaryCarrier?.pci != null && <span className="text-xs text-gray-500">PCI {primaryCarrier.pci}</span>}
            </div>
          </div>
        </Card>

        <Card>
          <div className="flex min-h-[174px] flex-col justify-between">
            <div>
              <div className="flex items-center gap-2 text-gray-500">
                <Radio className="h-4 w-4" />
                <p className="text-xs font-bold uppercase">Modem mode</p>
              </div>
              <span className={`mt-4 inline-flex rounded-xl border px-3 py-1.5 text-2xl font-bold ${modeTone(mode)}`}>
                {mode}
              </span>
              <p className="mt-2 text-sm text-gray-500">{modeSubtext(mode, hasNR, hasLTE)}</p>
            </div>
            <div className="border-t border-gray-200/60 pt-3 text-sm text-gray-600">
              {carrierCount} carrier{carrierCount !== 1 ? 's' : ''}
            </div>
          </div>
        </Card>

        <Card>
          <div className="flex min-h-[174px] flex-col justify-between">
            <div>
              <div className="flex items-center gap-2 text-gray-500">
                <Activity className="h-4 w-4" />
                <p className="text-xs font-bold uppercase">Spectrum</p>
              </div>
              <p className="mt-4 text-4xl font-bold leading-none text-gray-900">{formatBandwidthMHz(totalBandwidth)}</p>
              <p className="mt-2 text-sm text-gray-500">Aggregated bandwidth</p>
            </div>
            <div className="flex flex-wrap gap-1.5 border-t border-gray-200/60 pt-3">
              {nrBandwidth > 0 && <Chip tone="nr">NR {formatBandwidthMHz(nrBandwidth)}</Chip>}
              {lteBandwidth > 0 && <Chip tone="lte">LTE {formatBandwidthMHz(lteBandwidth)}</Chip>}
              {totalBandwidth <= 0 && <span className="text-sm text-gray-500">No carrier bandwidth reported</span>}
            </div>
          </div>
        </Card>

        <Card>
          <div className="flex min-h-[174px] flex-col justify-between">
            <div>
              <div className="flex items-center gap-2 text-gray-500">
                {battery?.charging ? <BatteryCharging className="h-4 w-4" /> : <BatteryMedium className="h-4 w-4" />}
                <p className="text-xs font-bold uppercase">Battery</p>
              </div>
              <div className="mt-4 flex items-center gap-4">
                <p className="text-4xl font-bold leading-none text-gray-900">
                  {battery?.percent != null ? `${battery.percent}%` : '\u2014'}
                </p>
                {battery && <BatteryIcon percent={battery.percent} charging={battery.charging} large />}
              </div>
              <p className="mt-2 text-sm text-gray-500">{battery?.charging ? 'Charging' : 'On battery'}</p>
            </div>
            <div className="border-t border-gray-200/60 pt-3 text-sm text-gray-600">
              {battery?.voltage_mv ? `${(battery.voltage_mv / 1000).toFixed(2)}V` : '\u2014'}
              {battery?.temperature_c ? ` \u00b7 ${battery.temperature_c}\u00b0C` : ''}
            </div>
          </div>
        </Card>
      </div>

      <div className="grid grid-cols-1 gap-4 xl:grid-cols-3">
        <Card title="Radio Details" className="xl:col-span-2">
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
            <Stat label="RSRP" value={pccRsrp != null ? `${pccRsrp} dBm` : '\u2014'} color={signalColor(pccRsrp)} />
            <Stat label="RSRQ" value={primaryCarrier?.rsrq != null ? `${primaryCarrier.rsrq} dB` : '\u2014'} />
            <Stat label="SINR" value={primaryCarrier?.sinr != null ? `${primaryCarrier.sinr} dB` : '\u2014'} />
            <Stat label="RSSI" value={primaryCarrier?.rssi != null ? `${primaryCarrier.rssi} dBm` : '\u2014'} />
          </div>
          {(hasLTE || hasNR) && (
            <div className="grid grid-cols-1 gap-4 border-t border-gray-200/60 pt-4 md:grid-cols-2">
              <div>
                <p className="mb-2 text-xs font-bold uppercase text-slds-blue">LTE</p>
                {signal && signal.lte_carriers.length > 0 ? (
                  <div className="flex flex-wrap gap-1.5">
                    {signal.lte_carriers.map((c, i) => (
                      <Chip key={i} tone="lte">{c.band}{c.rsrp != null ? ` ${c.rsrp} dBm` : ''}</Chip>
                    ))}
                  </div>
                ) : (
                  <p className="text-sm text-gray-500">No active LTE carrier</p>
                )}
              </div>
              <div>
                <p className="mb-2 text-xs font-bold uppercase text-purple-600">5G NR</p>
                {signal && signal.nr_carriers.length > 0 ? (
                  <div className="flex flex-wrap gap-1.5">
                    {signal.nr_carriers.map((c, i) => (
                      <Chip key={i} tone="nr">{c.band}{c.rsrp != null ? ` ${c.rsrp} dBm` : ''}</Chip>
                    ))}
                  </div>
                ) : (
                  <p className="text-sm text-gray-500">No active NR carrier</p>
                )}
              </div>
            </div>
          )}
        </Card>

        <Card title="Throughput">
          <div className="grid grid-cols-2 gap-4">
            <div>
              <div className="flex items-center gap-2 text-green-500">
                <Download className="h-4 w-4" />
                <p className="text-xs font-bold uppercase">Download</p>
              </div>
              <p className="mt-1 text-2xl font-bold text-green-500">{speed ? formatSpeed(speed.rx_bps) : '\u2014'}</p>
              {speed && speed.max_rx_bps > 0 && <p className="text-xs text-gray-500">Peak {formatSpeed(speed.max_rx_bps)}</p>}
            </div>
            <div>
              <div className="flex items-center gap-2 text-slds-blue">
                <Upload className="h-4 w-4" />
                <p className="text-xs font-bold uppercase">Upload</p>
              </div>
              <p className="mt-1 text-2xl font-bold text-slds-blue">{speed ? formatSpeed(speed.tx_bps) : '\u2014'}</p>
              {speed && speed.max_tx_bps > 0 && <p className="text-xs text-gray-500">Peak {formatSpeed(speed.max_tx_bps)}</p>}
            </div>
          </div>
        </Card>
      </div>

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-3">
        <Card title="Connection">
          <div className="space-y-2 text-sm">
            <Row label="Operator" value={signal?.carrier ?? '\u2014'} />
            <Row label="IPv4" value={wan?.ipv4 ?? '\u2014'} />
            <Row label="Gateway" value={wan?.gateway ?? '\u2014'} />
            <Row label="IPv6" value={wan6?.ipv6 ?? '\u2014'} wrap />
            {wan6?.prefix && <Row label="IPv6 Prefix" value={wan6.prefix} wrap />}
            {wan?.dns && wan.dns.length > 0 && (
              <Row label="DNS (v4)" value={wan.dns.filter(d => !d.includes(':')).join(', ') || '\u2014'} />
            )}
            {wan6?.dns && wan6.dns.length > 0 && (
              <Row label="DNS (v6)" value={wan6.dns.join(', ')} wrap />
            )}
          </div>
        </Card>

        <Card title="Device">
          <div className="space-y-2 text-sm">
            <Row label="Model" value={device?.model ?? '\u2014'} />
            <Row label="Firmware" value={device?.firmware ?? '\u2014'} />
            <Row label="Uptime" value={formatUptime(device?.uptime_secs)} />
            {cpu && <Row label="CPU" value={`${cpu.overall.toFixed(1)}%`} />}
            {mem && <Row label="Memory" value={`${mem.usage_pct.toFixed(0)}%`} />}
          </div>
        </Card>

        <Card title="Data Usage">
          {usage ? (
            <div className="space-y-3">
              {[
                { label: 'Today', data: usage.day },
                { label: 'This Month', data: usage.month },
                { label: 'Total', data: usage.total },
              ].map(({ label, data }) => (
                <div key={label}>
                  <p className="text-xs font-bold uppercase text-gray-500">{label}</p>
                  <div className="mt-0.5 flex gap-4 text-sm">
                    <span className="text-green-500">&#x2193; {formatBytes(data.rx_bytes)}</span>
                    <span className="text-slds-blue">&#x2191; {formatBytes(data.tx_bytes)}</span>
                  </div>
                </div>
              ))}
            </div>
          ) : (
            <p className="text-sm text-gray-500">Loading...</p>
          )}
        </Card>
      </div>
    </div>
  )
}
