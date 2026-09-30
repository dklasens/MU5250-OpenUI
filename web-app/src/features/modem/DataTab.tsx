import { useCallback, useEffect, useState } from 'react'
import { useHome } from '../../app/HomeContext'
import { api } from '../../data/api'
import { formatBytes, formatUptime } from '../../format'
import { IDownload, IUpload } from '../../icons'
import type { UsagePeriod } from '../../types'
import { Button, Input } from '../../ui/controls'
import { toast, toastError } from '../../ui/feedback'
import { Card, Skeleton } from '../../ui/primitives'

function clampResetDay(day: number, year: number, month: number) {
  return Math.min(day, new Date(year, month + 1, 0).getDate())
}

function cycleWindow(resetDay: number, now = new Date()) {
  const currentDay = clampResetDay(resetDay, now.getFullYear(), now.getMonth())
  const startsThisMonth = now.getDate() >= currentDay
  const startMonth = startsThisMonth ? now.getMonth() : now.getMonth() - 1
  const startYear = now.getFullYear() + (startMonth < 0 ? -1 : 0)
  const normalizedStartMonth = (startMonth + 12) % 12
  const start = new Date(startYear, normalizedStartMonth, clampResetDay(resetDay, startYear, normalizedStartMonth))
  const nextMonth = normalizedStartMonth + 1
  const nextYear = startYear + (nextMonth > 11 ? 1 : 0)
  const normalizedNextMonth = nextMonth % 12
  const nextStart = new Date(nextYear, normalizedNextMonth, clampResetDay(resetDay, nextYear, normalizedNextMonth))
  const end = new Date(nextStart)
  end.setDate(end.getDate() - 1)
  return { start, end, nextStart }
}

function formatDate(date: Date) {
  return date.toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' })
}

function UsageTotals({ usage }: { usage: UsagePeriod }) {
  const total = usage.rx_bytes + usage.tx_bytes
  return (
    <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
      <div className="rounded-ctl bg-surface2/70 p-3">
        <p className="label text-ok">Download</p>
        <p className="tnum font-mono mt-1 text-xl font-medium text-ink">{formatBytes(usage.rx_bytes)}</p>
      </div>
      <div className="rounded-ctl bg-surface2/70 p-3">
        <p className="label text-accent">Upload</p>
        <p className="tnum font-mono mt-1 text-xl font-medium text-ink">{formatBytes(usage.tx_bytes)}</p>
      </div>
      <div className="rounded-ctl bg-surface2/70 p-3">
        <p className="label">Total</p>
        <p className="tnum font-mono mt-1 text-xl font-medium text-ink">{formatBytes(total)}</p>
      </div>
    </div>
  )
}

