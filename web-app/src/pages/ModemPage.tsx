import { useState, useEffect, useCallback } from 'react'
import { api, formatBytes, type ApnProfile, type DataUsage, type UsagePeriod } from '../api'
import Card from '../components/Card'

function Input({ label, value, onChange, placeholder }: { label: string; value: string; onChange: (v: string) => void; placeholder?: string }) {
  return (
    <div>
      <label className="mb-0.5 block text-xs text-gray-500">{label}</label>
      <input value={value} onChange={e => onChange(e.target.value)} placeholder={placeholder}
        className="w-full px-3.5 py-2.5 bg-gray-50 border border-gray-200 rounded-xl focus:ring-0 focus:shadow-macos-focus focus:border-slds-blue outline-none text-sm transition-all" />
    </div>
  )
}

function Alert({ msg, type = 'success' }: { msg: string; type?: 'success' | 'error' }) {
  return <p className={`text-xs ${type === 'error' ? 'text-red-500' : 'text-green-500'}`}>{msg}</p>
}

const APN_PRESETS: { name: string; apn: string; user: string; pass: string; auth: number; pdp: number }[] = [
  { name: 'Vodafone AU', apn: 'live.vodafone.com', user: '', pass: '', auth: 0, pdp: 3 },
  { name: 'Optus', apn: 'yesinternet', user: '', pass: '', auth: 0, pdp: 3 },
  { name: 'Telstra', apn: 'telstra.internet', user: '', pass: '', auth: 0, pdp: 3 },
  { name: 'T-Mobile US', apn: 'fast.t-mobile.com', user: '', pass: '', auth: 0, pdp: 3 },
  { name: 'AT&T', apn: 'broadband', user: '', pass: '', auth: 0, pdp: 3 },
  { name: 'Verizon', apn: 'vzwinternet', user: '', pass: '', auth: 0, pdp: 3 },
  { name: 'EE UK', apn: 'everywhere', user: 'eesecure', pass: 'secure', auth: 2, pdp: 3 },
  { name: 'Three UK', apn: 'three.co.uk', user: '', pass: '', auth: 0, pdp: 3 },
  { name: 'Vodafone UK', apn: 'wap.vodafone.co.uk', user: 'wap', pass: 'wap', auth: 1, pdp: 3 },
  { name: 'DoCoMo', apn: 'ppsim.jp', user: 'pp@sim', pass: 'jpn', auth: 2, pdp: 3 },
  { name: 'SoftBank', apn: 'plus.4g', user: 'plus', pass: '4g', auth: 2, pdp: 3 },
  { name: 'KDDI au', apn: 'uad5gn.au-net.ne.jp', user: '', pass: '', auth: 0, pdp: 3 },
  { name: 'Generic IPv4v6', apn: 'internet', user: '', pass: '', auth: 0, pdp: 3 },
]

const PDP_LABELS: Record<number, string> = { 1: 'IPv4', 2: 'IPv6', 3: 'IPv4v6' }
const AUTH_LABELS: Record<number, string> = { 0: 'None', 1: 'PAP', 2: 'CHAP', 3: 'PAP/CHAP' }

// ── APN Mode ────────────────────────────────────────────────────────────────
function ApnModeSection() {
  const [mode, setMode] = useState<number | null>(null)
  const [loading, setLoading] = useState(false)
  const [msg, setMsg] = useState('')

  useEffect(() => {
    api.apnModeGet()
      .then((d: Record<string, unknown>) => setMode((d?.apn_mode as number) ?? 0))
      .catch(() => {})
  }, [])

  async function apply(newMode: number) {
    setLoading(true); setMsg('')
    try {
      await api.apnModeSet({ apn_mode: newMode })
      setMode(newMode)
      setMsg(newMode === 0 ? 'APN set to automatic' : 'APN set to manual')
      setTimeout(() => setMsg(''), 3000)
    } catch (e) {
      setMsg(e instanceof Error ? e.message : 'Error')
    }
    setLoading(false)
  }

  return (
    <Card title="APN Mode">
      <p className="mb-3 text-xs text-gray-500">
        In automatic mode, the device selects the APN based on your SIM card.
        Switch to manual to use a custom APN profile.
      </p>
      <div className="flex gap-2">
        <button onClick={() => apply(0)} disabled={loading || mode === 0}
          className={`rounded-xl px-4 py-2 text-sm font-medium transition-all duration-200 ${
            mode === 0 ? 'bg-green-500/10 text-green-600' : 'bg-gray-50 text-gray-600 hover:bg-gray-50/60'
          } disabled:opacity-40`}>
          Automatic
        </button>
        <button onClick={() => apply(1)} disabled={loading || mode === 1}
          className={`rounded-xl px-4 py-2 text-sm font-medium transition-all duration-200 ${
            mode === 1 ? 'bg-slds-blue text-white' : 'bg-gray-50 text-gray-600 hover:bg-gray-50/60'
          } disabled:opacity-40`}>
          Manual
        </button>
      </div>
      {msg && <div className="mt-2"><Alert msg={msg} /></div>}
    </Card>
  )
}

