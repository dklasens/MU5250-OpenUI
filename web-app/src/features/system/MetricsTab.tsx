import { useState } from 'react'
import { useFastHeartbeat, useHome } from '../../app/HomeContext'
import { api } from '../../data/api'
import { usePoll, type PollResult } from '../../data/poll'
import { formatBytes, tempColorClass } from '../../format'
import type { BatteryBspInfo, BatteryDetail, ChargeControlState, ThermalAll } from '../../types'
import { Button, Field, Toggle } from '../../ui/controls'
import { Card, InlineStatus, Loading, Meter, Skeleton } from '../../ui/primitives'
import { LIMIT_MAX, LIMIT_MIN, LIMIT_STEP, limitToApply } from './chargeLimit'

function ThermalBar({ label, value }: { label: string; value?: number | null }) {
  if (value == null) {
    return (
      <div className="flex justify-between text-meta">
        <span className="text-ink2">{label}</span>
        <span className="font-medium text-ink3">Unavailable</span>
      </div>
    )
  }
  const pct = Math.min((value / 100) * 100, 100)
  const tone = value > 80 ? 'bg-danger' : value > 60 ? 'bg-warn' : 'bg-ok'
  return (
    <div>
      <div className="mb-0.5 flex justify-between text-meta">
        <span className="text-ink2">{label}</span>
        <span className={`tnum font-mono font-semibold ${tempColorClass(value)}`}>{value.toFixed(1)}°C</span>
      </div>
      <Meter pct={pct} tone={tone} />
    </div>
  )
}

// ── Charge control ────────────────────────────────────────────────────────────