export default function DataTab() {
  // The home batch already carries data_usage; a second poll for it was
  // duplicate ubus load on the agent.
  const { data: home, refresh } = useHome()
  const [editingResetDay, setEditingResetDay] = useState(false)
  const [resetDay, setResetDay] = useState('1')
  const [busy, setBusy] = useState(false)

  const usage = home?.usage ?? null

  useEffect(() => {
    if (usage?.reset_day && !editingResetDay) setResetDay(String(usage.reset_day))
  }, [usage?.reset_day, editingResetDay])

  const saveResetDay = useCallback(async () => {
    const day = parseInt(resetDay, 10)
    if (!day || day < 1 || day > 31) {
      toast('Reset day must be between 1 and 31', 'err')
      return
    }
    setBusy(true)
    try {
      await api.dataUsageResetDaySet(day)
      setEditingResetDay(false)
      toast(`Reset day set to day ${day}`)
      refresh()
    } catch (e) {
      toastError(e, 'Failed to set reset day')
    } finally {
      setBusy(false)
    }
  }, [resetDay, refresh])

  if (!usage) {
    return (
      <div className="space-y-3">
        <Skeleton className="h-48" />
        <Skeleton className="h-32" />
      </div>
    )
  }

  const currentResetDay = usage.reset_day ?? (parseInt(resetDay, 10) || 1)
  const dates = cycleWindow(currentResetDay)
  const cycle = usage.cycle ?? usage.month
  const sincePowerOn = usage.since_power_on
  const otherCounters = [
    { label: 'Today', data: usage.day },
    { label: 'Device lifetime', data: usage.total },
  ]

  return (
    <div className="space-y-3">
      <Card
        title="Current data cycle"
        action={
          <Button size="sm" variant="ghost" onClick={() => setEditingResetDay((v) => !v)}>
            Set reset day
          </Button>
        }
      >
        {editingResetDay && (
          <div className="mb-3 flex flex-wrap items-end gap-2 rounded-ctl bg-surface2/70 p-3">
            <div className="w-28">
              <p className="label mb-1">Reset day</p>
              <Input type="number" min={1} max={31} value={resetDay} onChange={(e) => setResetDay(e.target.value)} />
            </div>
            <Button variant="primary" onClick={saveResetDay} loading={busy}>
              Save
            </Button>
            <Button variant="ghost" onClick={() => setEditingResetDay(false)}>
              Cancel
            </Button>
          </div>
        )}

        {cycle ? (
          <div className="space-y-3">
            <div className="flex flex-wrap gap-x-4 gap-y-1 text-body text-ink2">
              <span>
                Reset day: <span className="tnum font-mono font-bold text-ink">{currentResetDay}</span>
              </span>
              <span>
                Period: <span className="font-bold text-ink">{formatDate(dates.start)} – {formatDate(dates.end)}</span>
              </span>
              <span>
                Next reset: <span className="font-bold text-ink">{formatDate(dates.nextStart)}</span>
              </span>
            </div>
            <UsageTotals usage={cycle} />
            <p className="text-meta text-ink3">
              Counters are maintained by the router and reset on the configured day each month.
            </p>
          </div>
        ) : (
          <p className="text-body text-ink3">No cycle data</p>
        )}
      </Card>

      {sincePowerOn && (
        <Card title="Since power on">
          <UsageTotals usage={sincePowerOn} />
          <p className="mt-2 text-meta text-ink3">Counter time: {formatUptime(sincePowerOn.time_secs)}</p>
        </Card>
      )}

      <Card title="Other counters" pad={false}>
        {/* Mobile: stacked rows instead of a five-column table */}
        <ul className="divide-y divide-line/6 px-4 sm:hidden">
          {otherCounters.map(({ label, data: d }) => (
            <li key={label} className="py-2.5">
              <div className="flex items-baseline justify-between gap-2 text-body">
                <span className="text-ink2">{label}</span>
                <span className="tnum font-mono font-semibold text-ink">{formatBytes(d.rx_bytes + d.tx_bytes)}</span>
              </div>
              <div className="tnum font-mono mt-0.5 flex flex-wrap gap-x-3 text-meta">
                <span className="flex items-center gap-1 text-ok"><IDownload size={12} /> {formatBytes(d.rx_bytes)}</span>
                <span className="flex items-center gap-1 text-accent"><IUpload size={12} /> {formatBytes(d.tx_bytes)}</span>
                <span className="ml-auto text-ink3">{formatUptime(d.time_secs)}</span>
              </div>
            </li>
          ))}
        </ul>
        <div className="hidden overflow-x-auto px-4 pb-3 sm:block">
          <table className="w-full text-body">
            <thead>
              <tr className="label border-b border-line/8 text-left">
                <th className="pb-1.5 pr-4 font-semibold">Period</th>
                <th className="pb-1.5 pr-4 text-right font-semibold">Down</th>
                <th className="pb-1.5 pr-4 text-right font-semibold">Up</th>
                <th className="pb-1.5 pr-4 text-right font-semibold">Total</th>
                <th className="pb-1.5 text-right font-semibold">Time</th>
              </tr>
            </thead>
            <tbody>
              {otherCounters.map(({ label, data: d }) => (
                <tr key={label} className="border-b border-line/6 last:border-0">
                  <td className="py-2 pr-4 text-ink2">{label}</td>
                  <td className="tnum font-mono py-2 pr-4 text-right text-ok">{formatBytes(d.rx_bytes)}</td>
                  <td className="tnum font-mono py-2 pr-4 text-right text-accent">{formatBytes(d.tx_bytes)}</td>
                  <td className="tnum font-mono py-2 pr-4 text-right font-semibold text-ink">
                    {formatBytes(d.rx_bytes + d.tx_bytes)}
                  </td>
                  <td className="tnum font-mono py-2 text-right text-ink3">{formatUptime(d.time_secs)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </Card>
    </div>
  )
}
