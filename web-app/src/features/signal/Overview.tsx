import { useHome } from '../../app/HomeContext'
import { signalLegend, toneTextClass } from '../../data/signalQuality'
import type { SignalMetric } from '../../data/signalQuality'
import { formatBandwidthMHz, modemMode } from '../../format'
import type { CarrierComponent } from '../../types'
import { Card, Chip, SignalBars, Skeleton, Unavailable } from '../../ui/primitives'
import { MetricValue } from './MetricValue'
import { Tip } from './Tip'
import { METRIC_HELP, METRIC_LABEL, RATING_NOTE, bandwidthSummary, barsText, carrierCounts, servingView } from './telemetryView'

// ── Carrier table (desktop) / cards (mobile) ──────────────────────────────────

const METRICS: SignalMetric[] = ['rsrp', 'rsrq', 'sinr', 'rssi']

function CarrierStatus({ carrier, empty = null }: { carrier: CarrierComponent; empty?: React.ReactNode }) {
  if (carrier.ul_configured === undefined && carrier.active === undefined) return empty

  return (
    <span className="flex flex-wrap gap-1">
      {carrier.ul_configured !== undefined && (
        <Chip tone={carrier.ul_configured ? 'ok' : 'default'}>UL {carrier.ul_configured ? '✓' : '✗'}</Chip>
      )}
      {carrier.active !== undefined && (
        <Chip tone={carrier.active ? 'ok' : 'default'}>{carrier.active ? 'Active' : 'Idle'}</Chip>
      )}
    </span>
  )
}