function ChargeControlCard() {
  const { data: cc, error, status, refresh, mutate } = usePoll('charge-control', api.chargeControl, 10000)
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)
  // Local edit of the limit. `null` = pristine: the slider shows the device's
  // value, and polls can never overwrite a value the user is still choosing.
  const [draft, setDraft] = useState<number | null>(null)

  async function apply(body: Partial<ChargeControlState>) {
    if (busy) return
    setBusy(true)
    setFailure(null)
    try {
      // The PUT returns the authoritative new state — publish it rather than
      // spending a second request re-reading what we were just handed.
      mutate(await api.chargeControlSet(body))
      if ('charge_limit' in body) setDraft(null)
    } catch (e) {
      setFailure(e instanceof Error ? e.message : 'Charge control failed')
    } finally {
      setBusy(false)
    }
  }

  if (!cc) {
    return (
      <Card title="Charge control">
        {status === 'error' ? (
          <InlineStatus kind="error" action={{ label: 'Retry', onClick: refresh }}>
            Charge control unavailable: {error}
          </InlineStatus>
        ) : (
          <Skeleton className="h-32" />
        )}
      </Card>
    )
  }

  const shown = draft ?? cc.charge_limit
  const pending = limitToApply(draft, cc.charge_limit)
  const limitEditable = cc.charge_limit_enabled && cc.battery_available

  return (
    <Card title="Charge control">
      <div className="space-y-3">
        {status === 'stale' && (
          <InlineStatus kind="stale" action={{ label: 'Retry', onClick: refresh }}>
            Showing the last reading; refresh failed: {error}
          </InlineStatus>
        )}
        {cc.last_error && <InlineStatus kind="error">{cc.last_error}</InlineStatus>}
        {failure && <InlineStatus kind="error">Change not applied: {failure}</InlineStatus>}
        <div className="flex items-center justify-between gap-2">
          <div className="min-w-0">
            <p className="text-body font-medium text-ink">Charging</p>
            <p className="truncate text-meta text-ink2">
              {cc.battery_status ?? 'Battery data unavailable'}{cc.capacity != null ? ` at ${cc.capacity}%` : ''}{cc.manual_override ? ' · manual override' : ''}
            </p>
          </div>
          <Button
            size="sm"
            variant={cc.charging_stopped ? 'primary' : 'outline'}
            loading={busy}
            disabled={!cc.charger_available || cc.charging_stopped == null}
            onClick={() => apply({ charging_stopped: !cc.charging_stopped })}
          >
            {cc.charging_stopped ? 'Resume charging' : 'Stop charging'}
          </Button>
        </div>

        <div className="border-t border-line/8 pt-3">
          <div className="flex items-center justify-between">
            <div>
              <p id="charge-limit-enforcer" className="text-body font-medium text-ink">Charge limit</p>
              <p className="text-meta text-ink2">
                Stop at limit, resume {cc.hysteresis}% below
              </p>
            </div>
            <Toggle
              checked={cc.charge_limit_enabled}
              disabled={busy || !cc.battery_available}
              onChange={(v) => apply({ charge_limit_enabled: v })}
              labelledBy="charge-limit-enforcer"
            />
          </div>
          <div className="mt-3">
            <Field label="Stop charging at" hint={pending != null ? `Not applied yet. The device limit is ${cc.charge_limit}%.` : undefined}>
              {(ids) => (
                <div className="flex items-center gap-3">
                  <input
                    id={ids.id}
                    aria-describedby={ids.describedBy}
                    type="range"
                    min={LIMIT_MIN}
                    max={LIMIT_MAX}
                    step={LIMIT_STEP}
                    value={shown}
                    aria-valuetext={`${shown}%`}
                    disabled={busy || !limitEditable}
                    onChange={(e) => setDraft(Number(e.target.value))}
                    onKeyDown={(e) => {
                      if (e.key === 'Escape' && draft != null) {
                        e.preventDefault()
                        setDraft(null)
                      }
                    }}
                    className="w-full accent-[rgb(var(--accent))] disabled:opacity-40"
                  />
                  <span className="tnum w-12 text-right font-mono text-body font-semibold text-ink" aria-hidden="true">
                    {shown}%
                  </span>
                </div>
              )}
            </Field>
            {draft != null && (
              <div className="mt-2 flex justify-end gap-2">
                <Button size="sm" variant="ghost" disabled={busy} onClick={() => setDraft(null)}>
                  Cancel
                </Button>
                <Button
                  size="sm"
                  variant="primary"
                  loading={busy}
                  disabled={pending == null || !limitEditable}
                  onClick={() => pending != null && apply({ charge_limit: pending })}
                >
                  Apply limit
                </Button>
              </div>
            )}
          </div>
        </div>

        <p className="text-caption leading-snug text-ink3">
          Charging resumes automatically when the charger is unplugged or the limit is turned off.
        </p>
        {!cc.available && <p className="text-meta font-medium text-warn">Battery and charger hardware data are unavailable; controls are disabled.</p>}
      </div>
    </Card>
  )
}

// ── Sources ───────────────────────────────────────────────────────────────────

/**
 * One owner per resource. CPU and memory come from the heartbeat (`useHome`),
 * which this tab switches to its fast cadence while it is showing; the other
 * sources are not in the heartbeat and poll on their own, each at the rate its
 * readings actually change. A failed source never blanks the others.
 */
const THERMAL_MS = 5000
const BATTERY_DETAIL_MS = 10000
/** Fuel-gauge type and online state: near-static. */
const BATTERY_INFO_MS = 30000

type Source = Pick<PollResult<unknown>, 'status' | 'error' | 'refresh' | 'refreshing'>

/**
 * The read state of one source: an error with Retry when nothing could be
 * read, or a stale note over the last good reading when a later read failed.
 * `live` is false for a repeat of a status already announced in another card.
 */
