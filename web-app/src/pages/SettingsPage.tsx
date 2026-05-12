import { useState, useEffect, useCallback } from 'react'
import { Power, RefreshCw, RotateCcw } from 'lucide-react'
import { api, formatBytes, type DeviceInfo, type SimInfo, type MemInfo, API_BASE } from '../api'
import Card from '../components/Card'

interface Props { onLogout: () => void }
type ControlAction = 'restart' | 'reboot' | 'shutdown'
type ControlMessage = { type: 'success' | 'error' | 'info'; text: string }

function controlMessageClass(type: ControlMessage['type']) {
  if (type === 'success') return 'text-green-500'
  if (type === 'error') return 'text-red-500'
  return 'text-amber-500'
}

function UsbModeSection() {
  const [msg, setMsg] = useState<ControlMessage | null>(null)
  const [loading, setLoading] = useState(false)

  async function setMode(mode: string) {
    setLoading(true)
    setMsg(null)
    try {
      await api.usbMode(mode)
      setMsg({ type: 'success', text: `USB mode set to ${mode.toUpperCase()}. Device may need reboot.` })
    } catch (e) {
      setMsg({ type: 'error', text: e instanceof Error ? e.message : 'Failed to set USB mode' })
    } finally {
      setLoading(false)
    }
  }

  return (
    <Card title="USB Mode">
      <div className="space-y-3">
        <p className="text-xs text-gray-500">Switch USB operating mode. A reboot may be required.</p>
        {msg && <p className={`text-sm ${controlMessageClass(msg.type)}`}>{msg.text}</p>}
        <div className="flex flex-wrap gap-2">
          {['rndis', 'ecm', 'ncm', 'debug'].map(mode => (
            <button
              key={mode}
              onClick={() => setMode(mode)}
              disabled={loading}
              className="bg-white border border-gray-200 hover:bg-gray-50 px-3 py-2 rounded-xl font-bold text-gray-500 shadow-macos transition-all active:scale-95 text-sm disabled:opacity-40"
            >
              {mode.toUpperCase()}
            </button>
          ))}
        </div>
      </div>
    </Card>
  )
}

