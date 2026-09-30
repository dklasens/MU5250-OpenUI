import { useHome } from '../../app/HomeContext'
import { formatBandwidthMHz, formatBytes, formatSpeed, formatUptime, modemMode, qualityLabel, qualityText, rsrpQuality, sumBandwidthMHz } from '../../format'
import { IBolt, IDownload, IUpload } from '../../icons'
import { Card, Chip, Meter, Row, SignalBars, Skeleton } from '../../ui/primitives'

function PageSkeleton() {
  return (
    <div className="space-y-4">
      <Skeleton className="h-8 w-48" />
      <Skeleton className="h-64 xl:h-44" />
      <div className="grid grid-cols-1 gap-3 lg:grid-cols-3">
        <Skeleton className="h-48" />
        <Skeleton className="h-48" />
        <Skeleton className="h-48" />
      </div>
    </div>
  )
}

export default function HomePage() {
  const { data, error } = useHome()

  if (!data && !error) return <PageSkeleton />

  const signal = data?.signal ?? null
  const battery = data?.battery ?? null
  const speed = data?.speed ?? null
  const device = data?.device ?? null
  const wan = data?.wan ?? null
  const wan6 = data?.wan6 ?? null
  const cpu = data?.cpu ?? null
  const mem = data?.memory ?? null
  const usage = data?.usage ?? null

  const primary = signal?.lte_carriers?.[0] || signal?.nr_carriers?.[0]
  const pccRsrp = primary?.rsrp ?? signal?.rsrp
  const quality = rsrpQuality(pccRsrp)
  const mode = modemMode(signal?.type)
  const lteBw = signal ? sumBandwidthMHz(signal.lte_carriers) : 0
  const nrBw = signal ? sumBandwidthMHz(signal.nr_carriers) : 0
  const totalBw = lteBw + nrBw
  const carrierCount = (signal?.lte_carriers.length ?? 0) + (signal?.nr_carriers.length ?? 0)

  return (
    <div className="space-y-4">
      <div>
        <h1 className="hidden font-display text-2xl font-semibold tracking-[-0.015em] text-ink lg:block">Overview</h1>
        <p className="lg:mt-0.5 text-body text-ink2">{signal?.carrier ?? 'Mobile broadband status'}</p>
      </div>

      {error && !data && (
        <Card>
          <p className="text-body text-danger">{error}</p>
        </Card>
      )}

      {/* Readout band — the page's one graphite beat. Hairline grid via gap-px on a line-tinted ground. */}
      <section className="band overflow-hidden rounded-panel border border-line/10" aria-label="Live readouts">
        <div className="grid grid-cols-6 gap-px bg-line/10 xl:grid-cols-5">
          <div className="col-span-6 bg-band p-4 xl:col-span-2">
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0">
                <p className="label">Signal · RSRP</p>
                <div className="mt-2 flex items-baseline gap-2">
                  <span className={`tnum font-mono text-5xl font-medium leading-none tracking-[-0.03em] ${qualityText(quality)}`}>
                    {pccRsrp != null ? pccRsrp : '\u2014'}
                  </span>
                  <span className="font-display text-sm font-medium text-ink2">dBm</span>
                </div>
                <p className={`mt-1.5 text-body font-semibold ${qualityText(quality)}`}>{qualityLabel(quality)}</p>
              </div>
              <SignalBars bars={signal?.signal_bars} large />
            </div>
            <div className="tnum mt-4 flex flex-wrap items-center gap-x-3 gap-y-1.5 font-mono text-meta text-ink2">
              {primary?.band && <Chip tone={primary.band.startsWith('n') ? 'nr' : 'lte'}>{primary.band}</Chip>}
              {primary?.pci != null && primary.pci > 0 && <span>PCI {primary.pci}</span>}
              <span>{signal?.signal_bars ?? 0}/5 bars</span>
            </div>
          </div>

          <div className="col-span-6 bg-band p-4 sm:col-span-2 xl:col-span-1">
            <p className="label">Throughput</p>
            <div className="tnum mt-2 flex gap-x-6 font-mono font-medium text-ink sm:block sm:space-y-1.5">
              <p className="flex items-center gap-1.5 text-base leading-none">
                <IDownload size={14} className="shrink-0 text-ok" />
                <span className="truncate">{speed ? formatSpeed(speed.rx_bps) : '\u2014'}</span>
              </p>
              <p className="flex items-center gap-1.5 text-base leading-none">
                <IUpload size={14} className="shrink-0 text-accent" />
                <span className="truncate">{speed ? formatSpeed(speed.tx_bps) : '\u2014'}</span>
              </p>
            </div>
            <p className="tnum mt-3 truncate font-mono text-caption text-ink3">
              Peak down {speed && speed.max_rx_bps > 0 ? formatSpeed(speed.max_rx_bps) : '\u2014'}
            </p>
          </div>

          <div className="col-span-3 bg-band p-4 sm:col-span-2 xl:col-span-1">
            <p className="label">Mode</p>
            <p className="tnum mt-2 font-mono text-2xl font-medium leading-none text-ink">{mode}</p>
            <p className="mt-1.5 text-meta text-ink2">
              {carrierCount} carrier{carrierCount !== 1 ? 's' : ''}
            </p>
            <div className="mt-3 flex flex-wrap gap-1">
              {nrBw > 0 && <Chip tone="nr">NR {formatBandwidthMHz(nrBw)}</Chip>}
              {lteBw > 0 && <Chip tone="lte">LTE {formatBandwidthMHz(lteBw)}</Chip>}
              {totalBw <= 0 && <span className="text-caption text-ink3">No bandwidth</span>}
            </div>
          </div>

          <div className="col-span-3 bg-band p-4 sm:col-span-2 xl:col-span-1">
            <p className="label flex items-center gap-1">
              Battery {battery?.charging && <IBolt size={12} className="text-warn" />}
            </p>
            <p className="tnum mt-2 font-mono text-2xl font-medium leading-none text-ink">
              {battery?.percent != null ? `${battery.percent}%` : '\u2014'}
            </p>
            <p className="mt-1.5 text-meta text-ink2">{battery?.charging ? 'Charging' : 'On battery'}</p>
            <p className="tnum mt-3 truncate font-mono text-caption text-ink3">
              {battery?.voltage_mv ? `${(battery.voltage_mv / 1000).toFixed(2)} V` : '\u2014'}
              {battery?.temperature_c != null ? ` · ${battery.temperature_c.toFixed(1)}°C` : ''}
            </p>
          </div>
        </div>
      </section>

      {/* Radio details */}
      {signal && (signal.lte_carriers.length > 0 || signal.nr_carriers.length > 0) && (
        <Card title="Carriers">
          <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
            <div>
              <p className="label mb-1.5 text-accent">LTE</p>
              {signal.lte_carriers.length > 0 ? (
                <div className="flex flex-wrap gap-1.5">
                  {signal.lte_carriers.map((c, i) => (
                    <Chip key={i} tone="lte">
                      {c.label} · {c.band}
                      {c.rsrp != null ? ` · ${c.rsrp} dBm` : ''}
                    </Chip>
                  ))}
                </div>
              ) : (
                <p className="text-body text-ink3">No active LTE carrier</p>
              )}
            </div>
            <div>
              <p className="label mb-1.5 text-nr">
                5G NR
              </p>
              {signal.nr_carriers.length > 0 ? (
                <div className="flex flex-wrap gap-1.5">
                  {signal.nr_carriers.map((c, i) => (
                    <Chip key={i} tone="nr">
                      {c.label} · {c.band}
                      {c.rsrp != null ? ` · ${c.rsrp} dBm` : ''}
                    </Chip>
                  ))}
                </div>
              ) : (
                <p className="text-body text-ink3">No active NR carrier</p>
              )}
            </div>
          </div>
        </Card>
      )}

      {/* Details row */}
      <div className="grid grid-cols-1 gap-3 lg:grid-cols-3">
        <Card title="Connection">
          <Row label="Operator" value={signal?.carrier ?? '\u2014'} />
          <Row label="IPv4" value={wan?.ipv4 ?? '\u2014'} mono />
          <Row label="Gateway" value={wan?.gateway ?? '\u2014'} mono />
          <Row label="IPv6" value={wan6?.ipv6 ?? '\u2014'} mono wrap />
          {wan6?.prefix && <Row label="IPv6 prefix" value={wan6.prefix} mono wrap />}
          {wan?.dns && wan.dns.length > 0 && (
            <Row label="DNS" value={wan.dns.filter((d) => !d.includes(':')).join(', ') || '\u2014'} mono wrap />
          )}
        </Card>

        <Card title="Device">
          <Row label="Model" value={device?.model ?? '\u2014'} />
          <Row label="Firmware" value={device?.firmware ?? '\u2014'} />
          <Row label="Uptime" value={formatUptime(device?.uptime_secs)} />
          <div className="mt-2 space-y-2 border-t border-line/8 pt-2.5">
            <div>
              <div className="mb-1 flex justify-between text-caption">
                <span className="font-medium text-ink2">CPU</span>
                <span className="tnum font-mono text-ink2">{cpu ? `${cpu.overall.toFixed(0)}%` : '\u2014'}</span>
              </div>
              <Meter pct={cpu?.overall ?? 0} />
            </div>
            <div>
              <div className="mb-1 flex justify-between text-caption">
                <span className="font-medium text-ink2">Memory</span>
                <span className="tnum font-mono text-ink2">{mem ? `${mem.usage_pct.toFixed(0)}%` : '\u2014'}</span>
              </div>
              <Meter pct={mem?.usage_pct ?? 0} tone="bg-warn" />
            </div>
          </div>
        </Card>

        <Card title="Data usage">
          {usage ? (
            <div className="space-y-2.5">
              {[
                { label: 'Today', period: usage.day },
                { label: 'This month', period: usage.month },
                { label: 'Total', period: usage.total },
              ].map(({ label, period }) => (
                <div key={label}>
                  <p className="label">{label}</p>
                  <div className="tnum font-mono mt-0.5 flex gap-3 text-body font-medium">
                    <span className="flex items-center gap-1 text-ok">
                      <IDownload size={12} /> {formatBytes(period.rx_bytes)}
                    </span>
                    <span className="flex items-center gap-1 text-accent">
                      <IUpload size={12} /> {formatBytes(period.tx_bytes)}
                    </span>
                  </div>
                </div>
              ))}
            </div>
          ) : (
            <p className="text-body text-ink3">Not available</p>
          )}
        </Card>
      </div>
    </div>
  )
}