// ── APN Profiles ────────────────────────────────────────────────────────────
function ApnSection() {
  const [profiles, setProfiles] = useState<ApnProfile[]>([])
  const [loading, setLoading] = useState(true)
  const [msg, setMsg] = useState('')
  const [adding, setAdding] = useState(false)
  const [form, setForm] = useState({ name: '', apn: '', user: '', pass: '', auth: 0, pdp: 3 })

  const fetchProfiles = useCallback(async () => {
    try {
      const data = await api.apnProfiles()
      setProfiles(Array.isArray(data?.apnListArray) ? data.apnListArray : [])
    } catch { setProfiles([]) }
    setLoading(false)
  }, [])

  useEffect(() => { fetchProfiles() }, [fetchProfiles])

  const flash = (m: string) => { setMsg(m); setTimeout(() => setMsg(''), 3000) }

  async function addProfile() {
    try {
      await api.apnAdd({
        profilename: form.name, wanapn: form.apn,
        username: form.user, password: form.pass,
        pppAuthMode: form.auth, pdpType: form.pdp,
      })
      flash('APN profile added')
      setAdding(false)
      setForm({ name: '', apn: '', user: '', pass: '', auth: 0, pdp: 3 })
      fetchProfiles()
    } catch (e) { flash(e instanceof Error ? e.message : 'Error') }
  }

  async function activateProfile(id: string) {
    try {
      await api.apnActivate({ profileId: id })
      flash('APN activated — connection may briefly drop')
      fetchProfiles()
    } catch (e) { flash(e instanceof Error ? e.message : 'Error') }
  }

  async function deleteProfile(id: string) {
    try {
      await api.apnDelete({ profileId: id })
      flash('Profile deleted')
      fetchProfiles()
    } catch (e) { flash(e instanceof Error ? e.message : 'Error') }
  }

  function applyPreset(p: typeof APN_PRESETS[0]) {
    setForm({ name: p.name, apn: p.apn, user: p.user, pass: p.pass, auth: p.auth, pdp: p.pdp })
    setAdding(true)
  }

  return (
    <div className="space-y-4">
      {msg && <Alert msg={msg} type={msg.includes('Error') ? 'error' : 'success'} />}

      <Card title="APN Profiles">
        {loading ? <p className="text-sm text-gray-500">Loading...</p> : profiles.length === 0 ? (
          <p className="text-sm text-gray-500">No manual APN profiles</p>
        ) : (
          <div className="space-y-2">
            {profiles.map(p => (
              <div key={p.profileId} className={`flex items-center justify-between rounded-xl px-3 py-2 ${p.isEnable ? 'bg-white/95 border border-slds-blue/30' : 'bg-gray-50/50'}`}>
                <div>
                  <p className="text-sm font-medium text-gray-900">
                    {p.profilename}
                    {p.isEnable && <span className="ml-2 rounded-xl bg-green-100 px-1.5 py-0.5 text-[10px] text-green-700">Active</span>}
                  </p>
                  <p className="text-xs text-gray-500">
                    {p.wanapn} — {PDP_LABELS[p.pdpType] ?? '?'} / {AUTH_LABELS[p.pppAuthMode] ?? '?'}
                    {p.username ? ` — ${p.username}` : ''}
                  </p>
                </div>
                <div className="flex gap-1.5">
                  {!p.isEnable && (
                    <button onClick={() => activateProfile(p.profileId)}
                      className="rounded-xl bg-slds-blue text-white px-3 py-2 text-xs font-bold hover:bg-slds-blueHover active:scale-[0.98] transition-all">Activate</button>
                  )}
                  <button onClick={() => deleteProfile(p.profileId)}
                    className="rounded-xl bg-red-500/10 px-3 py-2 text-xs font-medium text-red-600 hover:bg-red-500/20 transition-all duration-200">Delete</button>
                </div>
              </div>
            ))}
          </div>
        )}
      </Card>

      {adding ? (
        <Card title="Add APN Profile">
          <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
            <Input label="Profile Name" value={form.name} onChange={v => setForm(f => ({ ...f, name: v }))} placeholder="e.g. My Carrier" />
            <Input label="APN" value={form.apn} onChange={v => setForm(f => ({ ...f, apn: v }))} placeholder="e.g. internet" />
            <Input label="Username" value={form.user} onChange={v => setForm(f => ({ ...f, user: v }))} placeholder="(optional)" />
            <Input label="Password" value={form.pass} onChange={v => setForm(f => ({ ...f, pass: v }))} placeholder="(optional)" />
            <div>
              <label className="mb-0.5 block text-xs text-gray-500">Authentication</label>
              <select value={form.auth} onChange={e => setForm(f => ({ ...f, auth: parseInt(e.target.value) }))}
                className="w-full px-3.5 py-2.5 bg-gray-50 border border-gray-200 rounded-xl focus:ring-0 focus:shadow-macos-focus focus:border-slds-blue outline-none text-sm transition-all">
                <option value={0}>None</option>
                <option value={1}>PAP</option>
                <option value={2}>CHAP</option>
                <option value={3}>PAP/CHAP</option>
              </select>
            </div>
            <div>
              <label className="mb-0.5 block text-xs text-gray-500">PDP Type</label>
              <select value={form.pdp} onChange={e => setForm(f => ({ ...f, pdp: parseInt(e.target.value) }))}
                className="w-full px-3.5 py-2.5 bg-gray-50 border border-gray-200 rounded-xl focus:ring-0 focus:shadow-macos-focus focus:border-slds-blue outline-none text-sm transition-all">
                <option value={3}>IPv4v6</option>
                <option value={1}>IPv4</option>
                <option value={2}>IPv6</option>
              </select>
            </div>
          </div>
          <div className="mt-3 flex gap-2">
            <button onClick={addProfile} disabled={!form.name || !form.apn}
              className="bg-slds-blue text-white py-3.5 rounded-2xl font-bold shadow-macos-lg shadow-slds-blue/20 hover:bg-slds-blue active:scale-[0.98] disabled:opacity-40 transition-all px-4 text-sm">Add Profile</button>
            <button onClick={() => setAdding(false)}
              className="bg-white border border-gray-200 hover:bg-gray-50 px-3 py-2 rounded-xl font-bold text-gray-500 shadow-sm transition-all active:scale-95 text-sm">Cancel</button>
          </div>
        </Card>
      ) : (
        <button onClick={() => setAdding(true)}
          className="bg-slds-blue text-white py-3.5 rounded-2xl font-bold shadow-macos-lg shadow-slds-blue/20 hover:bg-slds-blue active:scale-[0.98] disabled:opacity-40 transition-all px-4 text-sm">+ Add APN Profile</button>
      )}

      <Card title="Quick Presets">
        <p className="mb-2 text-xs text-gray-500">Tap a preset to pre-fill the add form with common carrier settings.</p>
        <div className="flex flex-wrap gap-1.5">
          {APN_PRESETS.map(p => (
            <button key={p.name} onClick={() => applyPreset(p)}
              className="rounded-xl bg-gray-50 px-2.5 py-1.5 text-xs font-medium text-gray-600 hover:bg-gray-50/60 transition-all duration-200">{p.name}</button>
          ))}
        </div>
      </Card>
    </div>
  )
}

