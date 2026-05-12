import { useState, useEffect, useCallback } from 'react'
import { api, formatBandwidthMHz, sumBandwidthMHz, type SignalInfo, type CarrierComponent } from '../api'
import Card from '../components/Card'

function rsrpColor(v?: number) {
  if (v == null) return 'text-gray-500'
  if (v > -80) return 'text-green-500'
  if (v > -90) return 'text-lime-500'
  if (v > -100) return 'text-yellow-500'
  if (v > -110) return 'text-orange-500'
  return 'text-red-500'
}

function rsrqColor(v?: number) {
  if (v == null) return 'text-gray-500'
  if (v > -10) return 'text-green-500'
  if (v > -15) return 'text-yellow-500'
  return 'text-red-500'
}

function sinrColor(v?: number) {
  if (v == null) return 'text-gray-500'
  if (v > 20) return 'text-green-500'
  if (v > 10) return 'text-lime-500'
  if (v > 0) return 'text-yellow-500'
  return 'text-red-500'
}

function fmt(v?: number, unit = '') {
  if (v == null) return '\u2014'
  return `${v}${unit}`
}

function fmtFreq(mhz?: number) {
  if (mhz == null) return '\u2014'
  return `${mhz.toFixed(mhz % 1 === 0 ? 0 : 2)} MHz`
}

function Tooltip({ text, children }: { text: string; children: React.ReactNode }) {
  const [show, setShow] = useState(false)
  return (
    <span className="relative cursor-help"
      onMouseEnter={() => setShow(true)} onMouseLeave={() => setShow(false)}
      onTouchStart={() => setShow(s => !s)}>
      {children}
      {show && (
        <span className="absolute bottom-full left-0 sm:left-1/2 z-20 mb-2 w-56 sm:w-60 sm:-translate-x-1/2 bg-gray-50/80 backdrop-blur-sm rounded-xl px-2 py-1 text-xs text-gray-600 shadow-sm border border-gray-200/60">
          {text}
        </span>
      )}
    </span>
  )
}

function rsrpTip(v?: number) {
  const b = 'Reference Signal Received Power \u2014 power of a single LTE/NR reference signal. Primary indicator of signal strength.'
  if (v == null) return b
  if (v > -80) return b + ' Currently excellent \u2014 very close to cell tower.'
  if (v > -90) return b + ' Currently good \u2014 reliable connection.'
  if (v > -100) return b + ' Currently fair \u2014 speeds may be reduced.'
  if (v > -110) return b + ' Currently poor \u2014 connection may be unstable.'
  return b + ' Very weak \u2014 consider repositioning the device.'
}

function rsrqTip(v?: number) {
  const b = 'Reference Signal Received Quality \u2014 signal quality accounting for noise and interference from neighbouring cells.'
  if (v == null) return b
  if (v > -10) return b + ' Currently good \u2014 minimal interference.'
  if (v > -15) return b + ' Currently fair \u2014 some interference present.'
  return b + ' Currently poor \u2014 significant interference or cell congestion.'
}

function sinrTip(v?: number) {
  const b = 'Signal to Interference plus Noise Ratio \u2014 how far the signal is above the noise floor. Key metric for achievable throughput.'
  if (v == null) return b
  if (v > 20) return b + ' Excellent \u2014 capable of peak throughput.'
  if (v > 10) return b + ' Good \u2014 solid throughput expected.'
  if (v > 0) return b + ' Fair \u2014 noise is impacting performance.'
  return b + ' Poor \u2014 noise exceeds signal, expect low speeds.'
}

function rssiTip() {
  return 'Received Signal Strength Indicator \u2014 total wideband received power including signal, noise, and interference. Less precise than RSRP for LTE/NR as it measures the entire channel bandwidth.'
}