function SourceStatus({ what, source, live = true }: { what: string; source: Source; live?: boolean }) {
  const action = { label: 'Retry', onClick: source.refresh, loading: source.refreshing }
  if (source.status === 'error') {
    return (
      <InlineStatus kind="error" live={live} action={action}>
        Could not read the {what}{source.error ? `: ${source.error}` : '.'}
      </InlineStatus>
    )
  }
  if (source.status === 'stale') {
    return (
      <InlineStatus kind="stale" live={live} action={action}>
        Showing the last {what} reading; refresh failed{source.error ? `: ${source.error}` : '.'}
      </InlineStatus>
    )
  }
  return null
}

function CardLoading({ label, className }: { label: string; className: string }) {
  return (
    <Loading label={label}>
      <Skeleton className={className} />
    </Loading>
  )
}

const THERMAL_LABELS = ['CPU (avg)', 'Modem (Q6 DSP)', 'Modem SS', 'PA (power amplifier)', 'SDR (radio)', 'Battery', 'USB', 'Ethernet PHY', 'PMIC', 'Board (XO)']

function cpuAverage(thermal: ThermalAll): number | undefined {
  const cores = [thermal.cpu_0, thermal.cpu_1, thermal.cpu_2, thermal.cpu_3].filter((v): v is number => v != null)
  return cores.length > 0 ? cores.reduce((a, b) => a + b, 0) / cores.length : undefined
}

function ThermalCard() {
  const thermal = usePoll('metrics:thermal', api.thermalAll, THERMAL_MS)
  const t = thermal.data
  return (
    <Card title="Temperatures">
      {!t ? (
        thermal.status === 'error' ? (
          <SourceStatus what="temperatures" source={thermal} />
        ) : (
          <CardLoading label="Loading temperatures" className="h-64" />
        )
      ) : (
        <div className="space-y-2.5">
          <SourceStatus what="temperatures" source={thermal} />
          {t.available ? (
            <>
              <ThermalBar label="CPU (avg)" value={cpuAverage(t)} />
              <ThermalBar label="Modem (Q6 DSP)" value={t.modem} />
              <ThermalBar label="Modem SS" value={t.modem_ss0} />
              <ThermalBar label="PA (power amplifier)" value={t.pa} />
              <ThermalBar label="SDR (radio)" value={t.sdr} />
              <ThermalBar label="Battery" value={t.battery} />
              <ThermalBar label="USB" value={t.usb} />
              <ThermalBar label="Ethernet PHY" value={t.eth_phy} />
              <ThermalBar label="PMIC" value={t.pmic} />
              <ThermalBar label="Board (XO)" value={t.xo_therm} />
            </>
          ) : (
            <>
              <p className="text-meta font-medium text-warn">Thermal sensors are unavailable.</p>
              {THERMAL_LABELS.map((label) => <ThermalBar key={label} label={label} />)}
            </>
          )}
        </div>
      )}
    </Card>
  )
}