// ── TTL Clamping ────────────────────────────────────────────────────────────
function TtlSection() {
  const [active, setActive] = useState<boolean | null>(null)
  const [ipv6Active, setIpv6Active] = useState(false)
  const [currentTtl, setCurrentTtl] = useState(0)
  const [ttlInput, setTtlInput] = useState('65')
  const [loading, setLoading] = useState(false)
  const [msg, setMsg] = useState('')

  async function fetchStatus() {
    try {
      const data = await api.ttlStatus()
      setActive(data.active || data.ipv6_active)
      setIpv6Active(data.ipv6_active)
      setCurrentTtl(data.ttl_value)
      if (data.ttl_value > 0) setTtlInput(String(data.ttl_value))
    } catch {
      setMsg('Unable to fetch TTL status')
    }
  }

  useEffect(() => { fetchStatus() }, [])

  async function applyTtl() {
    const val = parseInt(ttlInput)
    if (!val || val < 1 || val > 255) { setMsg('TTL must be 1-255'); return }
    setLoading(true); setMsg('')
    try {
      await api.ttlSet(val)
      setMsg(`TTL set to ${val} (IPv4 + IPv6)`)
      await fetchStatus()
    } catch (e) {
      setMsg(e instanceof Error ? e.message : 'Failed')
    } finally {
      setLoading(false)
    }
  }

  async function clearTtl() {
    setLoading(true); setMsg('')
    try {
      await api.ttlClear()
      setMsg('TTL clamping disabled')
      await fetchStatus()
    } catch (e) {
      setMsg(e instanceof Error ? e.message : 'Failed')
    } finally {
      setLoading(false)
    }
  }

  return (
    <Card title="TTL Clamping">
      <div className="space-y-3">
        <p className="text-xs text-gray-500">
          Override TTL/Hop Limit on LAN ingress traffic to prevent carrier tethering detection.
          Changes are applied immediately and persist across reboots.
        </p>

        {active === false ? (
          <div className="flex flex-wrap items-center gap-2 pt-1">
            <input
              type="number" min={1} max={255} value={ttlInput}
              onChange={e => setTtlInput(e.target.value)}
              placeholder="e.g. 65"
              className="w-24 px-3.5 py-2.5 bg-gray-50 border border-gray-200 rounded-xl focus:ring-0 focus:shadow-macos-focus focus:border-slds-blue outline-none text-sm transition-all"
            />
            <button onClick={applyTtl} disabled={loading || !ttlInput}
              className="bg-slds-blue text-white py-3.5 rounded-2xl font-bold shadow-macos-lg shadow-slds-blue/20 hover:bg-slds-blue active:scale-[0.98] disabled:opacity-40 transition-all px-4 text-sm py-1.5">
              {loading ? 'Applying...' : 'Enable Clamping'}
            </button>
          </div>
        ) : active === true ? (
          <div className="flex flex-wrap items-center gap-y-3 gap-x-4 pt-1">
            <div className="flex items-center gap-2">
              <span className="h-2 w-2 rounded-full bg-green-500" />
              <span className="text-sm font-medium text-green-500">Active (TTL={currentTtl})</span>
              {ipv6Active && <span className="rounded-xl bg-gray-50/50 px-1.5 py-0.5 text-[10px] text-gray-500">IPv4 + IPv6</span>}
            </div>
            
            <div className="flex items-center gap-2 border-t sm:border-t-0 sm:border-l border-gray-200/60 pt-2 sm:pt-0 sm:pl-4">
              <input
                type="number" min={1} max={255} value={ttlInput}
                onChange={e => setTtlInput(e.target.value)}
                className="w-20 px-3.5 py-2.5 bg-gray-50 border border-gray-200 rounded-xl focus:ring-0 focus:shadow-macos-focus focus:border-slds-blue outline-none text-sm transition-all"
              />
              <button onClick={applyTtl} disabled={loading}
                className="bg-white border border-gray-200 hover:bg-gray-50 px-3 py-2 rounded-xl font-bold text-gray-500 shadow-sm transition-all active:scale-95 text-sm py-1.5 disabled:opacity-40">
                Update
              </button>
              <button onClick={clearTtl} disabled={loading}
                className="rounded-xl bg-red-500/10 px-3 py-1.5 text-sm font-medium text-red-600 hover:bg-red-500/20 transition-all duration-200 disabled:opacity-40">
                Disable
              </button>
            </div>
          </div>
        ) : (
          <span className="text-sm text-gray-500 pt-1 block">Checking status...</span>
        )}

        {msg && <Alert msg={msg} type={msg.includes('Failed') || msg.includes('Unable') ? 'error' : 'success'} />}
      </div>
    </Card>
  )
}

