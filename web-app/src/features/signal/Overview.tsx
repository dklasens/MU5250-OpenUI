import { useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { useHome } from '../../app/HomeContext'
import { formatBandwidthMHz, qualityText, rsrpQuality, rsrqColorClass, sinrColorClass, sumBandwidthMHz } from '../../format'
import type { CarrierComponent } from '../../types'
import { Card, Chip, SignalBars, Skeleton } from '../../ui/primitives'

// ── Tooltip ───────────────────────────────────────────────────────────────────

const TIP_WIDTH = 224

function Tip({ text, children }: { text: string; children: React.ReactNode }) {
  const ref = useRef<HTMLSpanElement>(null)
  const [pos, setPos] = useState<{ x: number; top: number; bottom: number } | null>(null)

  const measure = () => {
    const r = ref.current?.getBoundingClientRect()
    return r ? { x: r.left + r.width / 2, top: r.top, bottom: r.bottom } : null
  }

  const half = TIP_WIDTH / 2
  const left = pos ? Math.min(Math.max(pos.x, half + 8), window.innerWidth - half - 8) : 0
  const below = pos ? pos.top < 130 : false

  return (
    <span
      ref={ref}
      className="cursor-help"
      onMouseEnter={() => setPos(measure())}
      onMouseLeave={() => setPos(null)}
      onTouchStart={() => setPos((p) => (p ? null : measure()))}
    >
      {children}
      {pos &&
        createPortal(
          <span
            role="tooltip"
            className="fixed z-50 w-56 rounded-ctl border border-line/10 bg-surface px-2.5 py-1.5 text-caption leading-snug text-ink2 shadow-lg"
            style={
              below
                ? { left, top: pos.bottom + 8 }
                : { left, top: pos.top - 8, transform: 'translateY(-100%)' }
            }
          >
            {text}
          </span>,
          document.body,
        )}
    </span>
  )
}

const RSRP_TIP =
  'Reference Signal Received Power — power of a single LTE/NR reference signal. Primary indicator of signal strength.'
const RSRQ_TIP =
  'Reference Signal Received Quality — signal quality accounting for noise and interference from neighbouring cells.'
const SINR_TIP =
  'Signal to Interference plus Noise Ratio — how far the signal is above the noise floor. Key metric for achievable throughput.'
const RSSI_TIP =
  'Received Signal Strength Indicator — total wideband received power including signal, noise, and interference.'

// ── Carrier table (desktop) / cards (mobile) ──────────────────────────────────

function CarrierStatus({ carrier, empty = null }: { carrier: CarrierComponent; empty?: React.ReactNode }) {
  if (carrier.ul_configured === undefined && carrier.active === undefined) return empty

  return (
    <span className="flex flex-wrap gap-1">
      {carrier.ul_configured !== undefined && (
        <Chip tone={carrier.ul_configured ? 'ok' : 'default'}>UL {carrier.ul_configured ? '\u2713' : '\u2717'}</Chip>
      )}
      {carrier.active !== undefined && (
        <Chip tone={carrier.active ? 'ok' : 'default'}>{carrier.active ? 'Active' : 'Idle'}</Chip>
      )}
    </span>
  )
}

function CarrierTable({ carriers, tech }: { carriers: CarrierComponent[]; tech: 'NR' | 'LTE' }) {
  if (carriers.length === 0) return null
  const isNR = tech === 'NR'
  const sorted = [...carriers].sort((a, b) => (a.label === 'PCC' ? -1 : b.label === 'PCC' ? 1 : 0))
  const bandText = isNR ? 'text-nr' : 'text-accent'

  return (
    <div className={isNR ? 'mb-4' : ''}>
      <p className={`label mb-2 ${bandText}`}>
        {isNR ? 'NR 5G' : 'LTE'} carriers
      </p>

      {/* Desktop table */}
      <div className="hidden overflow-x-auto sm:block">
        <table className="w-full text-left text-body">
          <thead>
            <tr className="label border-b border-line/8">
              <th className="pb-1.5 pr-3 font-semibold">Type</th>
              <th className="pb-1.5 pr-3 font-semibold">Band</th>
              <th className="pb-1.5 pr-3 font-semibold">Status</th>
              <th className="pb-1.5 pr-3 font-semibold">PCI</th>
              <th className="pb-1.5 pr-3 font-semibold">{isNR ? 'ARFCN' : 'EARFCN'}</th>
              <th className="pb-1.5 pr-3 font-semibold">BW</th>
              <th className="pb-1.5 pr-3 font-semibold">Freq</th>
              <th className="pb-1.5 pr-3 font-semibold">
                <Tip text={RSRP_TIP}>
                  <span className="underline decoration-dotted underline-offset-2">RSRP</span>
                </Tip>
              </th>
              <th className="pb-1.5 pr-3 font-semibold">
                <Tip text={RSRQ_TIP}>
                  <span className="underline decoration-dotted underline-offset-2">RSRQ</span>
                </Tip>
              </th>
              <th className="pb-1.5 pr-3 font-semibold">
                <Tip text={SINR_TIP}>
                  <span className="underline decoration-dotted underline-offset-2">SINR</span>
                </Tip>
              </th>
              <th className="pb-1.5 font-semibold">
                <Tip text={RSSI_TIP}>
                  <span className="underline decoration-dotted underline-offset-2">RSSI</span>
                </Tip>
              </th>
            </tr>
          </thead>
          <tbody>
            {sorted.map((c, i) => {
              const isPcc = c.label === 'PCC'
              return (
                <tr
                  key={i}
                  className={`border-b border-line/6 last:border-0 ${isPcc ? 'bg-accent/4' : ''}`}
                >
                  <td className="py-1.5 pr-3">
                    <Chip tone={isPcc ? (isNR ? 'nr' : 'lte') : 'default'}>{c.label}</Chip>
                  </td>
                  <td className={`py-1.5 pr-3 font-semibold ${bandText}`}>{c.band}</td>
                  <td className="py-1.5 pr-3 text-ink3">
                    <CarrierStatus carrier={c} empty={'\u2014'} />
                  </td>
                  <td className="tnum font-mono py-1.5 pr-3 text-ink">{c.pci}</td>
                  <td className="tnum font-mono py-1.5 pr-3 text-ink">{c.earfcn}</td>
                  <td className="tnum font-mono py-1.5 pr-3 text-ink2">{c.bandwidth}</td>
                  <td className="tnum font-mono py-1.5 pr-3 text-ink2">
                    {c.freq ? `${c.freq.toFixed(1)} MHz` : '\u2014'}
                  </td>
                  <td className={`tnum font-mono py-1.5 pr-3 font-semibold ${qualityText(rsrpQuality(c.rsrp))}`}>
                    {c.rsrp ?? '\u2014'}
                  </td>
                  <td className={`tnum font-mono py-1.5 pr-3 font-medium ${rsrqColorClass(c.rsrq)}`}>
                    {c.rsrq ?? '\u2014'}
                  </td>
                  <td className={`tnum font-mono py-1.5 pr-3 font-medium ${sinrColorClass(c.sinr)}`}>
                    {c.sinr ?? '\u2014'}
                  </td>
                  <td className="tnum font-mono py-1.5 text-ink2">{c.rssi ?? '\u2014'}</td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>

      {/* Mobile cards */}
      <div className="space-y-2 sm:hidden">
        {sorted.map((c, i) => {
          const isPcc = c.label === 'PCC'
          const metrics = [
            { label: 'RSRP', value: c.rsrp, cls: qualityText(rsrpQuality(c.rsrp)) },
            { label: 'RSRQ', value: c.rsrq, cls: rsrqColorClass(c.rsrq) },
            { label: 'SINR', value: c.sinr, cls: sinrColorClass(c.sinr) },
            { label: 'RSSI', value: c.rssi, cls: 'text-ink2' },
          ]
          return (
            <div
              key={i}
              className={`rounded-ctl border p-3 ${isPcc ? (isNR ? 'border-nr/30' : 'border-accent/30') : 'border-line/8'}`}
            >
              <div className="mb-2 flex items-center gap-2">
                <Chip tone={isPcc ? (isNR ? 'nr' : 'lte') : 'default'}>{c.label}</Chip>
                <span className={`text-sm font-bold ${bandText}`}>{c.band}</span>
                <div className="ml-auto">
                  <CarrierStatus carrier={c} />
                </div>
              </div>
              <div className="grid grid-cols-4 gap-2">
                {metrics.map((m) => (
                  <div key={m.label}>
                    <p className="label">{m.label}</p>
                    <p className={`tnum font-mono text-sm font-bold ${m.cls}`}>{m.value ?? '\u2014'}</p>
                  </div>
                ))}
              </div>
              <div className="tnum font-mono mt-2 flex flex-wrap gap-x-4 gap-y-1 border-t border-line/8 pt-2 text-caption text-ink3">
                <span>PCI {c.pci}</span>
                <span>
                  {isNR ? 'ARFCN' : 'EARFCN'} {c.earfcn}
                </span>
                <span>BW {c.bandwidth}</span>
                {c.freq != null && <span>{c.freq.toFixed(1)} MHz</span>}
              </div>
            </div>
          )
        })}
      </div>
    </div>
  )
}

// ── Page ──────────────────────────────────────────────────────────────────────

export default function Overview() {
  // Served by the shared home poll — the batch already carries this exact
  // payload, so a second /api/network/signal poll was pure duplicate load.
  const { data: home } = useHome()

  if (!home) {
    return (
      <div className="space-y-3">
        <Skeleton className="h-24" />
        <Skeleton className="h-64" />
      </div>
    )
  }

  const data = home.signal
  if (!data) {
    return (
      <Card>
        <p className="text-body text-ink3">No radio data reported by the modem.</p>
      </Card>
    )
  }

  const hasNR = data.nr_carriers.length > 0
  const hasLTE = data.lte_carriers.length > 0
  const nrBw = sumBandwidthMHz(data.nr_carriers)
  const lteBw = sumBandwidthMHz(data.lte_carriers)
  const totalBw = nrBw + lteBw

  return (
    <div className="space-y-3">
      <Card>
        <div className="flex flex-wrap items-center gap-x-6 gap-y-3">
          <div>
            <p className="label">Connection</p>
            <p className="mt-0.5 text-sm font-bold text-ink">{data.type ?? '\u2014'}</p>
          </div>
          <div>
            <p className="label">Provider</p>
            <p className="mt-0.5 text-sm font-medium text-ink">{data.carrier ?? '\u2014'}</p>
          </div>
          <div>
            <p className="label">Cell ID</p>
            <p className="tnum mt-0.5 font-mono text-body text-ink2">{data.cell_id ?? '\u2014'}</p>
          </div>
          <div>
            <p className="label">Carriers</p>
            <p className="mt-0.5 text-sm text-ink2">
              {hasNR ? `${data.nr_carriers.length} NR` : ''}
              {hasNR && hasLTE ? ' + ' : ''}
              {hasLTE ? `${data.lte_carriers.length} LTE` : ''}
              {!hasNR && !hasLTE ? '\u2014' : ''}
            </p>
          </div>
          <div>
            <p className="label">Bandwidth</p>
            <p className="tnum font-mono mt-0.5 text-sm font-bold text-ink">{formatBandwidthMHz(totalBw)}</p>
            {hasNR && hasLTE && (
              <p className="tnum font-mono text-caption text-ink3">
                NR {formatBandwidthMHz(nrBw)} + LTE {formatBandwidthMHz(lteBw)}
              </p>
            )}
          </div>
          <div className="ml-auto">
            <SignalBars bars={data.signal_bars} large />
          </div>
        </div>
      </Card>

      {(hasNR || hasLTE) && (
        <Card title="Current cell info">
          <CarrierTable carriers={data.nr_carriers} tech="NR" />
          <CarrierTable carriers={data.lte_carriers} tech="LTE" />
        </Card>
      )}

      <Card title="Signal quality reference">
        <div className="grid grid-cols-1 gap-4 text-body md:grid-cols-3">
          <div>
            <p className="mb-1.5 font-semibold text-ink">RSRP (dBm)</p>
            <div className="space-y-0.5 text-ink2">
              <div className="flex justify-between"><span className="text-ok">Excellent</span><span className="tnum font-mono text-ink3">&gt; -80</span></div>
              <div className="flex justify-between"><span className="text-ok">Good</span><span className="tnum font-mono text-ink3">-80 to -90</span></div>
              <div className="flex justify-between"><span className="text-warn">Fair</span><span className="tnum font-mono text-ink3">-90 to -100</span></div>
              <div className="flex justify-between"><span className="text-danger">Poor</span><span className="tnum font-mono text-ink3">&lt; -100</span></div>
            </div>
          </div>
          <div>
            <p className="mb-1.5 font-semibold text-ink">RSRQ (dB)</p>
            <div className="space-y-0.5 text-ink2">
              <div className="flex justify-between"><span className="text-ok">Good</span><span className="tnum font-mono text-ink3">&gt; -10</span></div>
              <div className="flex justify-between"><span className="text-warn">Fair</span><span className="tnum font-mono text-ink3">-10 to -15</span></div>
              <div className="flex justify-between"><span className="text-danger">Poor</span><span className="tnum font-mono text-ink3">&lt; -15</span></div>
            </div>
          </div>
          <div>
            <p className="mb-1.5 font-semibold text-ink">SINR (dB)</p>
            <div className="space-y-0.5 text-ink2">
              <div className="flex justify-between"><span className="text-ok">Excellent</span><span className="tnum font-mono text-ink3">&gt; 20</span></div>
              <div className="flex justify-between"><span className="text-ok">Good</span><span className="tnum font-mono text-ink3">10 to 20</span></div>
              <div className="flex justify-between"><span className="text-warn">Fair</span><span className="tnum font-mono text-ink3">0 to 10</span></div>
              <div className="flex justify-between"><span className="text-danger">Poor</span><span className="tnum font-mono text-ink3">&lt; 0</span></div>
            </div>
          </div>
        </div>
      </Card>
    </div>
  )
}
