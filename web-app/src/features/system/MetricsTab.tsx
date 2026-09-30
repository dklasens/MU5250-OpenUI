import { useEffect, useState } from 'react'
import { api } from '../../data/api'
import { usePoll } from '../../data/poll'
import { tempColorClass } from '../../format'
import type { BatteryBspInfo, BatteryDetail, ChargeControlState, CpuInfo, MemInfo, ThermalAll } from '../../types'
import { formatBytes } from '../../format'
import { Button, Toggle } from '../../ui/controls'
import { toastError } from '../../ui/feedback'
import { Card, Meter, Skeleton } from '../../ui/primitives'

interface MetricsData {
  thermal: ThermalAll | null
  battery: BatteryDetail | null
  batteryInfo: BatteryBspInfo | null
  cpu: CpuInfo | null
  mem: MemInfo | null
}

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
  const { data: cc, mutate } = usePoll('charge-control', api.chargeControl, 10000)
  const [busy, setBusy] = useState(false)
  const [limit, setLimit] = useState<number | null>(null)
  const [dragging, setDragging] = useState(false)

  useEffect(() => {
    if (!dragging && cc) setLimit(cc.charge_limit)
  }, [cc, dragging])

  async function apply(body: Partial<ChargeControlState>) {
    setBusy(true)
    try {
      // The PUT returns the authoritative new state — publish it rather than
      // spending a second request re-reading what we were just handed.
      mutate(await api.chargeControlSet(body))
    } catch (e) {
      toastError(e, 'Charge control failed')
    } finally {
      setBusy(false)
    }
  }

  if (!cc) return null

  return (
    <Card title="Charge control">
      <div className="space-y-3">
        {cc.last_error && <p role="alert" className="text-meta text-danger">{cc.last_error}</p>}
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
            disabled={!cc.charger_available}
            onClick={() => apply({ charging_stopped: !cc.charging_stopped })}
          >
            {cc.charging_stopped ? 'Resume charging' : 'Stop charging'}
          </Button>
        </div>

        <div className="border-t border-line/8 pt-3">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-body font-medium text-ink">Charge limit</p>
              <p className="text-meta text-ink2">
                Stop at limit, resume {cc.hysteresis}% below
              </p>
            </div>
            <Toggle
              checked={cc.charge_limit_enabled}
              disabled={busy || !cc.battery_available}
              onChange={(v) => apply({ charge_limit_enabled: v })}
              label="Charge limit enforcer"
            />
          </div>
          <div className="mt-2 flex items-center gap-3">
            <input
              type="range"
              min={50}
              max={100}
              step={5}
              value={limit ?? cc.charge_limit}
              disabled={busy || !cc.charge_limit_enabled || !cc.battery_available}
              onChange={(e) => setLimit(Number(e.target.value))}
              onPointerDown={() => setDragging(true)}
              onPointerUp={(e) => {
                setDragging(false)
                apply({ charge_limit: Number(e.currentTarget.value) })
              }}
              onKeyUp={(e) => apply({ charge_limit: Number(e.currentTarget.value) })}
              className="w-full accent-[rgb(var(--accent))] disabled:opacity-40"
            />
            <span className="tnum font-mono w-12 text-right text-body font-semibold text-ink">
              {limit ?? cc.charge_limit}%
            </span>
          </div>
        </div>

        <p className="text-caption leading-snug text-ink3">
          Firmware note: the charger switch is inverted (enable = stop). Charging auto-resumes when the
          charger is unplugged or the limit is disabled.
        </p>
        {!cc.available && <p className="text-meta font-medium text-warn">Battery and charger hardware data are unavailable; controls are disabled.</p>}
      </div>
    </Card>
  )
}

// ── Tab ───────────────────────────────────────────────────────────────────────

