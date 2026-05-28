import { useState, useEffect, useCallback } from 'react'
import { Power, RefreshCw, RotateCcw } from 'lucide-react'
import { api, formatBytes, type DeviceInfo, type SimInfo, type UsbStatus, API_BASE } from '../api'
import Card from '../components/Card'

interface Props { onLogout: () => void }
type ControlAction = 'restart' | 'reboot' | 'shutdown'
type ControlMessage = { type: 'success' | 'error' | 'info'; text: string }

function controlMessageClass(type: ControlMessage['type']) {
  if (type === 'success') return 'text-green-500'
  if (type === 'error') return 'text-red-500'
  return 'text-amber-500'
}

type UsbModeKey = 'rndis' | 'ecm' | 'ncm' | 'debug'

const USB_MODE_INFO: Record<UsbModeKey, { label: string; description: string; warning?: string }> = {
  rndis: {
    label: 'RNDIS',
    description: "Microsoft's USB networking. Native on Windows; needs unmaintained drivers on macOS — avoid for Mac.",
  },
  ecm: {
    label: 'ECM',
    description: 'CDC-ECM USB Ethernet. Driver-free on macOS, Linux, and modern Windows. Best supported mode on this firmware.',
  },
  ncm: {
    label: 'NCM',
    description: 'CDC-NCM USB Ethernet. Higher throughput than ECM in theory. This firmware exposes ncm.0 in configfs, but ZTE does not wire it into the normal USB switch.',
    warning: 'Experimental',
  },
  debug: {
    label: 'DEBUG',
    description: 'ADB / serial console mode. Not for network tethering.',
  },
}

