import { useState, useEffect, useCallback } from 'react'
import { api, type WifiAll, type WifiBand } from '../api'
import Card from '../components/Card'

const INPUT_CLS = 'w-full px-3.5 py-2.5 bg-gray-50 border border-gray-200 rounded-xl focus:ring-0 focus:shadow-macos-focus focus:border-slds-blue outline-none text-sm transition-all'
const SELECT_CLS = 'w-full px-3.5 py-2.5 bg-gray-50 border border-gray-200 rounded-xl focus:ring-0 focus:shadow-macos-focus focus:border-slds-blue outline-none text-sm transition-all'
const DFS_5G_CHANNELS = new Set(['52', '56', '60', '64', '100', '104', '108', '112', '116', '120', '124', '128', '132', '136', '140', '144'])

function normalizeConfiguredChannel(channel?: string) {
  const raw = (channel ?? '').trim().toLowerCase()
  return (!raw || raw === '0' || raw === 'auto') ? 'auto' : raw
}

function formatChannel(channel?: number) {
  return channel != null ? String(channel) : '—'
}

function formatBandwidth(mode?: string) {
  if (!mode) return '—'
  if (mode.startsWith('HT')) return `${mode.replace('HT', '')} MHz`
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
        insights.push('DFS channel selected. Radar events can force channel changes and brief reconnects.')
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

function BandCard({ label, band, suffix, masterEnabled, onRefresh }: { label: string; band: WifiBand; suffix: '2g' | '5g'; masterEnabled: boolean; onRefresh: () => void }) {
  const [editing, setEditing] = useState(false)
  const [ssid, setSsid] = useState('')
  const [password, setPassword] = useState('')
  const [passwordDirty, setPasswordDirty] = useState(false)
  const [channel, setChannel] = useState('')
  const [htmode, setHtmode] = useState('')
  const [txpower, setTxpower] = useState('')
  const [hidden, setHidden] = useState(false)
  const [saving, setSaving] = useState(false)
  const [msg, setMsg] = useState('')

  useEffect(() => {
    setSsid(band.ssid ?? '')
    setPassword(band.password ?? '')
    setPasswordDirty(false)
    setChannel(normalizeConfiguredChannel(band.configuredChannel))
    setHtmode((band.configuredBandwidth ?? '').startsWith('HT') ? (band.configuredBandwidth ?? '') : '')
    setTxpower('')
    setHidden(band.hidden)
  }, [band])

  async function handleSave() {
    setSaving(true)
    setMsg('')
    try {
      const settings: Record<string, unknown> = {
        [`ssid_${suffix}`]: ssid,
        [`hidden_${suffix}`]: hidden ? '1' : '0',
      }
      if (passwordDirty) settings[`key_${suffix}`] = password
      if (channel) settings[`channel_${suffix}`] = channel
      if (htmode) settings[`htmode_${suffix}`] = htmode
      if (txpower) settings[`txpower_${suffix}`] = txpower
      await api.wifiSet(settings)
      setMsg('Saved \u2014 WiFi may reconnect')
      setPasswordDirty(false)
      setEditing(false)
      onRefresh()
    } catch (e) {
      setMsg(e instanceof Error ? e.message : 'Error')
    } finally {
      setSaving(false)
    }
  }

  function handleCancel() {
    setSsid(band.ssid ?? '')
    setPassword(band.password ?? '')
    setPasswordDirty(false)
    setChannel(normalizeConfiguredChannel(band.configuredChannel))
    setHtmode((band.configuredBandwidth ?? '').startsWith('HT') ? (band.configuredBandwidth ?? '') : '')
    setTxpower('')
    setHidden(band.hidden)
    setMsg('')
    setEditing(false)
  }

  async function toggleRadio() {
    setSaving(true)
    setMsg('')
    try {
      const key = suffix === '2g' ? 'radio2_disabled' : 'radio5_disabled'
      await api.wifiSet({ [key]: band.enabled ? '1' : '0' })
      setMsg(band.enabled ? 'Radio disabled' : 'Radio enabled')
      onRefresh()
    } catch (e) {
      setMsg(e instanceof Error ? e.message : 'Error')
    } finally {
      setSaving(false)
    }
  }

  const channels2g = ['auto', '1', '2', '3', '4', '5', '6', '7', '8', '9', '10', '11', '12', '13']
  const channels5g = ['auto', '36', '40', '44', '48', '52', '56', '60', '64', '100', '104', '108', '112', '116', '120', '124', '128', '132', '136', '140', '144', '149', '153', '157', '161', '165']
  const channels = suffix === '2g' ? channels2g : channels5g
  const htmodes2g = ['HT20', 'HT40']
  const htmodes5g = ['HT20', 'HT40', 'HT80', 'HT160']
  const htmodes = suffix === '2g' ? htmodes2g : htmodes5g
  const configuredChannel = normalizeConfiguredChannel(band.configuredChannel)
  const configuredChannelText = configuredChannel === 'auto' ? 'Auto' : configuredChannel
  const configuredBandwidthText = formatBandwidth(band.configuredBandwidth)
  const actualChannelText = formatChannel(band.actualChannel ?? band.channel)
  const actualBandwidthText = formatBandwidth(band.actualBandwidth ?? band.bandwidth)
  const insights = getBandInsights(suffix, band)

  return (
    <Card title={label} action={
      !editing ? (
        <button onClick={() => setEditing(true)} className="py-2 px-2 text-xs text-slds-blue hover:text-slds-blueHover transition-colors font-bold">Edit</button>
      ) : (
        <div className="flex gap-2">
          <button onClick={handleCancel} className="py-2 px-2 text-xs text-gray-500 hover:text-gray-900 transition-colors">Cancel</button>
          <button onClick={handleSave} disabled={saving} className="py-2 px-2 text-xs text-slds-blue hover:text-slds-blueHover transition-colors font-bold">
            {saving ? 'Saving\u2026' : 'Save'}
          </button>
        </div>
      )
    }>
      {msg && <p className={`mb-2 text-xs ${msg.startsWith('Saved') || msg.includes('abled') ? 'text-green-500' : 'text-red-500'}`}>{msg}</p>}

      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <span className={`h-2 w-2 rounded-full ${masterEnabled ? (band.enabled ? 'bg-green-500' : 'bg-red-500') : 'bg-amber-500'}`} />
            <span className="text-sm text-gray-600">{masterEnabled ? (band.enabled ? 'Enabled' : 'Disabled') : 'Master Off'}</span>
            {band.clients != null && (
              <span className="text-xs text-gray-500">({band.clients} client{band.clients !== 1 ? 's' : ''})</span>
            )}
          </div>
          <button onClick={toggleRadio} disabled={saving}
            className={`rounded-xl px-3 py-1.5 text-xs font-bold transition-all duration-150 ${
              band.enabled
                ? 'bg-red-50 text-red-600 border border-red-200 hover:bg-red-100'
                : 'bg-green-50 text-green-600 border border-green-200 hover:bg-green-100'
            } disabled:opacity-40`}>
            {band.enabled ? 'Disable' : 'Enable'}
          </button>
        </div>
        {!masterEnabled && (
          <p className="text-xs text-amber-600">Global Wi-Fi is off. Band settings are still saved.</p>
        )}

        <div>
          <label className="text-[9px] font-bold text-gray-500 uppercase tracking-widest mb-0.5 block">SSID</label>
          {editing ? (
            <input value={ssid} onChange={e => setSsid(e.target.value)} className={INPUT_CLS} />
          ) : (
            <p className="text-sm font-medium text-gray-900">{band.ssid ?? '\u2014'}</p>
          )}
        </div>

        <div>
          <label className="text-[9px] font-bold text-gray-500 uppercase tracking-widest mb-0.5 block">Password</label>
          {editing ? (
            <input
              type="password"
              value={password}
              onChange={e => {
                setPassword(e.target.value)
                setPasswordDirty(true)
              }}
              className={INPUT_CLS}
            />
          ) : (
            <p className="text-sm text-gray-900 font-mono">{band.password ?? '\u2014'}</p>
          )}
        </div>

        {editing ? (
          <div className="space-y-3 border-t border-gray-200/60 pt-3">
            <p className="text-[9px] font-bold text-gray-500 uppercase tracking-widest">Advanced</p>
            <div className="grid grid-cols-2 gap-3">
              <div>
                <label className="text-[9px] font-bold text-gray-500 uppercase tracking-widest mb-0.5 block">Channel</label>
                <select value={channel} onChange={e => setChannel(e.target.value)} className={SELECT_CLS}>
                  {channels.map(c => <option key={c} value={c}>{c === 'auto' ? 'Auto' : c}</option>)}
                </select>
              </div>
              <div>
                <label className="text-[9px] font-bold text-gray-500 uppercase tracking-widest mb-0.5 block">Bandwidth</label>
                <select value={htmode} onChange={e => setHtmode(e.target.value)} className={SELECT_CLS}>
                  {htmodes.map(m => <option key={m} value={m}>{m.replace('HT', '')} MHz</option>)}
                </select>
              </div>
              <div>
                <label className="text-[9px] font-bold text-gray-500 uppercase tracking-widest mb-0.5 block">TX Power</label>
                <select value={txpower} onChange={e => setTxpower(e.target.value)} className={SELECT_CLS}>
                  <option value="">Default</option>
                  <option value="100">100%</option>
                  <option value="75">75%</option>
                  <option value="50">50%</option>
                  <option value="25">25%</option>
                </select>
              </div>
              <div className="flex items-center gap-2 pt-5 min-h-[44px]">
                <input type="checkbox" id={`hidden_${suffix}`} checked={hidden} onChange={e => setHidden(e.target.checked)}
                  className="h-5 w-5 rounded border-gray-200 bg-gray-50 text-slds-blue" />
                <label htmlFor={`hidden_${suffix}`} className="text-xs text-gray-500 font-medium">Hidden SSID</label>
              </div>
            </div>
          </div>
        ) : (
          <div className="space-y-3">
            <div className="grid grid-cols-2 gap-3 text-sm">
              <Info label="Configured Channel" value={configuredChannelText} />
              <Info label="Actual Channel" value={actualChannelText} />
              <Info label="Configured Bandwidth" value={configuredBandwidthText} />
              <Info label="Actual Bandwidth" value={actualBandwidthText} />
              <Info label="Clients" value={band.clients != null ? String(band.clients) : '0'} />
              <Info label="Status" value={masterEnabled ? (band.enabled ? 'On' : 'Off') : 'Master Off'} />
              <Info label="Security" value={band.security ?? '\u2014'} />
              <Info label="Hidden" value={band.hidden ? 'Yes' : 'No'} />
            </div>
            <div className="rounded-xl border border-gray-200/70 bg-gray-50 px-3 py-2">
              <p className="text-[9px] font-bold text-gray-500 uppercase tracking-widest mb-1">Channel Intelligence</p>
              {insights.length > 0 ? (
                <div className="space-y-1">
                  {insights.map((insight, idx) => (
                    <p key={`${suffix}-insight-${idx}`} className="text-xs text-gray-700">{insight}</p>
                  ))}
                </div>
              ) : (
                <p className="text-xs text-gray-500">No obvious channel conflicts detected.</p>
              )}
            </div>
          </div>
        )}
      </div>
    </Card>
  )
}

function Info({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <p className="text-[9px] font-bold text-gray-500 uppercase tracking-widest">{label}</p>
      <p className="text-gray-900">{value}</p>
    </div>
  )
}

export default function WiFiPage() {
  const [wifi, setWifi] = useState<WifiAll | null>(null)
  const [masterSaving, setMasterSaving] = useState(false)
  const [masterMsg, setMasterMsg] = useState('')
  const [masterErr, setMasterErr] = useState(false)
  const [syncSaving, setSyncSaving] = useState(false)
  const [syncMsg, setSyncMsg] = useState('')
  const [syncErr, setSyncErr] = useState(false)

  const fetchWifi = useCallback(async () => {
    try { setWifi(await api.wifiStatus()) } catch { /* ignore */ }
  }, [])

  useEffect(() => { fetchWifi() }, [fetchWifi])

  async function toggleMasterWifi() {
    if (!wifi) return
    const next = !wifi.master_enabled
    setMasterSaving(true)
    setMasterMsg('')
    setMasterErr(false)
    try {
      await api.wifiSet({ wifi_onoff: next ? '1' : '0' })
      setMasterMsg(next ? 'Global Wi-Fi enabled' : 'Global Wi-Fi disabled')
      await fetchWifi()
    } catch (e) {
      setMasterErr(true)
      setMasterMsg(e instanceof Error ? e.message : 'Error')
    } finally {
      setMasterSaving(false)
    }
  }

  async function syncBands(source: '2g' | '5g') {
    if (!wifi) return
    const sourceBand = source === '2g' ? wifi.band_2g : wifi.band_5g
    const sourceLabel = source === '2g' ? '2.4 GHz' : '5 GHz'
    const targetSuffix = source === '2g' ? '5g' : '2g'
    const targetLabel = source === '2g' ? '5 GHz' : '2.4 GHz'

    if (!sourceBand.ssid) {
      setSyncErr(true)
      setSyncMsg(`Cannot sync from ${sourceLabel}: source SSID is empty.`)
      return
    }

    const payload: Record<string, unknown> = {
      [`ssid_${targetSuffix}`]: sourceBand.ssid,
      [`hidden_${targetSuffix}`]: sourceBand.hidden ? '1' : '0',
    }
    if (sourceBand.security) payload[`encryption_${targetSuffix}`] = sourceBand.security
    const includePassword = Boolean(sourceBand.password && sourceBand.password !== '••••••••')
    if (includePassword) payload[`key_${targetSuffix}`] = sourceBand.password

    setSyncSaving(true)
    setSyncMsg('')
    setSyncErr(false)
    try {
      await api.wifiSet(payload)
      setSyncMsg(`Copied ${sourceLabel} settings to ${targetLabel}${includePassword ? ' (including password)' : ''}`)
      await fetchWifi()
    } catch (e) {
      setSyncErr(true)
      setSyncMsg(e instanceof Error ? e.message : 'Error')
    } finally {
      setSyncSaving(false)
    }
  }

  if (!wifi) return <div className="text-gray-500 text-sm">Loading\u2026</div>

  return (
    <div className="space-y-4">
      <h1 className="text-2xl sm:text-3xl font-bold text-gray-900">Wi-Fi</h1>

      <Card title="Global Wi-Fi">
        <div className="space-y-3">
          <div className="flex items-center justify-between gap-3">
            <div>
              <p className="text-sm font-medium text-gray-900">Master Switch</p>
              <p className="text-xs text-gray-500">
                {!wifi.master_supported
                  ? 'This firmware does not expose a reliable global Wi-Fi toggle.'
                  : wifi.master_enabled
                  ? 'ON: radios follow your per-band settings'
                  : 'OFF: all Wi-Fi radios are globally disabled'}
              </p>
            </div>
            <button
              onClick={toggleMasterWifi}
              disabled={masterSaving || !wifi.master_supported}
              className={`rounded-xl px-3 py-1.5 text-xs font-bold transition-all duration-150 ${
                !wifi.master_supported
                  ? 'bg-gray-100 text-gray-400 border border-gray-200'
                  : wifi.master_enabled
                  ? 'bg-green-50 text-green-600 border border-green-200 hover:bg-green-100'
                  : 'bg-red-50 text-red-600 border border-red-200 hover:bg-red-100'
              } disabled:opacity-40`}
            >
              {masterSaving ? 'Saving\u2026' : (!wifi.master_supported ? 'Unsupported' : (wifi.master_enabled ? 'On' : 'Off'))}
            </button>
          </div>
          {wifi.wifi6_supported && (
            <p className="text-xs text-gray-500">Wi-Fi 6: {wifi.wifi6_enabled ? 'Enabled' : 'Disabled'}</p>
          )}
          {masterMsg && (
            <p className={`text-xs ${masterErr ? 'text-red-500' : 'text-green-500'}`}>
              {masterMsg}
            </p>
          )}
        </div>
      </Card>

      <Card title="Band Sync">
        <div className="space-y-3">
          <p className="text-xs text-gray-500">Copy SSID, password, security, and hidden-state from one band to the other.</p>
          <div className="flex flex-wrap gap-2">
            <button
              onClick={() => syncBands('2g')}
              disabled={syncSaving}
              className="rounded-xl px-3 py-1.5 text-xs font-bold bg-white border border-gray-200 text-gray-700 hover:bg-gray-50 disabled:opacity-40"
            >
              {syncSaving ? 'Syncing\u2026' : 'Use 2.4 GHz for Both'}
            </button>
            <button
              onClick={() => syncBands('5g')}
              disabled={syncSaving}
              className="rounded-xl px-3 py-1.5 text-xs font-bold bg-white border border-gray-200 text-gray-700 hover:bg-gray-50 disabled:opacity-40"
            >
              {syncSaving ? 'Syncing\u2026' : 'Use 5 GHz for Both'}
            </button>
          </div>
          {syncMsg && (
            <p className={`text-xs ${syncErr ? 'text-red-500' : 'text-green-500'}`}>
              {syncMsg}
            </p>
          )}
        </div>
      </Card>

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <BandCard label="2.4 GHz" band={wifi.band_2g} suffix="2g" masterEnabled={wifi.master_enabled} onRefresh={fetchWifi} />
        <BandCard label="5 GHz" band={wifi.band_5g} suffix="5g" masterEnabled={wifi.master_enabled} onRefresh={fetchWifi} />
      </div>
      {wifi.guest_ssid && (
        <Card title="Guest Network">
          <p className="text-sm text-gray-600">SSID: <span className="font-medium text-gray-900">{wifi.guest_ssid}</span></p>
        </Card>
      )}
    </div>
  )
}