/** CPU and memory from the heartbeat; `null` sections mean the agent did not report them. */
function HeartbeatCards() {
  useFastHeartbeat()
  const home = useHome()
  const cpu = home.data?.cpu ?? null
  const mem = home.data?.memory ?? null

  if (!home.data) {
    return (
      <div className="space-y-3">
        <Card title="CPU usage">
          {home.status === 'error' ? <SourceStatus what="CPU usage" source={home} /> : <CardLoading label="Loading CPU usage" className="h-40" />}
        </Card>
        <Card title="Memory">
          {home.status === 'error' ? <SourceStatus what="memory usage" source={home} live={false} /> : <CardLoading label="Loading memory usage" className="h-12" />}
        </Card>
      </div>
    )
  }

  return (
    <div className="space-y-3">
      <Card title="CPU usage">
        <div className="space-y-3">
          <SourceStatus what="CPU usage" source={home} />
          {cpu ? (
            <>
              <div>
                <div className="mb-1 flex justify-between text-body">
                  <span className="font-medium text-ink">Overall</span>
                  <span className="tnum font-mono text-ink2">{cpu.overall.toFixed(1)}%</span>
                </div>
                <Meter pct={cpu.overall} />
              </div>
              {cpu.cores.length > 0 && (
                <div className="space-y-1.5 border-t border-line/8 pt-2.5">
                  {cpu.cores.map((pct, i) => (
                    <div key={i}>
                      <div className="mb-0.5 flex justify-between text-caption">
                        <span className="text-ink3">Core {i}</span>
                        <span className="tnum font-mono text-ink2">{pct.toFixed(1)}%</span>
                      </div>
                      <Meter pct={pct} />
                    </div>
                  ))}
                </div>
              )}
            </>
          ) : (
            <p className="text-body font-medium text-warn">The router did not report CPU usage.</p>
          )}
        </div>
      </Card>

      <Card title="Memory">
        <div className="space-y-3">
          <SourceStatus what="memory usage" source={home} live={false} />
          {mem ? (
            <div>
              <div className="mb-1 flex justify-between text-body">
                <span className="text-ink2">Usage</span>
                <span className="tnum font-mono text-ink">
                  {formatBytes(mem.used_kb * 1024)} / {formatBytes(mem.total_kb * 1024)} ({mem.usage_pct.toFixed(0)}%)
                </span>
              </div>
              <Meter pct={mem.usage_pct} tone="bg-warn" />
            </div>
          ) : (
            <p className="text-body font-medium text-warn">The router did not report memory usage.</p>
          )}
        </div>
      </Card>
    </div>
  )
}

function BatteryCard({ detail, info }: { detail: PollResult<BatteryDetail>; info: PollResult<BatteryBspInfo> }) {
  const battery = detail.data
  if (!battery) {
    return (
      <Card title="Battery">
        {detail.status === 'error' ? <SourceStatus what="battery" source={detail} /> : <CardLoading label="Loading battery" className="h-48" />}
      </Card>
    )
  }

  const bsp = info.data?.available ? info.data : null
  const bspValue = (value: boolean | null | undefined, yes: string, no: string) =>
    bsp && value != null ? (value ? yes : no) : info.status === 'loading' ? 'Loading' : 'Unavailable'

  return (
    <Card title="Battery">
      <div className="space-y-3">
        <SourceStatus what="battery" source={detail} />
        {battery.available ? (
          <>
            {/* Fuel gauge and online state come from a separate source. */}
            <SourceStatus what="fuel gauge" source={info} />
            <div>
              <div className="mb-1 flex justify-between text-body">
                <span className="tnum font-mono font-semibold text-ink">{battery.capacity != null ? `${battery.capacity}%` : 'Unavailable'}</span>
                <span className="text-ink2">{battery.status ?? 'Unavailable'}</span>
              </div>
              {battery.capacity != null && <Meter pct={battery.capacity} tone={battery.capacity > 20 ? 'bg-ok' : 'bg-danger'} />}
            </div>
            <div className="grid grid-cols-2 gap-x-4 gap-y-2 text-body">
              <Info label="Power" value={formatMeasure(battery.power_mw, (value) => `${(value / 1000).toFixed(2)} W`)} />
              <Info label="Voltage" value={formatMeasure(battery.voltage_mv, (value) => `${(value / 1000).toFixed(3)} V`)} />
              <Info label="Current" value={formatMeasure(battery.current_ma, (value) => `${value} mA`)} />
              <Info label="Charge type" value={battery.charge_type ?? 'Unavailable'} />
              <Info label="Temperature" value={formatMeasure(battery.temperature_c, (value) => `${value.toFixed(1)}°C`)} cls={battery.temperature_c != null ? tempColorClass(battery.temperature_c) : 'text-ink3'} />
              <Info
                label={battery.status === 'Charging' ? 'Time to full' : 'Time to empty'}
                value={formatClock(battery.status === 'Charging' ? battery.time_to_full_secs : battery.time_to_empty_secs)}
              />
              <Info label="Charge counter" value={formatMeasure(battery.charge_counter_mah, (value) => `${value.toLocaleString()} mAh`)} />
              <Info label="Cycles" value={formatMeasure(battery.cycle_count, String)} />
              <Info label="Fuel gauge" value={bspValue(bsp?.using_hw_fg_chip, 'Hardware', 'Software')} />
              <Info label="Battery online" value={bspValue(bsp?.online, 'Yes', 'No')} />
            </div>
          </>
        ) : (
          <p className="text-body font-medium text-warn">Battery hardware data are unavailable.</p>
        )}
      </div>
    </Card>
  )
}

