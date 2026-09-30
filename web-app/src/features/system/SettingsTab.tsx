import { useCallback, useEffect, useState } from 'react'
import { api } from '../../data/api'
import { API_BASE } from '../../data/client'
import { formatUptime } from '../../format'
import type { DeviceInfo, SimInfo, UsbStatus } from '../../types'
import { ILogout, IPower, IRefresh, IRestart } from '../../icons'
import { useTheme, type ThemePref } from '../../app/theme'
import { Button, Segmented, Toggle } from '../../ui/controls'
import { confirm, toast, toastError } from '../../ui/feedback'
import { Card, Row } from '../../ui/primitives'

// ── USB mode + powerbank ──────────────────────────────────────────────────────

type UsbModeKey = 'rndis' | 'ecm' | 'ncm' | 'debug'

const USB_MODE_INFO: Record<UsbModeKey, { label: string; description: string; warning?: string }> = {
  rndis: {
    label: 'RNDIS',
    description: "Microsoft's USB networking. Native on Windows; needs unmaintained drivers on macOS.",
  },
  ecm: {
    label: 'ECM',
    description: 'CDC-ECM USB Ethernet. Driver-free on macOS, Linux and modern Windows. Best supported mode.',
  },
  ncm: {
    label: 'NCM',
    description:
      'CDC-NCM USB Ethernet. Higher throughput in theory. Experimental — ZTE does not wire ncm.0 into the normal USB switch.',
    warning: 'Experimental',
  },
  debug: {
    label: 'Debug (ADB)',
    description: 'USB debug composition with adbd. Reboot returns to normal tethering.',
  },
}

