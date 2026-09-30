import { useCallback, useEffect, useState } from 'react'
import { api } from '../../data/api'
import type { WifiAll, WifiBand } from '../../types'
import { Button, Field, Input, Select, Toggle } from '../../ui/controls'
import { toast, toastError } from '../../ui/feedback'
import { Card, Chip, Skeleton } from '../../ui/primitives'

const DFS_5G_CHANNELS = new Set(['52', '56', '60', '64', '100', '104', '108', '112', '116', '120', '124', '128', '132', '136', '140', '144'])

function normalizeConfiguredChannel(channel?: string) {
  const raw = (channel ?? '').trim().toLowerCase()
  return !raw || raw === '0' || raw === 'auto' ? 'auto' : raw
}

function formatBandwidthMode(mode?: string) {
  if (!mode) return '\u2014'
  if (/^(EHT|HE|VHT|HT)\d+$/.test(mode)) return `${mode.replace(/^(EHT|HE|VHT|HT)/, '')} MHz (${mode.match(/^(EHT|HE|VHT|HT)/)?.[0]})`
  return mode
}

function getBandInsights(suffix: '2g' | '5g', band: WifiBand): string[] {
  const insights: string[] = []
  const configuredChannel = normalizeConfiguredChannel(band.configuredChannel)
  const actualChannel = band.actualChannel ?? band.channel

  if (configuredChannel === 'auto' && actualChannel != null) {
    insights.push(`Auto channel selected ${actualChannel} at runtime.`)
  }
  if (configuredChannel !== 'auto') {
    const configuredNum = parseInt(configuredChannel, 10)
    if (!Number.isNaN(configuredNum)) {
      if (actualChannel != null && configuredNum !== actualChannel) {
        insights.push(`Configured channel ${configuredNum}, currently operating on ${actualChannel}.`)
      }
      if (suffix === '2g' && ![1, 6, 11].includes(configuredNum)) {
        insights.push('2.4 GHz usually performs best on channels 1, 6, or 11 to reduce overlap.')
      }
      if (suffix === '5g' && DFS_5G_CHANNELS.has(String(configuredNum))) {
        insights.push('DFS channel selected — radar events can force channel changes.')
      }
    }
  }
  const configuredBw = (band.configuredBandwidth ?? '').toUpperCase()
  const actualBw = (band.actualBandwidth ?? band.bandwidth ?? '').toUpperCase()
  if (configuredBw && actualBw && configuredBw !== actualBw) {
    insights.push(`Configured bandwidth ${configuredBw}, runtime reports ${actualBw}.`)
  }
  if ((band.clients ?? 0) >= 15) {
    insights.push('High client count detected. Fixed channels can improve stability.')
  }
  return insights
}

// ── Band card ─────────────────────────────────────────────────────────────────