function UsbModeSection() {
  const [msg, setMsg] = useState<ControlMessage | null>(null)
  const [loading, setLoading] = useState(false)
  const [defaultLoading, setDefaultLoading] = useState(false)
  const [status, setStatus] = useState<UsbStatus | null>(null)

  const fetchStatus = useCallback(async () => {
    try { setStatus(await api.usbStatus()) } catch { /* ignore */ }
  }, [])

  useEffect(() => { fetchStatus() }, [fetchStatus])

  async function setMode(mode: UsbModeKey) {
    setMsg(null)
    const capability = status?.mode_capabilities?.find(c => c.mode === mode)
    const isSupported = mode === 'debug' || capability?.supported || status?.supported_modes?.includes(mode)
    if (!isSupported) {
      setMsg({ type: 'error', text: `${USB_MODE_INFO[mode].label} is not available on this firmware.` })
      return
    }
    const isExperimentalNcm = mode === 'ncm'
    if (isExperimentalNcm) {
      const confirmed = window.confirm(
        'Experimental NCM will disconnect and re-enumerate USB. Keep a Wi-Fi management path open before continuing.',
      )
      if (!confirmed) return
    }

    setLoading(true)
    try {
      await api.usbMode(mode, isExperimentalNcm ? { confirm_experimental: true } : undefined)
      const note = isExperimentalNcm
        ? 'Experimental NCM switch scheduled. USB will disconnect and should re-enumerate shortly.'
        : mode === 'ecm' && activeMode === 'ncm'
          ? 'ECM rollback scheduled. USB will disconnect and should re-enumerate shortly.'
          : `USB mode set to ${mode.toUpperCase()}. A reboot is required for the change to take effect.`
      setMsg({ type: 'success', text: note })
      fetchStatus()
    } catch (e) {
      setMsg({ type: 'error', text: e instanceof Error ? e.message : 'Failed to set USB mode' })
    } finally {
      setLoading(false)
    }
  }

  async function setNcmDefault(enabled: boolean) {
    setMsg(null)
    const capability = status?.mode_capabilities?.find(c => c.mode === 'ncm')
    const ncmSupported = capability?.supported || status?.supported_modes?.includes('ncm')
    if (enabled && !ncmSupported) {
      setMsg({ type: 'error', text: 'NCM is not available on this firmware.' })
      return
    }
    if (enabled) {
      const confirmed = window.confirm(
        'Persisting NCM will re-enumerate USB automatically after each boot. Keep Wi-Fi management available before enabling it.',
      )
      if (!confirmed) return
    }

    setDefaultLoading(true)
    try {
      await api.usbDefaultMode(enabled ? 'ncm' : 'ecm', enabled ? { confirm_experimental: true } : undefined)
      setMsg({
        type: 'success',
        text: enabled
          ? 'NCM will be applied automatically after boot.'
          : 'USB boot default returned to ECM.',
      })
      fetchStatus()
    } catch (e) {
      setMsg({ type: 'error', text: e instanceof Error ? e.message : 'Failed to set USB boot default' })
    } finally {
      setDefaultLoading(false)
    }
  }

  const activeMode = status?.active_mode ?? null
  const ncmDefaultEnabled = status?.ncm_persist_on_boot ?? status?.default_mode === 'ncm'
  const supported = new Set(status?.supported_modes ?? ['rndis', 'ecm'])
  const experimental = new Set(status?.experimental_modes ?? [])
  const modes: UsbModeKey[] = ['rndis', 'ecm', 'ncm', 'debug']
  const composition = status?.composition_functions?.join(', ')
  const bridgeMembers = status?.bridge?.members?.join(', ')

  return (
    <Card title="USB Mode">
      <div className="space-y-3">
        <p className="text-xs text-gray-500">
          Switch USB operating mode. A reboot is required for the change to take effect.
          {activeMode && (
            <> Currently active: <span className="font-bold text-gray-700">{activeMode.toUpperCase()}</span>.</>
          )}
          {status?.default_mode && (
            <> Boot default: <span className="font-bold text-gray-700">{status.default_mode.toUpperCase()}</span>.</>
          )}
        </p>
        {(composition || bridgeMembers) && (
          <p className="text-[11px] text-gray-500">
            {composition && <>Composition: <span className="font-mono text-gray-600">{composition}</span>.</>}
            {bridgeMembers && <> Bridge: <span className="font-mono text-gray-600">{bridgeMembers}</span>.</>}
          </p>
        )}
        {status?.ncm_last_error && (
          <p className="text-xs text-red-500">Last NCM attempt: {status.ncm_last_error}</p>
        )}
        {msg && <p className={`text-sm ${controlMessageClass(msg.type)}`}>{msg.text}</p>}
        <div className="flex flex-wrap gap-2">
          {modes.map(mode => {
            const info = USB_MODE_INFO[mode]
            const isActive = activeMode === mode
            const capability = status?.mode_capabilities?.find(c => c.mode === mode)
            const isExperimental = experimental.has(mode)
            const isDebugMode = mode === 'debug'
            const isSupported = isDebugMode || capability?.supported || supported.has(mode)
            const isUnsupported = !isSupported && !isDebugMode
            const baseCls = 'inline-flex flex-col items-start gap-0.5 px-3 py-2 rounded-xl font-bold shadow-macos transition-all active:scale-95 text-sm border'
            const stateCls = isActive
              ? 'bg-green-50 border-green-300 text-green-700 hover:bg-green-100'
              : isExperimental
                ? 'bg-amber-50 border-amber-200 text-amber-700 hover:bg-amber-100'
              : isUnsupported
                ? 'bg-gray-50 border-gray-200 text-gray-400 hover:bg-gray-100'
                : 'bg-white border-gray-200 text-gray-700 hover:bg-gray-50'
            return (
              <button
                key={mode}
                onClick={() => setMode(mode)}
                disabled={loading || isUnsupported}
                title={capability?.note ? `${info.description} ${capability.note}` : info.description}
                className={`${baseCls} ${stateCls} disabled:opacity-40`}
              >
                <span className="flex items-center gap-1.5">
                  {info.label}
                  {isActive && <span className="h-1.5 w-1.5 rounded-full bg-green-500" />}
                </span>
                {info.warning && (
                  <span className="text-[10px] font-normal text-amber-600 normal-case">{info.warning}</span>
                )}
                {isUnsupported && (
                  <span className="text-[10px] font-normal text-gray-400 normal-case">Unavailable</span>
                )}
                {isActive && (
                  <span className="text-[10px] font-normal text-green-600 normal-case">Active</span>
                )}
              </button>
            )
          })}
        </div>
        <div className="flex flex-col gap-2 border-t border-gray-100 pt-3 sm:flex-row sm:items-center sm:justify-between">
          <div>
            <p className="text-sm font-bold text-gray-700">NCM after boot</p>
            <p className="text-xs text-gray-500">Applies the NCM composition after the stock USB stack has settled.</p>
          </div>
          <button
            onClick={() => setNcmDefault(!ncmDefaultEnabled)}
            disabled={defaultLoading || (!ncmDefaultEnabled && !supported.has('ncm'))}
            className={`inline-flex min-w-20 items-center justify-center rounded-xl px-3 py-2 text-sm font-bold shadow-macos transition-all active:scale-95 disabled:opacity-40 ${
              ncmDefaultEnabled
                ? 'bg-amber-50 text-amber-700 hover:bg-amber-100'
                : 'bg-white border border-gray-200 text-gray-700 hover:bg-gray-50'
            }`}
          >
            {defaultLoading ? 'Saving...' : ncmDefaultEnabled ? 'On' : 'Off'}
          </button>
        </div>
      </div>
    </Card>
  )
}

export default function SettingsPage({ onLogout }: Props) {
  const [device, setDevice] = useState<DeviceInfo | null>(null)
  const [sim, setSim] = useState<SimInfo | null>(null)
  const [imei, setImei] = useState('')
  const [top, setTop] = useState<{ pid?: number; name?: string; cpu_percent?: number; mem_kb?: number }[]>([])
  const [controlMsg, setControlMsg] = useState<ControlMessage | null>(null)
  const [confirmAction, setConfirmAction] = useState<'reboot' | 'shutdown' | null>(null)
  const [busyAction, setBusyAction] = useState<ControlAction | null>(null)

  const fetchAll = useCallback(async () => {
    const results = await Promise.allSettled([
      api.device(), api.simInfo(), api.simImei(), api.top(),
    ])
    const [d, s, i, p] = results
    if (d.status === 'fulfilled') setDevice(d.value)
    if (s.status === 'fulfilled') setSim(s.value)
    if (i.status === 'fulfilled' && i.value) setImei(i.value.imei ?? '')
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