function UsbSection() {
  const [status, setStatus] = useState<UsbStatus | null>(null)
  const [busy, setBusy] = useState(false)
  const [powerbank, setPowerbank] = useState<boolean | null>(null)

  const fetchStatus = useCallback(async () => {
    try {
      setStatus(await api.usbStatus())
    } catch {
      /* ignore */
    }
    try {
      const charger = await api.chargerInfo()
      setPowerbank(Number(charger.otg_powerbank_state ?? 0) === 1)
    } catch {
      setPowerbank(null)
    }
  }, [])

  useEffect(() => {
    fetchStatus()
  }, [fetchStatus])

  async function setMode(mode: UsbModeKey) {
    const capability = status?.mode_capabilities?.find((c) => c.mode === mode)
    const isSupported =
      mode === 'debug' || capability?.supported || status?.supported_modes?.includes(mode)
    if (!isSupported) {
      toast(`${USB_MODE_INFO[mode].label} is not available on this firmware`, 'err')
      return
    }
    const isExperimentalNcm = mode === 'ncm'
    if (isExperimentalNcm) {
      const ok = await confirm({
        title: 'Switch to experimental NCM?',
        body: 'USB will disconnect and re-enumerate. Keep a Wi-Fi management path open.',
        confirmLabel: 'Switch',
      })
      if (!ok) return
    }

    setBusy(true)
    try {
      await api.usbMode(mode, isExperimentalNcm ? { confirm_experimental: true } : undefined)
      toast(
        isExperimentalNcm
          ? 'NCM switch scheduled — USB will re-enumerate shortly'
          : mode === 'ecm' && status?.active_mode === 'ncm'
            ? 'ECM rollback scheduled — USB will re-enumerate shortly'
            : `USB mode set to ${mode.toUpperCase()} — reboot to apply`,
      )
      fetchStatus()
    } catch (e) {
      toastError(e, 'Failed to set USB mode')
    } finally {
      setBusy(false)
    }
  }

  async function setNcmDefault(enabled: boolean) {
    if (enabled) {
      const ok = await confirm({
        title: 'Persist NCM after boot?',
        body: 'NCM will be re-applied automatically after each boot. Keep Wi-Fi management available.',
        confirmLabel: 'Enable',
      })
      if (!ok) return
    }
    setBusy(true)
    try {
      await api.usbDefaultMode(enabled ? 'ncm' : 'ecm', enabled ? { confirm_experimental: true } : undefined)
      toast(enabled ? 'NCM will be applied automatically after boot' : 'USB boot default returned to ECM')
      fetchStatus()
    } catch (e) {
      toastError(e, 'Failed to set USB boot default')
    } finally {
      setBusy(false)
    }
  }

  async function togglePowerbank(on: boolean) {
    try {
      await api.usbPowerbank(on)
      setPowerbank(on)
      toast(on ? 'Powerbank (OTG) enabled' : 'Powerbank disabled')
    } catch (e) {
      toastError(e, 'Failed to set powerbank')
    }
  }

  const activeMode = status?.active_mode ?? null
  const ncmDefaultEnabled = status?.ncm_persist_on_boot ?? status?.default_mode === 'ncm'
  const supported = new Set(status?.supported_modes ?? ['rndis', 'ecm'])
  const modes: UsbModeKey[] = ['rndis', 'ecm', 'ncm', 'debug']

  return (
    <Card title="USB mode">
      <div className="space-y-3">
        <p className="text-meta text-ink2">
          Switch USB operating mode. Most changes need a reboot to take effect.
          {activeMode && (
            <>
              {' '}Active: <span className="font-bold text-ink">{activeMode.toUpperCase()}</span>.
            </>
          )}
        </p>
        {status?.ncm_last_error && <p className="text-meta text-danger">Last NCM attempt: {status.ncm_last_error}</p>}

        <div className="flex flex-wrap gap-1.5">
          {modes.map((mode) => {
            const info = USB_MODE_INFO[mode]
            const isActive = activeMode === mode
            const capability = status?.mode_capabilities?.find((c) => c.mode === mode)
            const isSupported = mode === 'debug' || capability?.supported || supported.has(mode)
            return (
              <button
                key={mode}
                onClick={() => setMode(mode)}
                disabled={busy || !isSupported}
                title={info.description}
                className={`rounded-ctl border px-3 py-1.5 text-meta font-bold transition-colors disabled:opacity-40 ${
                  isActive
                    ? 'border-ok/30 bg-ok/10 text-ok'
                    : !isSupported
                      ? 'border-line/8 bg-surface2/50 text-ink3'
                      : 'border-line/10 bg-surface text-ink2 hover:bg-surface2'
                }`}
              >
                {info.label}
                {!isSupported && ' · n/a'}
              </button>
            )
          })}
        </div>

        <div className="flex flex-wrap items-center justify-between gap-2 border-t border-line/8 pt-3">
          <div>
            <p className="text-body font-semibold text-ink">NCM after boot</p>
            <p className="text-meta text-ink2">Applies NCM after the stock USB stack settles.</p>
          </div>
          <Toggle checked={ncmDefaultEnabled} disabled={busy} onChange={setNcmDefault} label="NCM after boot" />
        </div>

        {powerbank !== null && (
          <div className="flex flex-wrap items-center justify-between gap-2 border-t border-line/8 pt-3">
            <div>
              <p className="text-body font-semibold text-ink">Powerbank / OTG</p>
              <p className="text-meta text-ink2">Drive the USB-C port as a power output.</p>
            </div>
            <Toggle checked={powerbank} onChange={togglePowerbank} label="Powerbank" />
          </div>
        )}
      </div>
    </Card>
  )
}

// ── Settings tab ──────────────────────────────────────────────────────────────