function BatteryHealthCard({ detail }: { detail: PollResult<BatteryDetail> }) {
  const battery = detail.data
  if (!battery) {
    return (
      <Card title="Battery health">
        {detail.status === 'error' ? (
          // The Battery card already announced this failure.
          <SourceStatus what="battery health" source={detail} live={false} />
        ) : (
          <CardLoading label="Loading battery health" className="h-28" />
        )}
      </Card>
    )
  }

  const retention =
    battery.available && battery.charge_full_design_mah != null && battery.charge_full_design_mah > 0 && battery.charge_full_mah != null
      ? Math.round((battery.charge_full_mah / battery.charge_full_design_mah) * 100)
      : undefined

  return (
    <Card title="Battery health">
      <div className="space-y-3">
        <SourceStatus what="battery health" source={detail} live={false} />
        {battery.available ? (
          <div className="space-y-1.5 text-body">
            <KV k="Health" v={battery.health ?? 'Unavailable'} />
            <KV k="Capacity" v={battery.charge_full_mah != null && battery.charge_full_design_mah != null ? `${battery.charge_full_mah.toLocaleString()} / ${battery.charge_full_design_mah.toLocaleString()} mAh` : 'Unavailable'} />
            {retention != null && (
              <div className="flex justify-between">
                <span className="text-ink2">Capacity retention</span>
                <span className={`tnum font-mono font-semibold ${retention > 80 ? 'text-ok' : 'text-warn'}`}>{retention}%</span>
              </div>
            )}
            <KV k="OCV" v={formatMeasure(battery.voltage_ocv_mv, (value) => `${(value / 1000).toFixed(3)} V`)} />
          </div>
        ) : (
          <p className="text-body font-medium text-warn">Battery health measures are unavailable.</p>
        )}
      </div>
    </Card>
  )
}

// ── Tab ───────────────────────────────────────────────────────────────────────

export default function MetricsTab() {
  // Battery detail feeds two cards, so the tab owns it (and the fuel-gauge read beside it).
  const detail = usePoll('metrics:battery-detail', api.batteryDetail, BATTERY_DETAIL_MS)
  const info = usePoll('metrics:battery-info', api.batteryInfoUbus, BATTERY_INFO_MS)

  return (
    <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
      <ThermalCard />
      <HeartbeatCards />
      <BatteryCard detail={detail} info={info} />
      <div className="space-y-3">
        <BatteryHealthCard detail={detail} />
        <ChargeControlCard />
      </div>
    </div>
  )
}

function Info({ label, value, cls = 'text-ink' }: { label: string; value: string; cls?: string }) {
  return (
    <div className="min-w-0">
      <p className="label">{label}</p>
      <p className={`tnum font-mono truncate font-medium ${cls}`}>{value}</p>
    </div>
  )
}

function KV({ k, v }: { k: string; v: string }) {
  return (
    <div className="flex justify-between">
      <span className="text-ink2">{k}</span>
      <span className="tnum font-mono font-medium text-ink">{v}</span>
    </div>
  )
}

function formatMeasure(value: number | null, format: (value: number) => string) {
  return value == null ? 'Unavailable' : format(value)
}

function formatClock(secs: number | null) {
  if (secs == null || secs <= 0) return 'Unavailable'
  const h = Math.floor(secs / 3600)
  const m = Math.floor((secs % 3600) / 60)
  return h > 0 ? `${h}h ${m}m` : `${m}m`
}