export default function SettingsPage({ onLogout }: Props) {
  const [device, setDevice] = useState<DeviceInfo | null>(null)
  const [sim, setSim] = useState<SimInfo | null>(null)
  const [imei, setImei] = useState('')
  const [mem, setMem] = useState<MemInfo | null>(null)
  const [top, setTop] = useState<{ pid?: number; name?: string; cpu_percent?: number; mem_kb?: number }[]>([])
  const [controlMsg, setControlMsg] = useState<ControlMessage | null>(null)
  const [confirmAction, setConfirmAction] = useState<'reboot' | 'shutdown' | null>(null)
  const [busyAction, setBusyAction] = useState<ControlAction | null>(null)

  const fetchAll = useCallback(async () => {
    const results = await Promise.allSettled([
      api.device(), api.simInfo(), api.simImei(), api.memory(), api.top(),
    ])
    const [d, s, i, m, p] = results
    if (d.status === 'fulfilled') setDevice(d.value)
    if (s.status === 'fulfilled') setSim(s.value)
    if (i.status === 'fulfilled' && i.value) setImei(i.value.imei ?? '')
    if (m.status === 'fulfilled') setMem(m.value)
    if (p.status === 'fulfilled') setTop(Array.isArray(p.value) ? p.value.slice(0, 15) : [])
  }, [])

  useEffect(() => {
    fetchAll()
    const id = setInterval(fetchAll, 5000)
    return () => clearInterval(id)
  }, [fetchAll])

  function formatUptime(s?: number) {
    if (!s) return '—'
    const d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600), m = Math.floor((s % 3600) / 60)
    return [d && `${d}d`, (d || h) && `${h}h`, `${m}m`].filter(Boolean).join(' ')
  }

  async function restartAgent() {
    setBusyAction('restart')
    setControlMsg({ type: 'info', text: 'Restarting agent...' })
    try {
      await api.restartAgent()
      setControlMsg({ type: 'success', text: 'Agent restarting. Reconnecting in a few seconds...' })
      setTimeout(() => window.location.reload(), 5000)
    } catch (e) {
      setControlMsg({ type: 'error', text: e instanceof Error ? e.message : 'Failed to restart agent' })
    } finally {
      setBusyAction(null)
    }
  }

  async function runPowerAction(action: 'reboot' | 'shutdown') {
    setBusyAction(action)
    setControlMsg({ type: 'info', text: action === 'reboot' ? 'Sending reboot command...' : 'Sending shutdown command...' })
    try {
      if (action === 'reboot') {
        await api.reboot()
        setControlMsg({ type: 'success', text: 'Reboot command sent. Device should restart within about 10-30 seconds.' })
      } else {
        await api.shutdown()
        setControlMsg({ type: 'success', text: 'Shutdown command sent. Use the physical power button to turn the device back on.' })
      }
      setConfirmAction(null)
    } catch (e) {
      setControlMsg({ type: 'error', text: e instanceof Error ? e.message : `Failed to ${action} device.` })
    } finally {
      setBusyAction(null)
    }
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl sm:text-3xl font-bold text-gray-900">Settings</h1>
        <button onClick={onLogout} className="px-4 py-2 text-sm bg-white border border-gray-200 hover:bg-gray-50 rounded-xl font-bold text-gray-500 shadow-sm transition-all active:scale-95">
          Sign out
        </button>
      </div>

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <Card title="Device">
          <div className="space-y-2 text-sm">
            {([
              ['Model', device?.model],
              ['Firmware', device?.firmware],
              ['Uptime', formatUptime(device?.uptime_secs)],
              ['Load', device?.load_avg?.map(v => v.toFixed(2)).join(', ')],
              ['IMEI', imei],
            ] as const).map(([l, v]) => (
              <div key={l} className="flex justify-between gap-2">
                <span className="text-gray-500">{l}</span>
                <span className="text-right font-medium text-gray-900 font-mono text-xs break-all">{v ?? '—'}</span>
              </div>
            ))}
          </div>
        </Card>

        <Card title="SIM Card">
          <div className="space-y-2 text-sm">
            {([
              ['Status', sim?.state],
              ['ICCID', sim?.iccid],
              ['IMSI', sim?.imsi],
              ['MCC/MNC', sim?.mcc && sim?.mnc ? `${sim.mcc}/${sim.mnc}` : undefined],
            ] as const).map(([l, v]) => (
              <div key={l} className="flex justify-between gap-2">
                <span className="text-gray-500">{l}</span>
                <span className="text-right font-medium text-gray-900 font-mono text-xs break-all">{v ?? '—'}</span>
              </div>
            ))}
          </div>
        </Card>

        <Card title="Memory">
          {mem && (
            <div className="space-y-2">
              <div className="flex items-center justify-between text-sm">
                <span className="text-gray-500">Usage</span>
                <span className="text-gray-900">{formatBytes(mem.used_kb * 1024)} / {formatBytes(mem.total_kb * 1024)} ({mem.usage_pct.toFixed(0)}%)</span>
              </div>
              <div className="h-2 rounded-full bg-gray-100 overflow-hidden">
                <div className="h-full rounded-full bg-slds-blue transition-all duration-500" style={{ width: `${mem.usage_pct}%` }} />
              </div>
            </div>
          )}
        </Card>

        <Card title="Connection">
          <div className="space-y-2 text-sm">
            <div className="flex justify-between gap-2">
              <span className="text-gray-500">API</span>
              <span className="font-mono text-xs text-gray-900">{API_BASE}</span>
            </div>
            <div className="flex justify-between gap-2">
              <span className="text-gray-500">Dashboard</span>
              <span className="font-mono text-xs text-gray-900">{window.location.origin}</span>
            </div>
          </div>
        </Card>
      </div>

      <Card title="Service Controls">
        <div className="space-y-3">
          <div className="flex flex-wrap items-center gap-3">
            <button
              onClick={restartAgent}
              disabled={busyAction !== null}
              className="inline-flex items-center gap-2 rounded-xl bg-amber-50 border border-amber-200 px-4 py-2 text-sm font-medium text-amber-600 hover:bg-amber-100 transition-all duration-200 disabled:opacity-40"
            >
              <RefreshCw className="h-4 w-4" />
              {busyAction === 'restart' ? 'Restarting...' : 'Restart Agent'}
            </button>
            <button
              onClick={() => {
                setControlMsg({ type: 'info', text: 'Reloading dashboard...' })
                setTimeout(() => window.location.reload(), 500)
              }}
              disabled={busyAction !== null}
              className="inline-flex items-center gap-2 px-4 py-2 text-sm bg-white border border-gray-200 hover:bg-gray-50 rounded-xl font-bold text-gray-500 shadow-sm transition-all active:scale-95 disabled:opacity-40"
            >
              <RotateCcw className="h-4 w-4" />
              Reload Dashboard
            </button>
            {confirmAction === 'reboot' ? (
              <div className="flex flex-wrap gap-2">
                <button
                  onClick={() => runPowerAction('reboot')}
                  disabled={busyAction !== null}
                  className="inline-flex items-center gap-2 rounded-xl bg-red-50 px-4 py-2 text-sm font-bold text-red-600 border border-red-200 hover:bg-red-100 transition-all duration-150 disabled:opacity-40"
                >
                  <RotateCcw className="h-4 w-4" />
                  {busyAction === 'reboot' ? 'Rebooting...' : 'Confirm Reboot'}
                </button>
                <button
                  onClick={() => setConfirmAction(null)}
                  disabled={busyAction !== null}
                  className="bg-white border border-gray-200 hover:bg-gray-50 px-3 py-2 rounded-xl font-bold text-gray-500 shadow-macos transition-all active:scale-95 text-sm disabled:opacity-40"
                >
                  Cancel
                </button>
              </div>
            ) : (
              <button
                onClick={() => setConfirmAction('reboot')}
                disabled={busyAction !== null}
                className="inline-flex items-center gap-2 rounded-xl bg-red-50 px-4 py-2 text-sm font-bold text-red-600 border border-red-200 hover:bg-red-100 transition-all duration-150 disabled:opacity-40"
              >
                <RotateCcw className="h-4 w-4" />
                Reboot
              </button>
            )}
            {confirmAction === 'shutdown' ? (
              <div className="flex flex-wrap gap-2">
                <button
                  onClick={() => runPowerAction('shutdown')}
                  disabled={busyAction !== null}
                  className="inline-flex items-center gap-2 rounded-xl bg-red-600 px-4 py-2 text-sm font-bold text-white border border-red-700 hover:bg-red-700 transition-all duration-150 disabled:opacity-40"
                >
                  <Power className="h-4 w-4" />
                  {busyAction === 'shutdown' ? 'Shutting Down...' : 'Confirm Shut Down'}
                </button>
                <button
                  onClick={() => setConfirmAction(null)}
                  disabled={busyAction !== null}
                  className="bg-white border border-gray-200 hover:bg-gray-50 px-3 py-2 rounded-xl font-bold text-gray-500 shadow-macos transition-all active:scale-95 text-sm disabled:opacity-40"
                >
                  Cancel
                </button>
              </div>
            ) : (
              <button
                onClick={() => setConfirmAction('shutdown')}
                disabled={busyAction !== null}
                className="inline-flex items-center gap-2 rounded-xl bg-red-600 px-4 py-2 text-sm font-bold text-white border border-red-700 hover:bg-red-700 transition-all duration-150 disabled:opacity-40"
              >
                <Power className="h-4 w-4" />
                Shut Down
              </button>
            )}
          </div>
          <p className="text-xs text-gray-500">
            Restart Agent briefly interrupts the backend. Reboot and Shut Down interrupt all connections.
          </p>
          {controlMsg && <p className={`text-xs ${controlMessageClass(controlMsg.type)}`}>{controlMsg.text}</p>}
        </div>
      </Card>

      <UsbModeSection />

      {top.length > 0 && (
        <Card title="Top Processes">
          <div className="relative">
          <div className="overflow-x-auto">
            <table className="w-full text-xs">
              <thead>
                <tr className="text-left text-gray-500">
                  <th className="pb-1.5 pr-3 font-medium">PID</th>
                  <th className="pb-1.5 pr-3 font-medium">Name</th>
                  <th className="pb-1.5 pr-3 font-medium text-right">CPU%</th>
                  <th className="pb-1.5 font-medium text-right">Mem</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-100/60">
                {top.map(p => (
                  <tr key={p.pid} className="hover:bg-gray-50/60 transition-colors">
                    <td className="py-1 pr-3 text-gray-500">{p.pid}</td>
                    <td className="py-1 pr-3 font-medium text-gray-900 truncate max-w-[160px]">{p.name}</td>
                    <td className="py-1 pr-3 text-right text-gray-600">{p.cpu_percent?.toFixed(1) ?? '—'}</td>
                    <td className="py-1 text-right text-gray-600">{p.mem_kb ? formatBytes(p.mem_kb * 1024) : '—'}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div className="pointer-events-none absolute inset-y-0 right-0 w-6 bg-gradient-to-l from-white to-transparent sm:hidden" />
          </div>
        </Card>
      )}
    </div>
  )
}