function BandCard({
  label,
  band,
  suffix,
  masterEnabled,
  onRefresh,
}: {
  label: string
  band: WifiBand
  suffix: '2g' | '5g'
  masterEnabled: boolean
  onRefresh: () => void
}) {
  const [editing, setEditing] = useState(false)
  const [ssid, setSsid] = useState('')
  const [password, setPassword] = useState('')
  const [passwordDirty, setPasswordDirty] = useState(false)
  const [channel, setChannel] = useState('')
  const [htmode, setHtmode] = useState('')
  const [txpower, setTxpower] = useState('')
  const [hidden, setHidden] = useState(false)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    setSsid(band.ssid ?? '')
    setPassword(band.password ?? '')
    setPasswordDirty(false)
    setChannel(normalizeConfiguredChannel(band.configuredChannel))
    setHtmode(band.configuredBandwidth ?? '')
    setTxpower('')
    setHidden(band.hidden)
  }, [band])

  async function handleSave() {
    setBusy(true)
    try {
      const settings: Record<string, unknown> = {
        [`ssid_${suffix}`]: ssid,
        [`hidden_${suffix}`]: hidden ? '1' : '0',
      }
      if (passwordDirty) settings[`key_${suffix}`] = password
      if (channel && channel !== normalizeConfiguredChannel(band.configuredChannel)) settings[`channel_${suffix}`] = channel
      if (htmode && htmode !== band.configuredBandwidth) settings[`htmode_${suffix}`] = htmode
      if (txpower) settings[`txpower_${suffix}`] = txpower
      await api.wifiSet(settings)
      toast('Saved — Wi-Fi may reconnect')
      setPasswordDirty(false)
      setEditing(false)
      onRefresh()
    } catch (e) {
      toastError(e, 'Save failed')
    } finally {
      setBusy(false)
    }
  }

  async function toggleRadio() {
    setBusy(true)
    try {
      const key = suffix === '2g' ? 'radio2_disabled' : 'radio5_disabled'
      await api.wifiSet({ [key]: band.enabled ? '1' : '0' })
      toast(band.enabled ? 'Radio disabled' : 'Radio enabled')
      onRefresh()
    } catch (e) {
      toastError(e)
    } finally {
      setBusy(false)
    }
  }

  const channels =
    suffix === '2g'
      ? ['auto', '1', '2', '3', '4', '5', '6', '7', '8', '9', '10', '11', '12', '13']
      : ['auto', '36', '40', '44', '48', '52', '56', '60', '64', '100', '104', '108', '112', '116', '120', '124', '128', '132', '136', '140', '144', '149', '153', '157', '161', '165']
  const htmodes = band.bandwidthOptions?.length ? band.bandwidthOptions : [band.configuredBandwidth].filter(Boolean) as string[]
  const configuredChannel = normalizeConfiguredChannel(band.configuredChannel)
  const insights = getBandInsights(suffix, band)

  return (
    <Card
      title={label}
      action={
        !editing ? (
          <Button size="sm" variant="ghost" onClick={() => setEditing(true)}>
            Edit
          </Button>
        ) : (
          <div className="flex gap-1.5">
            <Button
              size="sm"
              variant="ghost"
              onClick={() => {
                setEditing(false)
                setSsid(band.ssid ?? '')
                setPassword(band.password ?? '')
                setPasswordDirty(false)
                setChannel(normalizeConfiguredChannel(band.configuredChannel))
                setHtmode(band.configuredBandwidth ?? '')
                setTxpower('')
                setHidden(band.hidden)
              }}
            >
              Cancel
            </Button>
            <Button size="sm" variant="primary" onClick={handleSave} loading={busy}>
              Save
            </Button>
          </div>
        )
      }
    >
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <span
              className={`h-2 w-2 rounded-full ${masterEnabled ? (band.enabled ? 'bg-ok' : 'bg-danger') : 'bg-warn'}`}
            />
            <span className="text-body text-ink2">
              {masterEnabled ? (band.enabled ? 'Enabled' : 'Disabled') : 'Master off'}
            </span>
            {band.clients != null && (
              <span className="text-meta text-ink3">
                {band.clients} client{band.clients !== 1 ? 's' : ''}
              </span>
            )}
          </div>
          <Toggle checked={band.enabled} onChange={toggleRadio} disabled={busy} label={`Toggle ${label} radio`} />
        </div>
        {!masterEnabled && (
          <p className="text-meta text-warn">Global Wi-Fi is off. Band settings are still saved.</p>
        )}

        {editing ? (
          <>
            <Field label="SSID">
              <Input value={ssid} onChange={(e) => setSsid(e.target.value)} />
            </Field>
            <Field label="Password" hint="Leave unchanged to keep the current password">
              <Input
                type="password"
                value={password}
                onChange={(e) => {
                  setPassword(e.target.value)
                  setPasswordDirty(true)
                }}
              />
            </Field>
            <div className="grid grid-cols-2 gap-2 border-t border-line/8 pt-3">
              <Field label="Channel">
                <Select value={channel} onChange={(e) => setChannel(e.target.value)}>
                  {channels.map((c) => (
                    <option key={c} value={c}>
                      {c === 'auto' ? 'Auto' : c}
                    </option>
                  ))}
                </Select>
              </Field>
              <Field label="Bandwidth">
                <Select value={htmode} onChange={(e) => setHtmode(e.target.value)}>
                  {htmodes.map((m) => (
                    <option key={m} value={m}>
                      {formatBandwidthMode(m)}
                    </option>
                  ))}
                </Select>
              </Field>
              <Field label="TX power">
                <Select value={txpower} onChange={(e) => setTxpower(e.target.value)}>
                  <option value="">Default</option>
                  <option value="100">100%</option>
                  <option value="75">75%</option>
                  <option value="50">50%</option>
                  <option value="25">25%</option>
                </Select>
              </Field>
              <div className="flex items-end gap-2 pb-1.5">
                <Toggle checked={hidden} onChange={setHidden} label="Hidden SSID" />
                <span className="text-meta font-medium text-ink2">Hidden SSID</span>
              </div>
            </div>
          </>
        ) : (
          <>
            <div className="grid grid-cols-2 gap-x-3 gap-y-2">
              <Info label="SSID" value={band.ssid ?? '\u2014'} strong />
              <Info label="Password" value={band.password ?? '\u2014'} mono />
              <Info label="Channel" value={configuredChannel === 'auto' ? `Auto (${band.actualChannel ?? band.channel ?? '\u2014'})` : configuredChannel} />
              <Info label="Bandwidth" value={formatBandwidthMode(band.configuredBandwidth)} />
              <Info label="Security" value={band.security ?? '\u2014'} />
              <Info label="Hidden" value={band.hidden ? 'Yes' : 'No'} />
            </div>
            <div className="rounded-ctl bg-surface2/70 px-3 py-2">
              <p className="label mb-1">Channel insights</p>
              {insights.length > 0 ? (
                <div className="space-y-1">
                  {insights.map((insight, i) => (
                    <p key={i} className="text-meta text-ink2">
                      {insight}
                    </p>
                  ))}
                </div>
              ) : (
                <p className="text-meta text-ink3">No obvious channel conflicts detected.</p>
              )}
            </div>
          </>
        )}
      </div>
    </Card>
  )
}