// ── Data Usage ──────────────────────────────────────────────────────────────
function formatUsagePeriod(secs?: number) {
  if (!secs) return '\u2014'
  const d = Math.floor(secs / 86400)
  const h = Math.floor((secs % 86400) / 3600)
  const m = Math.floor((secs % 3600) / 60)
  return [d && `${d}d`, (d || h) && `${h}h`, `${m}m`].filter(Boolean).join(' ')
}

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
    <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
      <div className="rounded-2xl border border-gray-200/60 bg-gray-50/60 p-4">
        <p className="text-xs font-bold uppercase text-green-500">Download</p>
        <p className="mt-1 text-2xl font-bold text-gray-900">{formatBytes(usage.rx_bytes)}</p>
      </div>
      <div className="rounded-2xl border border-gray-200/60 bg-gray-50/60 p-4">
        <p className="text-xs font-bold uppercase text-slds-blue">Upload</p>
        <p className="mt-1 text-2xl font-bold text-gray-900">{formatBytes(usage.tx_bytes)}</p>
      </div>
      <div className="rounded-2xl border border-gray-200/60 bg-gray-50/60 p-4">
        <p className="text-xs font-bold uppercase text-gray-500">Total</p>
        <p className="mt-1 text-2xl font-bold text-gray-900">{formatBytes(total)}</p>
      </div>
    </div>
  )
}