function CarrierCard({ carrier, tech }: { carrier: CarrierComponent; tech: 'LTE' | 'NR' }) {
  const isPcc = carrier.label === 'PCC'
  const badgeColor = tech === 'NR'
    ? 'rounded-lg bg-purple-100 px-2 py-0.5 text-[9px] font-bold text-purple-700 border border-purple-200 shadow-sm'
    : 'rounded-lg bg-slds-blue/10 px-2 py-0.5 text-[9px] font-bold text-slds-blueHover border border-slds-blue/30 shadow-sm'
  const pccBadge = isPcc
    ? 'rounded-lg bg-slds-blue/10 px-2 py-0.5 text-[9px] font-bold text-slds-blue border border-slds-blue/30 shadow-sm'
    : 'rounded-lg bg-gray-50 px-2 py-0.5 text-[9px] font-bold text-gray-600 border border-gray-200 shadow-sm'

  return (
    <div className="bg-white/95 rounded-2xl shadow-macos-xl border border-gray-200/50 ring-1 ring-black/5">
      <div className="flex items-center gap-2 bg-gray-50/80 backdrop-blur-md px-4 py-2.5 border-b border-gray-200/60">
        <span className={badgeColor}>
          {carrier.band}
        </span>
        <span className={pccBadge}>
          {carrier.label}
        </span>
        {carrier.ul_configured !== undefined && (
          <span className={`rounded-lg px-1.5 py-0.5 text-[9px] font-bold border shadow-sm ${carrier.ul_configured ? 'bg-green-100 text-green-700 border-green-200' : 'bg-gray-100 text-gray-500 border-gray-200'}`}>
            UL {carrier.ul_configured ? '\u2713' : '\u2717'}
          </span>
        )}
        {carrier.active !== undefined && (
          <span className={`rounded-lg px-1.5 py-0.5 text-[9px] font-bold border shadow-sm ${carrier.active ? 'bg-green-100 text-green-700 border-green-200' : 'bg-gray-100 text-gray-500 border-gray-200'}`}>
            {carrier.active ? 'Active' : 'Idle'}
          </span>
        )}
        <span className="ml-auto text-xs text-gray-500">PCI {carrier.pci}</span>
      </div>

      <div className="grid grid-cols-2 gap-x-4 gap-y-2 px-4 py-3">
        <div>
          <Tooltip text={rsrpTip(carrier.rsrp)}>
            <p className="text-[9px] font-bold text-gray-500 uppercase tracking-widest underline decoration-dotted decoration-gray-400 underline-offset-2">RSRP</p>
          </Tooltip>
          <p className={`text-2xl font-bold ${rsrpColor(carrier.rsrp)}`}>{fmt(carrier.rsrp, ' dBm')}</p>
        </div>
        <div>
          <Tooltip text={rsrqTip(carrier.rsrq)}>
            <p className="text-[9px] font-bold text-gray-500 uppercase tracking-widest underline decoration-dotted decoration-gray-400 underline-offset-2">RSRQ</p>
          </Tooltip>
          <p className={`text-2xl font-bold ${rsrqColor(carrier.rsrq)}`}>{fmt(carrier.rsrq, ' dB')}</p>
        </div>
        <div>
          <Tooltip text={sinrTip(carrier.sinr)}>
            <p className="text-[9px] font-bold text-gray-500 uppercase tracking-widest underline decoration-dotted decoration-gray-400 underline-offset-2">SINR</p>
          </Tooltip>
          <p className={`text-2xl font-bold ${sinrColor(carrier.sinr)}`}>{fmt(carrier.sinr, ' dB')}</p>
        </div>
        <div>
          <Tooltip text={rssiTip()}>
            <p className="text-[9px] font-bold text-gray-500 uppercase tracking-widest underline decoration-dotted decoration-gray-400 underline-offset-2">RSSI</p>
          </Tooltip>
          <p className="text-2xl font-bold text-gray-600">{fmt(carrier.rssi, ' dBm')}</p>
        </div>
      </div>

      <div className="border-t border-gray-200/60 px-4 py-2.5">
        <div className="grid grid-cols-2 gap-x-4 gap-y-1.5 text-sm">
          <div className="flex justify-between">
            <span className="text-gray-500">BW</span>
            <span className="text-gray-600 font-medium">{carrier.bandwidth}</span>
          </div>
          <div className="flex justify-between">
            <span className="text-gray-500">{tech === 'NR' ? 'ARFCN' : 'EARFCN'}</span>
            <span className="font-mono text-gray-600">{carrier.earfcn || '\u2014'}</span>
          </div>
          <div className="flex justify-between">
            <span className="text-gray-500">Freq</span>
            <span className="text-gray-600">{fmtFreq(carrier.freq)}</span>
          </div>
          <div className="flex justify-between">
            <span className="text-gray-500">PCI</span>
            <span className="font-mono text-gray-600">{carrier.pci}</span>
          </div>
        </div>
      </div>
    </div>
  )
}

