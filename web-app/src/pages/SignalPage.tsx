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

function CellInfoTable({ carriers, tech }: { carriers: CarrierComponent[]; tech: 'NR' | 'LTE' }) {
  if (carriers.length === 0) return null
  const isNR = tech === 'NR'
  // Promote PCC: sort PCC first, then SCCs in original order.
  const sorted = [...carriers].sort((a, b) => (a.label === 'PCC' ? -1 : b.label === 'PCC' ? 1 : 0))
  return (
    <div className={isNR ? 'mb-4' : ''}>
      <p className={`mb-2 text-[9px] font-bold uppercase tracking-widest ${isNR ? 'text-purple-600' : 'text-slds-blue'}`}>{isNR ? 'NR 5G' : 'LTE'} Carriers</p>

      <div className="hidden sm:block overflow-x-auto">
        <table className="w-full text-left text-sm">
          <thead>
            <tr className="border-b border-gray-200/60 text-gray-500">
              <th className="pb-1.5 pr-3">Type</th>
              <th className="pb-1.5 pr-3">Status</th>
              <th className="pb-1.5 pr-3">Band</th>
              <th className="pb-1.5 pr-3">PCI</th>
              <th className="pb-1.5 pr-3">{isNR ? 'ARFCN' : 'EARFCN'}</th>
              <th className="pb-1.5 pr-3">BW</th>
              <th className="hidden sm:table-cell pb-1.5 pr-3">Freq</th>
              <th className="pb-1.5 pr-3"><Tooltip text={rsrpTip()}><span className="underline decoration-dotted decoration-gray-400 underline-offset-2">RSRP</span></Tooltip></th>
              <th className="pb-1.5 pr-3"><Tooltip text={rsrqTip()}><span className="underline decoration-dotted decoration-gray-400 underline-offset-2">RSRQ</span></Tooltip></th>
              <th className="pb-1.5 pr-3"><Tooltip text={sinrTip()}><span className="underline decoration-dotted decoration-gray-400 underline-offset-2">SINR</span></Tooltip></th>
              <th className="hidden sm:table-cell pb-1.5"><Tooltip text={rssiTip()}><span className="underline decoration-dotted decoration-gray-400 underline-offset-2">RSSI</span></Tooltip></th>
            </tr>
          </thead>
          <tbody>
            {sorted.map((c, i) => {
              const isPcc = c.label === 'PCC'
              const rowBg = isPcc
                ? (isNR ? 'bg-purple-50/60' : 'bg-slds-blue/5')
                : ''
              const accent = isPcc
                ? (isNR ? 'border-l-2 border-purple-400' : 'border-l-2 border-slds-blue')
                : 'border-l-2 border-transparent'
              return (
                <tr key={i} className={`border-b border-gray-100/60 last:border-0 hover:bg-gray-50/60 transition-colors ${rowBg} ${accent}`}>
                  <td className="py-1.5 pl-2 pr-3">
                    <span className={`rounded-lg px-2 py-0.5 text-[9px] font-bold border shadow-sm ${isPcc
                      ? (isNR ? 'bg-purple-100 text-purple-700 border-purple-200' : 'bg-slds-blue/10 text-slds-blue border-slds-blue/30')
                      : 'bg-gray-50 text-gray-600 border-gray-200'}`}>
                      {c.label}
                    </span>
                  </td>
                  <td className="py-1.5 pr-3">
                    <div className="flex flex-wrap gap-1">
                      {c.ul_configured !== undefined && (
                        <span className={`rounded-md px-1.5 py-0.5 text-[9px] font-bold border ${c.ul_configured ? 'bg-green-100 text-green-700 border-green-200' : 'bg-gray-100 text-gray-500 border-gray-200'}`}>
                          UL {c.ul_configured ? '\u2713' : '\u2717'}
                        </span>
                      )}
                      {c.active !== undefined && (
                        <span className={`rounded-md px-1.5 py-0.5 text-[9px] font-bold border ${c.active ? 'bg-green-100 text-green-700 border-green-200' : 'bg-gray-100 text-gray-500 border-gray-200'}`}>
                          {c.active ? 'Active' : 'Idle'}
                        </span>
                      )}
                    </div>
                  </td>
                  <td className={`py-1.5 pr-3 ${isPcc ? 'font-bold' : 'font-medium'} ${isNR ? 'text-purple-600' : 'text-slds-blue'}`}>{c.band}</td>
                  <td className="py-1.5 pr-3 text-gray-900">{c.pci}</td>
                  <td className="py-1.5 pr-3 text-gray-900">{c.earfcn}</td>
                  <td className="py-1.5 pr-3 text-gray-600">{c.bandwidth}</td>
                  <td className="hidden sm:table-cell py-1.5 pr-3 text-gray-600">{c.freq ? `${c.freq.toFixed(1)} MHz` : '\u2014'}</td>
                  <td className={`py-1.5 pr-3 ${isPcc ? 'font-bold' : 'font-medium'} ${rsrpColor(c.rsrp)}`}>{c.rsrp ?? '\u2014'}</td>
                  <td className={`py-1.5 pr-3 ${isPcc ? 'font-bold' : 'font-medium'} ${rsrqColor(c.rsrq)}`}>{c.rsrq ?? '\u2014'}</td>
                  <td className={`py-1.5 pr-3 ${isPcc ? 'font-bold' : 'font-medium'} ${sinrColor(c.sinr)}`}>{c.sinr ?? '\u2014'}</td>
                  <td className="hidden sm:table-cell py-1.5 font-medium text-gray-600">{c.rssi ?? '\u2014'}</td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>

      <div className="space-y-2 sm:hidden">
        {sorted.map((c, i) => {
          const isPcc = c.label === 'PCC'
          const metrics = [
            { label: 'RSRP', value: c.rsrp, color: rsrpColor(c.rsrp) },
            { label: 'RSRQ', value: c.rsrq, color: rsrqColor(c.rsrq) },
            { label: 'SINR', value: c.sinr, color: sinrColor(c.sinr) },
            { label: 'RSSI', value: c.rssi, color: 'text-gray-600' },
          ]
          return (
            <div key={i} className={`rounded-xl border p-3 ${isPcc
              ? (isNR ? 'border-purple-200 bg-purple-50/60' : 'border-slds-blue/30 bg-slds-blue/5')
              : 'border-gray-200 bg-white'}`}>
              <div className="mb-2.5 flex items-center gap-2">
                <span className={`rounded-lg px-2 py-0.5 text-[9px] font-bold border shadow-sm ${isPcc
                  ? (isNR ? 'bg-purple-100 text-purple-700 border-purple-200' : 'bg-slds-blue/10 text-slds-blue border-slds-blue/30')
                  : 'bg-gray-50 text-gray-600 border-gray-200'}`}>
                  {c.label}
                </span>
                <span className={`text-base font-bold ${isNR ? 'text-purple-600' : 'text-slds-blue'}`}>{c.band}</span>
                <div className="ml-auto flex flex-wrap justify-end gap-1">
                  {c.ul_configured !== undefined && (
                    <span className={`rounded-md px-1.5 py-0.5 text-[9px] font-bold border ${c.ul_configured ? 'bg-green-100 text-green-700 border-green-200' : 'bg-gray-100 text-gray-500 border-gray-200'}`}>
                      UL {c.ul_configured ? '✓' : '✗'}
                    </span>
                  )}
                  {c.active !== undefined && (
                    <span className={`rounded-md px-1.5 py-0.5 text-[9px] font-bold border ${c.active ? 'bg-green-100 text-green-700 border-green-200' : 'bg-gray-100 text-gray-500 border-gray-200'}`}>
                      {c.active ? 'Active' : 'Idle'}
                    </span>
                  )}
                </div>
              </div>
              <div className="grid grid-cols-4 gap-2">
                {metrics.map(m => (
                  <div key={m.label}>
                    <p className="text-[9px] font-bold uppercase tracking-widest text-gray-500">{m.label}</p>
                    <p className={`text-sm font-bold ${m.color}`}>{m.value ?? '—'}</p>
                  </div>
                ))}
              </div>
              <div className="mt-2.5 flex flex-wrap gap-x-4 gap-y-1 border-t border-gray-200/60 pt-2 text-[11px] text-gray-500">
                <span>PCI <span className="font-medium text-gray-700">{c.pci}</span></span>
                <span>{isNR ? 'ARFCN' : 'EARFCN'} <span className="font-medium text-gray-700">{c.earfcn}</span></span>
                <span>BW <span className="font-medium text-gray-700">{c.bandwidth}</span></span>
                {c.freq != null && <span>Freq <span className="font-medium text-gray-700">{c.freq.toFixed(1)} MHz</span></span>}
              </div>
            </div>
          )
        })}
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
              {hasNR && hasLTE ? ` (${totalCarriers} total)` : ''}
            </p>
          </div>
          <div>
            <p className="text-[9px] font-bold text-gray-500 uppercase tracking-widest">Bandwidth</p>
            <p className="text-sm font-bold text-gray-900">{formatBandwidthMHz(totalBw)}</p>
            {hasNR && hasLTE && (
              <p className="text-[10px] text-gray-500">
                NR {formatBandwidthMHz(nrBw)} + LTE {formatBandwidthMHz(lteBw)}
              </p>
            )}
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

      <details className="group bg-white rounded-2xl shadow-macos-lg border border-black/5">
        <summary className="flex cursor-pointer list-none items-center justify-between rounded-2xl px-4 py-4 sm:px-6 [&::-webkit-details-marker]:hidden">
          <h2 className="text-sm font-bold text-gray-900">Signal Quality Reference</h2>
          <svg className="h-4 w-4 text-gray-400 transition-transform group-open:rotate-180" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M19 9l-7 7-7-7" />
          </svg>
        </summary>
        <div className="border-t border-black/5 px-4 py-5 sm:px-6">
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
        </div>
      </details>
    </div>
  )
}