function DataUsageSection() {
  const [usage, setUsage] = useState<DataUsage | null>(null)
  const [editingResetDay, setEditingResetDay] = useState(false)
  const [resetDay, setResetDay] = useState('1')
  const [savingResetDay, setSavingResetDay] = useState(false)
  const [msg, setMsg] = useState('')

  const fetchUsage = useCallback(async () => {
    try {
      const next = await api.dataUsage()
      setUsage(next)
      if (!editingResetDay && next.reset_day) setResetDay(String(next.reset_day))
    } catch { /* ignore */ }
  }, [editingResetDay])

  async function saveResetDay() {
    const day = parseInt(resetDay, 10)
    if (!day || day < 1 || day > 31) {
      setMsg('Reset day must be between 1 and 31')
      return
    }
    setSavingResetDay(true)
    setMsg('')
    try {
      const next = await api.dataUsageResetDaySet(day)
      setUsage(next)
      setResetDay(String(next.reset_day ?? day))
      setEditingResetDay(false)
      setMsg(`Reset day set to day ${next.reset_day ?? day}`)
    } catch (e) {
      setMsg(e instanceof Error ? e.message : 'Failed to set reset day')
    } finally {
      setSavingResetDay(false)
    }
  }

  useEffect(() => {
    fetchUsage()
    const id = setInterval(fetchUsage, 10000)
    return () => clearInterval(id)
  }, [fetchUsage])

  const currentResetDay = usage?.reset_day ?? (parseInt(resetDay, 10) || 1)
  const dates = cycleWindow(currentResetDay)
  const cycle = usage?.cycle ?? usage?.month
  const sincePowerOn = usage?.since_power_on
  const rows = usage ? [
    { label: 'Today', data: usage.day },
    { label: 'Device lifetime', data: usage.total },
  ] : []

  return (
    <div className="space-y-4">
      <Card title="Current Data Cycle" action={
        <button
          onClick={() => {
            setEditingResetDay(true)
            setResetDay(String(currentResetDay))
            setMsg('')
          }}
          className="text-xs text-slds-blue hover:text-slds-blue transition-colors"
        >
          Set Reset Day
        </button>
      }>
        {editingResetDay && (
          <div className="mb-4 flex flex-col gap-3 rounded-2xl bg-gray-50/60 p-3 sm:flex-row sm:items-end">
            <div>
              <label className="mb-0.5 block text-xs text-gray-500">Reset day of month</label>
              <input
                type="number"
                min={1}
                max={31}
                value={resetDay}
                onChange={e => setResetDay(e.target.value)}
                className="w-full px-3.5 py-2.5 bg-white border border-gray-200 rounded-xl focus:ring-0 focus:shadow-macos-focus focus:border-slds-blue outline-none text-sm transition-all sm:w-28"
              />
            </div>
            <div className="flex gap-2">
              <button onClick={saveResetDay} disabled={savingResetDay}
                className="flex-1 sm:flex-none bg-slds-blue text-white py-2.5 rounded-xl font-bold shadow-macos-lg shadow-slds-blue/20 hover:bg-slds-blue active:scale-[0.98] disabled:opacity-40 transition-all px-4 text-sm">
                {savingResetDay ? 'Saving...' : 'Save'}
              </button>
              <button onClick={() => setEditingResetDay(false)} disabled={savingResetDay}
                className="flex-1 sm:flex-none bg-white border border-gray-200 hover:bg-gray-50 px-3 py-2.5 rounded-xl font-bold text-gray-500 shadow-sm transition-all active:scale-95 text-sm disabled:opacity-40">
                Cancel
              </button>
            </div>
          </div>
        )}
        {msg && <Alert msg={msg} type={msg.includes('Failed') || msg.includes('must') ? 'error' : 'success'} />}

        {cycle ? (
          <div className="space-y-4">
            <div className="flex flex-wrap gap-x-4 gap-y-1 text-sm text-gray-500">
              <span>Reset day: <span className="font-bold text-gray-900">{currentResetDay}</span></span>
              <span>Current period: <span className="font-bold text-gray-900">{formatDate(dates.start)} - {formatDate(dates.end)}</span></span>
              <span>Next reset: <span className="font-bold text-gray-900">{formatDate(dates.nextStart)}</span></span>
            </div>
            <UsageTotals usage={cycle} />
            <p className="text-xs text-gray-500">
              These counters are maintained by the router and reset on the configured day of each month.
            </p>
          </div>
        ) : (
          <p className="text-sm text-gray-500">Loading...</p>
        )}
      </Card>

      <Card title="Data Since Power On">
        {sincePowerOn ? (
          <div className="space-y-3">
            <UsageTotals usage={sincePowerOn} />
            <p className="text-xs text-gray-500">Counter time: {formatUsagePeriod(sincePowerOn.time_secs)}</p>
          </div>
        ) : (
          <p className="text-sm text-gray-500">Loading...</p>
        )}
      </Card>

      {usage && (
        <Card title="Other Counters">
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-xs text-gray-500">
                  <th className="pb-2 pr-4 font-medium">Period</th>
                  <th className="pb-2 pr-4 font-medium text-right">Download</th>
                  <th className="pb-2 pr-4 font-medium text-right">Upload</th>
                  <th className="pb-2 pr-4 font-medium text-right">Total</th>
                  <th className="pb-2 font-medium text-right">Time</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-100/60">
                {rows.map(({ label, data }) => (
                  <tr key={label}>
                    <td className="py-2 pr-4 text-gray-600">{label}</td>
                    <td className="py-2 pr-4 text-right text-green-500">{formatBytes(data.rx_bytes)}</td>
                    <td className="py-2 pr-4 text-right text-slds-blue">{formatBytes(data.tx_bytes)}</td>
                    <td className="py-2 pr-4 text-right font-medium text-gray-900">{formatBytes(data.rx_bytes + data.tx_bytes)}</td>
                    <td className="py-2 text-right text-gray-500">{formatUsagePeriod(data.time_secs)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Card>
      )}
    </div>
  )
}

// ── Main ────────────────────────────────────────────────────────────────────
export default function ModemPage() {
  const [tab, setTab] = useState<'apn' | 'data' | 'ttl'>('apn')

  return (
    <div className="space-y-4">
      <h1 className="text-2xl sm:text-3xl font-bold text-gray-900">Modem</h1>

      <div className="bg-gray-50/50 rounded-2xl p-1 flex gap-1 border border-gray-200/50 w-fit">
        {([
          ['apn', 'APN'],
          ['data', 'Data Usage'],
          ['ttl', 'TTL'],
        ] as const).map(([id, label]) => (
          <button key={id} onClick={() => setTab(id)}
            className={`rounded-xl px-4 py-2.5 text-sm font-medium transition-all duration-200 ${
              tab === id ? 'bg-white text-gray-900 shadow-sm' : 'text-gray-500 hover:text-gray-600'
            }`}>
            {label}
          </button>
        ))}
      </div>

      {tab === 'apn' && (
        <div className="space-y-4">
          <ApnModeSection />
          <ApnSection />
        </div>
      )}
      {tab === 'data' && <DataUsageSection />}
      {tab === 'ttl' && <TtlSection />}
    </div>
  )
}