function Info({ label, value, strong = false, mono = false }: { label: string; value: string; strong?: boolean; mono?: boolean }) {
  return (
    <div className="min-w-0">
      <p className="label">{label}</p>
      <p className={`truncate text-body ${strong ? 'font-semibold text-ink' : 'text-ink2'} ${mono ? 'font-mono text-meta' : ''}`}>
        {value}
      </p>
    </div>
  )
}

// ── Tab ───────────────────────────────────────────────────────────────────────

export default function WifiTab() {
  const [wifi, setWifi] = useState<WifiAll | null>(null)
  const [busy, setBusy] = useState(false)
  const [syncBusy, setSyncBusy] = useState(false)

  const refresh = useCallback(() => {
    api.wifiStatus().then(setWifi).catch(() => {})
  }, [])

  useEffect(() => {
    refresh()
  }, [refresh])

  async function toggleMaster() {
    if (!wifi) return
    const next = !wifi.master_enabled
    setBusy(true)
    try {
      await api.wifiSet({ wifi_onoff: next ? '1' : '0' })
      toast(next ? 'Global Wi-Fi enabled' : 'Global Wi-Fi disabled')
      refresh()
    } catch (e) {
      toastError(e)
    } finally {
      setBusy(false)
    }
  }

  async function syncBands(source: '2g' | '5g') {
    if (!wifi) return
    const sourceBand = source === '2g' ? wifi.band_2g : wifi.band_5g
    const sourceLabel = source === '2g' ? '2.4 GHz' : '5 GHz'
    const targetSuffix = source === '2g' ? '5g' : '2g'
    const targetLabel = source === '2g' ? '5 GHz' : '2.4 GHz'

    if (!sourceBand.ssid) {
      toast(`Cannot sync from ${sourceLabel}: source SSID is empty`, 'err')
      return
    }

    const payload: Record<string, unknown> = {
      [`ssid_${targetSuffix}`]: sourceBand.ssid,
      [`hidden_${targetSuffix}`]: sourceBand.hidden ? '1' : '0',
    }
    if (sourceBand.security) payload[`encryption_${targetSuffix}`] = sourceBand.security
    const includePassword = Boolean(sourceBand.password && sourceBand.password !== '••••••••')
    if (includePassword) payload[`key_${targetSuffix}`] = sourceBand.password

    setSyncBusy(true)
    try {
      await api.wifiSet(payload)
      toast(`Copied ${sourceLabel} settings to ${targetLabel}${includePassword ? ' (including password)' : ''}`)
      refresh()
    } catch (e) {
      toastError(e)
    } finally {
      setSyncBusy(false)
    }
  }

  if (!wifi) {
    return (
      <div className="space-y-3">
        <Skeleton className="h-24" />
        <Skeleton className="h-72" />
      </div>
    )
  }

  return (
    <div className="space-y-3">
      <Card title="Global Wi-Fi">
        <div className="flex items-center justify-between gap-3">
          <div>
            <p className="text-body font-medium text-ink">Master switch</p>
            <p className="mt-0.5 text-meta text-ink2">
              {!wifi.master_supported
                ? 'This firmware does not expose a reliable global Wi-Fi toggle.'
                : wifi.master_enabled
                  ? 'On — radios follow your per-band settings'
                  : 'Off — all Wi-Fi radios are globally disabled'}
            </p>
          </div>
          <Toggle checked={wifi.master_enabled} onChange={toggleMaster} disabled={busy || !wifi.master_supported} label="Master Wi-Fi switch" />
        </div>
        {wifi.wifi6_supported && (
          <div className="mt-3 border-t border-line/8 pt-3">
            <Chip tone={wifi.wifi6_enabled ? 'ok' : 'default'}>Wi-Fi 6 {wifi.wifi6_enabled ? 'enabled' : 'disabled'}</Chip>
          </div>
        )}
        {wifi.wifi7_supported && (
          <div className="mt-3 border-t border-line/8 pt-3">
            <Chip tone="ok">Wi-Fi 7 / 802.11be supported</Chip>
          </div>
        )}
      </Card>

      <Card title="Band sync">
        <p className="mb-2.5 text-meta text-ink2">
          Copy SSID, password, security and hidden-state from one band to the other.
        </p>
        <div className="flex flex-wrap gap-2">
          <Button variant="outline" onClick={() => syncBands('2g')} loading={syncBusy}>
            Use 2.4 GHz for both
          </Button>
          <Button variant="outline" onClick={() => syncBands('5g')} loading={syncBusy}>
            Use 5 GHz for both
          </Button>
        </div>
      </Card>

      <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
        <BandCard label="2.4 GHz" band={wifi.band_2g} suffix="2g" masterEnabled={wifi.master_enabled} onRefresh={refresh} />
        <BandCard label="5 GHz" band={wifi.band_5g} suffix="5g" masterEnabled={wifi.master_enabled} onRefresh={refresh} />
      </div>

      {wifi.guest_ssid && (
        <Card title="Guest network">
          <p className="text-body text-ink2">
            SSID: <span className="font-semibold text-ink">{wifi.guest_ssid}</span>
          </p>
        </Card>
      )}
    </div>
  )
}