function CellInfoTable({ carriers, tech }: { carriers: CarrierComponent[]; tech: 'NR' | 'LTE' }) {
  if (carriers.length === 0) return null
  const isNR = tech === 'NR'
  return (
    <div className={isNR ? 'mb-3' : ''}>
      <p className={`mb-2 text-[9px] font-bold uppercase tracking-widest ${isNR ? 'text-purple-600' : 'text-slds-blue'}`}>{isNR ? 'NR 5G' : 'LTE'} Carriers</p>
      <div className="relative">
      <div className="overflow-x-auto">
        <table className="w-full text-left text-sm">
          <thead>
            <tr className="border-b border-gray-200/60 text-gray-500">
              <th className="pb-1.5 pr-3">Type</th>
              <th className="pb-1.5 pr-3">Band</th>
              <th className="pb-1.5 pr-3">PCI</th>
              <th className="pb-1.5 pr-3">{isNR ? 'ARFCN' : 'EARFCN'}</th>
              <th className="pb-1.5 pr-3">BW</th>
              <th className="hidden sm:table-cell pb-1.5 pr-3">Freq</th>
              <th className="pb-1.5 pr-3">RSRP</th>
              <th className="pb-1.5 pr-3">RSRQ</th>
              <th className="pb-1.5 pr-3">SINR</th>
              <th className="hidden sm:table-cell pb-1.5">RSSI</th>
            </tr>
          </thead>
          <tbody>
            {carriers.map((c, i) => (
              <tr key={i} className="border-b border-gray-100/60 last:border-0 hover:bg-gray-50/60 transition-colors">
                <td className="py-1.5 pr-3">
                  <span className={`rounded-lg px-2 py-0.5 text-[9px] font-bold border shadow-sm ${c.label === 'PCC' ? 'bg-slds-blue/10 text-slds-blue border-slds-blue/30' : 'bg-gray-50 text-gray-600 border-gray-200'}`}>
                    {c.label}
                  </span>
                </td>
                <td className={`py-1.5 pr-3 font-medium ${isNR ? 'text-purple-600' : 'text-slds-blue'}`}>{c.band}</td>
                <td className="py-1.5 pr-3 text-gray-900">{c.pci}</td>
                <td className="py-1.5 pr-3 text-gray-900">{c.earfcn}</td>
                <td className="py-1.5 pr-3 text-gray-600">{c.bandwidth}</td>
                <td className="hidden sm:table-cell py-1.5 pr-3 text-gray-600">{c.freq ? `${c.freq.toFixed(1)} MHz` : '\u2014'}</td>
                <td className={`py-1.5 pr-3 font-medium ${rsrpColor(c.rsrp)}`}>{c.rsrp ?? '\u2014'}</td>
                <td className={`py-1.5 pr-3 font-medium ${rsrqColor(c.rsrq)}`}>{c.rsrq ?? '\u2014'}</td>
                <td className={`py-1.5 pr-3 font-medium ${sinrColor(c.sinr)}`}>{c.sinr ?? '\u2014'}</td>
                <td className="hidden sm:table-cell py-1.5 font-medium text-gray-600">{c.rssi ?? '\u2014'}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="pointer-events-none absolute inset-y-0 right-0 w-6 bg-gradient-to-l from-white to-transparent sm:hidden" />
      </div>
    </div>
  )
}

function SignalBars({ bars }: { bars?: number }) {
  const n = bars ?? 0
  const color = n >= 4 ? '#22c55e' : n >= 2 ? '#eab308' : '#ef4444'
  const heights = [4, 7, 10, 13, 16]
  return (
    <div className="flex items-end gap-0.5">
      {heights.map((h, i) => (
        <div key={i} className="w-2.5 rounded-sm transition-colors"
          style={{ height: `${h}px`, backgroundColor: i < n ? color : 'rgba(148,163,184,0.2)' }} />
      ))}
    </div>
  )
}

export default function SignalPage() {
  const [current, setCurrent] = useState<SignalInfo | null>(null)

  const fetchSignal = useCallback(async () => {
    try { setCurrent(await api.signal()) } catch { /* ignore */ }
  }, [])

  useEffect(() => {
    fetchSignal()
    const id = setInterval(fetchSignal, 3000)
    return () => clearInterval(id)
  }, [fetchSignal])

  if (!current) return <div className="text-gray-600 text-sm">Loading...</div>

  const hasNR = current.nr_carriers.length > 0
  const hasLTE = current.lte_carriers.length > 0
  const totalCarriers = current.lte_carriers.length + current.nr_carriers.length
  const nrBw = sumBandwidthMHz(current.nr_carriers)
  const lteBw = sumBandwidthMHz(current.lte_carriers)
  const totalBw = nrBw + lteBw

  return (
    <div className="space-y-5">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl sm:text-3xl font-bold text-gray-900">Signal Monitor</h1>
        <SignalBars bars={current.signal_bars} />
      </div>

      <Card>
        <div className="flex flex-wrap items-center gap-x-6 gap-y-2">
          <div>
            <p className="text-[9px] font-bold text-gray-500 uppercase tracking-widest">Connection</p>
            <p className="text-sm font-bold text-gray-900">{current.type ?? '\u2014'}</p>
          </div>
          <div>
            <p className="text-[9px] font-bold text-gray-500 uppercase tracking-widest">Provider</p>
            <p className="text-sm font-medium text-gray-900">{current.carrier ?? '\u2014'}</p>
          </div>
          <div>
            <p className="text-[9px] font-bold text-gray-500 uppercase tracking-widest">Cell ID</p>
            <p className="text-sm font-mono text-gray-600">{current.cell_id ?? '\u2014'}</p>
          </div>
          <div>
            <p className="text-[9px] font-bold text-gray-500 uppercase tracking-widest">Carriers</p>
            <p className="text-sm text-gray-600">
              {hasNR ? `${current.nr_carriers.length} NR` : ''}
              {hasNR && hasLTE ? ' + ' : ''}
              {hasLTE ? `${current.lte_carriers.length} LTE` : ''}
              {' '}({totalCarriers} total)
            </p>
          </div>
          <div>
            <p className="text-[9px] font-bold text-gray-500 uppercase tracking-widest">Bandwidth</p>
            <p className="text-sm font-bold text-gray-900">{formatBandwidthMHz(totalBw)}</p>
            <p className="text-[10px] text-gray-500">
              {hasNR ? `NR ${formatBandwidthMHz(nrBw)}` : ''}{hasNR && hasLTE ? ' + ' : ''}{hasLTE ? `LTE ${formatBandwidthMHz(lteBw)}` : ''}
            </p>
          </div>
          <div>
            <p className="text-[9px] font-bold text-gray-500 uppercase tracking-widest">Bands</p>
            <div className="flex flex-wrap gap-1 mt-0.5">
              {current.nr_carriers.map((c, i) => (
                <span key={`nr-${i}`} className="rounded-lg bg-purple-100 px-1.5 py-0.5 text-[9px] font-bold text-purple-700 border border-purple-200 shadow-sm">{c.band}</span>
              ))}
              {current.lte_carriers.map((c, i) => (
                <span key={`lte-${i}`} className="rounded-lg bg-slds-blue/10 px-1.5 py-0.5 text-[9px] font-bold text-slds-blueHover border border-slds-blue/30 shadow-sm">{c.band}</span>
              ))}
            </div>
          </div>
        </div>
      </Card>

      {(hasNR || hasLTE) && (
        <Card title="Current Cell Info">
          <CellInfoTable carriers={current.nr_carriers} tech="NR" />
          <CellInfoTable carriers={current.lte_carriers} tech="LTE" />
        </Card>
      )}

      {hasNR && (
        <div>
          <div className="mb-3 flex items-center gap-2">
            <h2 className="text-sm font-bold text-purple-600">5G NR</h2>
            <span className="rounded-lg bg-purple-100 px-2 py-0.5 text-[9px] font-bold text-purple-700 border border-purple-200 shadow-sm">
              {current.nr_carriers.length} carrier{current.nr_carriers.length !== 1 ? 's' : ''}
            </span>
            <span className="rounded-lg bg-purple-50 px-2 py-0.5 text-[9px] font-bold text-purple-600 border border-purple-200">
              {formatBandwidthMHz(nrBw)}
            </span>
          </div>
          <div className="grid grid-cols-1 gap-3 md:grid-cols-2 lg:grid-cols-3">
            {current.nr_carriers.map((c, i) => (
              <CarrierCard key={`nr-${i}`} carrier={c} tech="NR" />
            ))}
          </div>
        </div>
      )}

      {hasLTE && (
        <div>
          <div className="mb-3 flex items-center gap-2">
            <h2 className="text-sm font-bold text-slds-blue">LTE</h2>
            <span className="rounded-lg bg-slds-blue/10 px-2 py-0.5 text-[9px] font-bold text-slds-blueHover border border-slds-blue/30 shadow-sm">
              {current.lte_carriers.length} carrier{current.lte_carriers.length !== 1 ? 's' : ''}
              {current.lte_carriers.length > 1 ? ' (CA)' : ''}
            </span>
            <span className="rounded-lg bg-slds-blue/10 px-2 py-0.5 text-[9px] font-bold text-slds-blue border border-slds-blue/30">
              {formatBandwidthMHz(lteBw)}
            </span>
          </div>
          <div className="grid grid-cols-1 gap-3 md:grid-cols-2 lg:grid-cols-3">
            {current.lte_carriers.map((c, i) => (
              <CarrierCard key={`lte-${i}`} carrier={c} tech="LTE" />
            ))}
          </div>
        </div>
      )}

      <Card title="Signal Quality Reference">
        <div className="grid grid-cols-1 gap-4 text-sm md:grid-cols-3">
          <div>
            <p className="mb-1.5 font-bold text-gray-900">RSRP (dBm)</p>
            <div className="space-y-0.5">
              <div className="flex justify-between"><span className="text-green-500">Excellent</span><span className="text-gray-500">&gt; -80</span></div>
              <div className="flex justify-between"><span className="text-lime-500">Good</span><span className="text-gray-500">-80 to -90</span></div>
              <div className="flex justify-between"><span className="text-yellow-500">Fair</span><span className="text-gray-500">-90 to -100</span></div>
              <div className="flex justify-between"><span className="text-orange-500">Poor</span><span className="text-gray-500">-100 to -110</span></div>
              <div className="flex justify-between"><span className="text-red-500">No signal</span><span className="text-gray-500">&lt; -110</span></div>
            </div>
          </div>
          <div>
            <p className="mb-1.5 font-bold text-gray-900">RSRQ (dB)</p>
            <div className="space-y-0.5">
              <div className="flex justify-between"><span className="text-green-500">Good</span><span className="text-gray-500">&gt; -10</span></div>
              <div className="flex justify-between"><span className="text-yellow-500">Fair</span><span className="text-gray-500">-10 to -15</span></div>
              <div className="flex justify-between"><span className="text-red-500">Poor</span><span className="text-gray-500">&lt; -15</span></div>
            </div>
          </div>
          <div>
            <p className="mb-1.5 font-bold text-gray-900">SINR (dB)</p>
            <div className="space-y-0.5">
              <div className="flex justify-between"><span className="text-green-500">Excellent</span><span className="text-gray-500">&gt; 20</span></div>
              <div className="flex justify-between"><span className="text-lime-500">Good</span><span className="text-gray-500">10 to 20</span></div>
              <div className="flex justify-between"><span className="text-yellow-500">Fair</span><span className="text-gray-500">0 to 10</span></div>
              <div className="flex justify-between"><span className="text-red-500">Poor</span><span className="text-gray-500">&lt; 0</span></div>
            </div>
          </div>
        </div>
      </Card>
    </div>
  )
}