export default function MetricsTab() {
  const { data } = usePoll<MetricsData>(
    'metrics',
    async () => {
      const [t, b, bi, c, m] = await Promise.allSettled([
        api.thermalAll(),
        api.batteryDetail(),
        api.batteryInfoUbus(),
        api.cpu(),
        api.memory(),
      ])
      return {
        thermal: t.status === 'fulfilled' ? t.value : null,
        battery: b.status === 'fulfilled' ? b.value : null,
        batteryInfo: bi.status === 'fulfilled' ? bi.value : null,
        cpu: c.status === 'fulfilled' ? c.value : null,
        mem: m.status === 'fulfilled' ? m.value : null,
      }
    },
    5000,
  )

  if (!data) {
    return (
      <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
        <Skeleton className="h-64" />
        <Skeleton className="h-64" />
        <Skeleton className="h-48" />
        <Skeleton className="h-48" />
      </div>
    )
  }

  const { thermal, battery, batteryInfo, cpu, mem } = data

  const cpuAvg =
    thermal?.available && thermal.cpu_0 != null
      ? [thermal.cpu_0, thermal.cpu_1, thermal.cpu_2, thermal.cpu_3]
          .filter((v): v is number => v != null)
          .reduce((a, b) => a + b, 0) /
        [thermal.cpu_0, thermal.cpu_1, thermal.cpu_2, thermal.cpu_3].filter((v) => v != null).length
      : undefined

  const batteryHealth =
    battery?.available && battery.charge_full_design_mah != null && battery.charge_full_design_mah > 0 && battery.charge_full_mah != null
      ? Math.round((battery.charge_full_mah / battery.charge_full_design_mah) * 100)
      : undefined

  return (
    <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
      <Card title="Temperatures">
        {thermal?.available ? (
          <div className="space-y-2.5">
            <ThermalBar label="CPU (avg)" value={cpuAvg} />
            <ThermalBar label="Modem (Q6 DSP)" value={thermal.modem} />
            <ThermalBar label="Modem SS" value={thermal.modem_ss0} />
            <ThermalBar label="PA (power amplifier)" value={thermal.pa} />
            <ThermalBar label="SDR (radio)" value={thermal.sdr} />
            <ThermalBar label="Battery" value={thermal.battery} />
            <ThermalBar label="USB" value={thermal.usb} />
            <ThermalBar label="Ethernet PHY" value={thermal.eth_phy} />
            <ThermalBar label="PMIC" value={thermal.pmic} />
            <ThermalBar label="Board (XO)" value={thermal.xo_therm} />
          </div>
        ) : (
          <div className="space-y-2.5">
            <p className="text-meta font-medium text-warn">Thermal sensors are unavailable.</p>
            {['CPU (avg)', 'Modem (Q6 DSP)', 'Modem SS', 'PA (power amplifier)', 'SDR (radio)', 'Battery', 'USB', 'Ethernet PHY', 'PMIC', 'Board (XO)'].map((label) => <ThermalBar key={label} label={label} />)}
          </div>
        )}
      </Card>

      <div className="space-y-3">
        <Card title="CPU usage">
          {cpu ? (
            <div className="space-y-3">
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
            </div>
          ) : (
            <p className="text-body text-ink3">No CPU data</p>
          )}
        </Card>

        <Card title="Memory">
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
            <p className="text-body text-ink3">No memory data</p>
          )}
        </Card>
      </div>

      <Card title="Battery">
        {battery?.available ? (
          <div className="space-y-3">
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
              <Info label="Fuel gauge" value={batteryInfo?.available && batteryInfo.using_hw_fg_chip != null ? (batteryInfo.using_hw_fg_chip ? 'Hardware' : 'Software') : 'Unavailable'} />
              <Info label="Battery online" value={batteryInfo?.available && batteryInfo.online != null ? (batteryInfo.online ? 'Yes' : 'No') : 'Unavailable'} />
            </div>
          </div>
        ) : (
          <p className="text-body font-medium text-warn">Battery hardware data are unavailable.</p>
        )}
      </Card>

      <div className="space-y-3">
        <Card title="Battery health">
          {battery?.available ? (
            <div className="space-y-1.5 text-body">
              <KV k="Health" v={battery.health ?? 'Unavailable'} />
              <KV k="Capacity" v={battery.charge_full_mah != null && battery.charge_full_design_mah != null ? `${battery.charge_full_mah.toLocaleString()} / ${battery.charge_full_design_mah.toLocaleString()} mAh` : 'Unavailable'} />
              {batteryHealth != null && (
                <div className="flex justify-between">
                  <span className="text-ink2">Capacity retention</span>
                  <span className={`tnum font-mono font-semibold ${batteryHealth > 80 ? 'text-ok' : 'text-warn'}`}>{batteryHealth}%</span>
                </div>
              )}
              <KV k="OCV" v={formatMeasure(battery.voltage_ocv_mv, (value) => `${(value / 1000).toFixed(3)} V`)} />
            </div>
          ) : (
          <p className="text-body font-medium text-warn">Battery health measures are unavailable.</p>
          )}
        </Card>

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