function Pci({ pci }: { pci?: number }) {
  // PCI 0 is a real value; only an absent PCI is unavailable.
  return pci === undefined ? <Unavailable /> : <>{pci}</>
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
              {METRICS.map((m, i) => (
                <th key={m} className={`pb-1.5 font-semibold ${i < METRICS.length - 1 ? 'pr-3' : ''}`}>
                  <Tip text={METRIC_HELP[m]}>{METRIC_LABEL[m]}</Tip>
                </th>
              ))}
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
                    <CarrierStatus carrier={c} empty={'—'} />
                  </td>
                  <td className="tnum font-mono py-1.5 pr-3 text-ink"><Pci pci={c.pci} /></td>
                  <td className="tnum font-mono py-1.5 pr-3 text-ink">{c.earfcn}</td>
                  <td className="tnum font-mono py-1.5 pr-3 text-ink2">{c.bandwidth}</td>
                  <td className="tnum font-mono py-1.5 pr-3 text-ink2">
                    {c.freq ? `${c.freq.toFixed(1)} MHz` : '—'}
                  </td>
                  {METRICS.map((m, k) => (
                    <td key={m} className={`py-1.5 align-top font-medium ${k < METRICS.length - 1 ? 'pr-3' : ''}`}>
                      <MetricValue metric={m} value={c[m]} />
                    </td>
                  ))}
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>

      {/* Mobile cards: the same readings, ratings and help as the table */}
      <div className="space-y-2 sm:hidden">
        {sorted.map((c, i) => {
          const isPcc = c.label === 'PCC'
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
                {METRICS.map((m) => (
                  <div key={m} className="min-w-0">
                    <p>
                      <Tip text={METRIC_HELP[m]} className="label">
                        {METRIC_LABEL[m]}
                      </Tip>
                    </p>
                    <p className="text-sm font-bold">
                      <MetricValue metric={m} value={c[m]} />
                    </p>
                  </div>
                ))}
              </div>
              <div className="tnum font-mono mt-2 flex flex-wrap gap-x-4 gap-y-1 border-t border-line/8 pt-2 text-caption text-ink3">
                <span>
                  PCI <Pci pci={c.pci} />
                </span>
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

// ── Legend (generated from the policy) ────────────────────────────────────────

const LEGEND: { metric: SignalMetric; title: string }[] = [
  { metric: 'rsrp', title: 'RSRP (dBm)' },
  { metric: 'rsrq', title: 'RSRQ (dB)' },
  { metric: 'sinr', title: 'SINR (dB)' },
]

function Legend() {
  return (
    <Card title="Signal quality reference">
      <div className="grid grid-cols-1 gap-4 text-body md:grid-cols-3">
        {LEGEND.map(({ metric, title }) => (
          <div key={metric} data-legend={metric}>
            <p className="mb-1.5 font-semibold text-ink">{title}</p>
            <ul className="space-y-0.5 text-ink2">
              {signalLegend(metric).map((row) => (
                <li key={row.level} data-level={row.level} className="flex justify-between gap-3">
                  <span className={toneTextClass(row.tone)}>{row.label}</span>
                  <span className="tnum font-mono text-ink3">{row.range}</span>
                </li>
              ))}
            </ul>
          </div>
        ))}
      </div>
      <p className="mt-3 text-meta text-ink3">{RATING_NOTE}</p>
    </Card>
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
  const nr = carrierCounts(data.nr_carriers)
  const lte = carrierCounts(data.lte_carriers)
  const bw = bandwidthSummary([...data.nr_carriers, ...data.lte_carriers])
  const nrBw = bandwidthSummary(data.nr_carriers)
  const lteBw = bandwidthSummary(data.lte_carriers)
  const serving = servingView(data)
  const bars = barsText(data.signal_bars)
  const activeParts = [
    hasNR && `${nr.active} NR active${nr.idle ? `, ${nr.idle} idle` : ''}`,
    hasLTE && `${lte.active} LTE active${lte.idle ? `, ${lte.idle} idle` : ''}`,
  ].filter(Boolean)

  return (
    <div className="space-y-3">
      <Card>
        <div className="flex flex-wrap items-center gap-x-6 gap-y-3">
          <div>
            <p className="label">Connection</p>
            <p className="mt-0.5 text-sm font-bold text-ink">{data.type ?? '—'}</p>
          </div>
          <div>
            <p className="label">Serving cell</p>
            <p data-testid="serving-cell" className="mt-0.5 text-sm font-bold text-ink">
              {serving.label ?? <span className="text-ink3"><Unavailable label="No serving carrier reported" /></span>}
            </p>
          </div>
          <div>
            <p className="label">Provider</p>
            <p className="mt-0.5 text-sm font-medium text-ink">{data.carrier ?? '—'}</p>
          </div>
          <div>
            <p className="label">Cell ID</p>
            <p className="tnum mt-0.5 min-w-0 break-all font-mono text-body text-ink2">{data.cell_id ?? '—'}</p>
          </div>
          <div>
            <p className="label">Carriers reported</p>
            <p className="mt-0.5 text-sm text-ink2">
              {hasNR ? `${nr.reported} NR` : ''}
              {hasNR && hasLTE ? ' + ' : ''}
              {hasLTE ? `${lte.reported} LTE` : ''}
              {!hasNR && !hasLTE ? '—' : ''}
            </p>
            {activeParts.length > 0 && <p className="text-caption text-ink3">{activeParts.join(' · ')}</p>}
          </div>
          <div>
            <p className="label">Reported bandwidth</p>
            <p className="tnum font-mono mt-0.5 text-sm font-bold text-ink">{formatBandwidthMHz(bw.reportedMHz)}</p>
            <p className="text-caption text-ink3">Sum of all reported carriers</p>
            {hasNR && hasLTE && (
              <p className="tnum font-mono text-caption text-ink3">
                NR {formatBandwidthMHz(nrBw.reportedMHz)} + LTE {formatBandwidthMHz(lteBw.reportedMHz)}
              </p>
            )}
            {bw.hasIdle && (
              <p className="tnum font-mono text-caption text-ink3">Active only {formatBandwidthMHz(bw.activeMHz)}</p>
            )}
          </div>
          {home.bearer && (
            <>
              <div>
                <p className="label">{modemMode(data.type) === 'SA' ? '5QI' : 'QCI'}</p>
                <p className="tnum font-mono mt-0.5 text-sm font-bold text-ink">{home.bearer.qci ?? '—'}</p>
              </div>
              <div>
                <p className="label">AMBR</p>
                <p className="tnum font-mono mt-0.5 text-sm text-ink2">
                  {home.bearer.ambr_dl_mbps == null ? '—' : `${home.bearer.ambr_dl_mbps} / ${home.bearer.ambr_ul_mbps} Mbps`}
                </p>
                <p className="text-caption text-ink3">Operator rate cap, down / up</p>
              </div>
            </>
          )}
          <div className="ml-auto">
            {bars === null ? (
              <p className="text-caption text-ink3">
                Bars <Unavailable label="Signal bars unavailable" />
              </p>
            ) : (
              <SignalBars bars={data.signal_bars} large />
            )}
          </div>
        </div>
      </Card>

      {(hasNR || hasLTE) && (
        <Card title="Current cell info">
          <CarrierTable carriers={data.nr_carriers} tech="NR" />
          <CarrierTable carriers={data.lte_carriers} tech="LTE" />
        </Card>
      )}

      <Legend />
    </div>
  )
}