export default function SettingsTab({ onLogout }: { onLogout: () => void }) {
  const { pref: themePref, setPref: setThemePref } = useTheme()
  const [device, setDevice] = useState<DeviceInfo | null>(null)
  const [sim, setSim] = useState<SimInfo | null>(null)
  const [imei, setImei] = useState('')
  const [busy, setBusy] = useState<string | null>(null)

  const fetchAll = useCallback(async () => {
    const [d, s, i] = await Promise.allSettled([api.device(), api.simInfo(), api.simImei()])
    if (d.status === 'fulfilled') setDevice(d.value)
    if (s.status === 'fulfilled') setSim(s.value)
    if (i.status === 'fulfilled' && i.value) setImei((i.value as { imei?: string }).imei ?? '')
  }, [])

  useEffect(() => {
    fetchAll()
  }, [fetchAll])

  async function restartAgent() {
    setBusy('restart')
    try {
      await api.restartAgent()
      toast('Agent restarting — reloading in a few seconds')
      setTimeout(() => window.location.reload(), 5000)
    } catch (e) {
      toastError(e, 'Failed to restart agent')
      setBusy(null)
    }
  }

  async function runPowerAction(action: 'reboot' | 'shutdown') {
    const ok = await confirm({
      title: action === 'reboot' ? 'Reboot the device?' : 'Shut down the device?',
      body:
        action === 'reboot'
          ? 'All connections will drop for about 10-30 seconds.'
          : 'The device powers off. Use the physical power button to turn it back on.',
      confirmLabel: action === 'reboot' ? 'Reboot' : 'Shut down',
      danger: true,
    })
    if (!ok) return
    setBusy(action)
    try {
      if (action === 'reboot') {
        await api.reboot()
        toast('Reboot command sent')
      } else {
        await api.shutdown()
        toast('Shutdown command sent')
      }
    } catch (e) {
      toastError(e, `Failed to ${action} device`)
    } finally {
      setBusy(null)
    }
  }

  return (
    <div className="space-y-3">
      <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
        <Card title="Device">
          <Row label="Model" value={device?.model ?? '\u2014'} mono />
          <Row label="Firmware" value={device?.firmware ?? '\u2014'} mono />
          <Row label="Uptime" value={formatUptime(device?.uptime_secs)} />
          <Row label="Load" value={device?.load_avg?.map((v) => v.toFixed(2)).join(', ') ?? '\u2014'} mono />
          <Row label="IMEI" value={imei || '\u2014'} mono />
        </Card>

        <Card title="SIM card">
          <Row label="Status" value={sim?.state ?? '\u2014'} />
          <Row label="ICCID" value={sim?.iccid ?? '\u2014'} mono />
          <Row label="IMSI" value={sim?.imsi ?? '\u2014'} mono />
          <Row label="MCC/MNC" value={sim?.mcc && sim?.mnc ? `${sim.mcc}/${sim.mnc}` : '\u2014'} mono />
        </Card>
      </div>

      <UsbSection />

      <Card title="Service controls">
        <div className="flex flex-wrap items-center gap-2">
          <Button variant="outline" onClick={restartAgent} loading={busy === 'restart'}>
            <IRefresh size={14} /> Restart agent
          </Button>
          <Button
            variant="outline"
            onClick={() => {
              toast('Reloading dashboard…')
              setTimeout(() => window.location.reload(), 400)
            }}
          >
            <IRestart size={14} /> Reload dashboard
          </Button>
          <Button variant="danger" onClick={() => runPowerAction('reboot')} loading={busy === 'reboot'}>
            <IRestart size={14} /> Reboot
          </Button>
          <Button variant="danger" onClick={() => runPowerAction('shutdown')} loading={busy === 'shutdown'}>
            <IPower size={14} /> Shut down
          </Button>
        </div>
        <p className="mt-2.5 text-meta text-ink3">
          Restart agent briefly interrupts the backend. Reboot and shut down interrupt all connections.
        </p>
      </Card>

      <Card title="Appearance">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <span className="text-body text-ink2">Theme</span>
          <Segmented<ThemePref>
            options={[
              { value: 'auto', label: 'Auto' },
              { value: 'light', label: 'Light' },
              { value: 'dark', label: 'Dark' },
            ]}
            value={themePref}
            onChange={setThemePref}
          />
        </div>
        <p className="mt-2 text-meta text-ink3">Auto follows this device's light or dark setting.</p>
      </Card>

      <Card title="Connection">
        <Row label="API" value={API_BASE} mono />
        <Row label="Dashboard" value={window.location.origin} mono />
        <div className="mt-3 border-t border-line/8 pt-3">
          <Button variant="ghost" onClick={onLogout}>
            <ILogout size={14} /> Sign out
          </Button>
        </div>
      </Card>
    </div>
  )
}
