import { useCallback, useEffect, useState } from 'react'
import { api } from '../../data/api'
import type { ApnProfile } from '../../types'
import { Button, Field, Input, Select } from '../../ui/controls'
import { toast, toastError, confirm } from '../../ui/feedback'
import { Card, Chip, Empty, Skeleton } from '../../ui/primitives'

const PDP_LABELS: Record<number, string> = { 1: 'IPv4', 2: 'IPv6', 3: 'IPv4v6' }
const AUTH_LABELS: Record<number, string> = { 0: 'None', 1: 'PAP', 2: 'CHAP', 3: 'PAP/CHAP' }

// ── APN mode ──────────────────────────────────────────────────────────────────

function ApnMode() {
  const [mode, setMode] = useState<number | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    api.apnModeGet()
      .then((d) => setMode(Number(d?.apn_mode) || 0))
      .catch(() => {})
  }, [])

  async function apply(newMode: number) {
    setBusy(true)
    try {
      await api.apnModeSet({ apn_mode: newMode })
      setMode(newMode)
      toast(newMode === 0 ? 'APN set to automatic' : 'APN set to manual')
    } catch (e) {
      toastError(e)
    } finally {
      setBusy(false)
    }
  }

  return (
    <Card title="APN mode">
      <p className="mb-3 text-meta text-ink2">
        Automatic selects the APN from your SIM. Switch to manual to use a custom profile.
      </p>
      <div className="flex gap-1.5">
        <button
          onClick={() => apply(0)}
          disabled={busy || mode === 0}
          className={`rounded-ctl px-3.5 py-1.5 text-body font-semibold transition-colors disabled:opacity-45 ${
            mode === 0 ? 'bg-ok/12 text-ok' : 'bg-surface2 text-ink2 hover:bg-line/10'
          }`}
        >
          Automatic
        </button>
        <button
          onClick={() => apply(1)}
          disabled={busy || mode === 1}
          className={`rounded-ctl px-3.5 py-1.5 text-body font-semibold transition-colors disabled:opacity-45 ${
            mode === 1 ? 'bg-accent text-onaccent' : 'bg-surface2 text-ink2 hover:bg-line/10'
          }`}
        >
          Manual
        </button>
      </div>
    </Card>
  )
}

// ── Profiles ──────────────────────────────────────────────────────────────────

function Profiles() {
  const [profiles, setProfiles] = useState<ApnProfile[]>([])
  const [loading, setLoading] = useState(true)
  const [adding, setAdding] = useState(false)
  const [busy, setBusy] = useState(false)
  const [form, setForm] = useState({ name: '', apn: '', user: '', pass: '', auth: 0, pdp: 3 })

  const fetchProfiles = useCallback(async () => {
    try {
      const data = await api.apnProfiles()
      const list = data?.apnListArray
      setProfiles(
        Array.isArray(list)
          ? (list as Record<string, unknown>[]).map((profile) => ({
              ...profile,
              profileId: String(profile.profileId),
              pdpType: Number(profile.pdpType),
              pppAuthMode: Number(profile.pppAuthMode),
              isEnable: profile.isEnable === true || profile.isEnable === 1 || profile.isEnable === '1',
            })) as ApnProfile[]
          : [],
      )
    } catch {
      setProfiles([])
    }
    setLoading(false)
  }, [])

  useEffect(() => {
    fetchProfiles()
  }, [fetchProfiles])

  async function addProfile() {
    setBusy(true)
    try {
      await api.apnAdd({
        profilename: form.name,
        wanapn: form.apn,
        username: form.user,
        password: form.pass,
        pppAuthMode: form.auth,
        pdpType: form.pdp,
      })
      toast('APN profile added')
      setAdding(false)
      setForm({ name: '', apn: '', user: '', pass: '', auth: 0, pdp: 3 })
      fetchProfiles()
    } catch (e) {
      toastError(e, 'Failed to add profile')
    } finally {
      setBusy(false)
    }
  }

  async function activateProfile(id: string) {
    try {
      await api.apnActivate({ profileId: id })
      toast('APN activated — connection may briefly drop')
      fetchProfiles()
    } catch (e) {
      toastError(e)
    }
  }

  async function deleteProfile(id: string) {
    const ok = await confirm({ title: 'Delete this APN profile?', confirmLabel: 'Delete', danger: true })
    if (!ok) return
    try {
      await api.apnDelete({ profileId: id })
      toast('Profile deleted')
      fetchProfiles()
    } catch (e) {
      toastError(e)
    }
  }

  return (
    <>
      <Card title="APN profiles">
        {loading ? (
          <Skeleton className="h-20" />
        ) : profiles.length === 0 ? (
          <Empty title="No manual APN profiles" body="Add the exact settings supplied by your carrier." />
        ) : (
          <div className="space-y-2">
            {profiles.map((p) => (
              <div
                key={p.profileId}
                className={`flex flex-wrap items-center justify-between gap-2 rounded-ctl border px-3 py-2 ${
                  p.isEnable ? 'border-accent/30 bg-accent/4' : 'border-line/8'
                }`}
              >
                <div className="min-w-0">
                  <p className="flex items-center gap-2 text-body font-semibold text-ink">
                    <span className="truncate">{p.profilename}</span>
                    {p.isEnable && <Chip tone="ok">Active</Chip>}
                  </p>
                  <p className="tnum font-mono mt-0.5 truncate text-meta text-ink2">
                    {p.wanapn} — {PDP_LABELS[p.pdpType] ?? '?'} / {AUTH_LABELS[p.pppAuthMode] ?? '?'}
                    {p.username ? ` — ${p.username}` : ''}
                  </p>
                </div>
                <div className="flex shrink-0 gap-1.5">
                  {!p.isEnable && (
                    <Button size="sm" variant="primary" onClick={() => activateProfile(p.profileId)}>
                      Activate
                    </Button>
                  )}
                  <Button size="sm" variant="ghost" disabled={p.isEnable} onClick={() => deleteProfile(p.profileId)} title={p.isEnable ? 'Switch to another APN before deleting this profile' : undefined}>
                    Delete
                  </Button>
                </div>
              </div>
            ))}
          </div>
        )}
      </Card>

      {adding ? (
        <Card title="Add APN profile">
          <div className="grid grid-cols-1 gap-2.5 lg:grid-cols-2">
            <Field label="Profile name">
              <Input value={form.name} onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))} placeholder="My Carrier" />
            </Field>
            <Field label="APN">
              <Input value={form.apn} onChange={(e) => setForm((f) => ({ ...f, apn: e.target.value }))} placeholder="internet" />
            </Field>
            <Field label="Username">
              <Input value={form.user} onChange={(e) => setForm((f) => ({ ...f, user: e.target.value }))} placeholder="(optional)" />
            </Field>
            <Field label="Password">
              <Input type="password" autoComplete="new-password" value={form.pass} onChange={(e) => setForm((f) => ({ ...f, pass: e.target.value }))} placeholder="(optional)" />
            </Field>
            <Field label="Authentication">
              <Select value={form.auth} onChange={(e) => setForm((f) => ({ ...f, auth: parseInt(e.target.value) }))}>
                <option value={0}>None</option>
                <option value={1}>PAP</option>
                <option value={2}>CHAP</option>
                <option value={3}>PAP/CHAP</option>
              </Select>
            </Field>
            <Field label="PDP type">
              <Select value={form.pdp} onChange={(e) => setForm((f) => ({ ...f, pdp: parseInt(e.target.value) }))}>
                <option value={3}>IPv4v6</option>
                <option value={1}>IPv4</option>
                <option value={2}>IPv6</option>
              </Select>
            </Field>
          </div>
          <div className="mt-3 flex gap-2">
            <Button variant="primary" onClick={addProfile} loading={busy} disabled={!form.name || !form.apn}>
              Add profile
            </Button>
            <Button variant="ghost" onClick={() => setAdding(false)}>
              Cancel
            </Button>
          </div>
        </Card>
      ) : (
        <Button variant="primary" onClick={() => setAdding(true)}>
          Add APN profile
        </Button>
      )}

    </>
  )
}

export default function ApnTab() {
  return (
    <div className="space-y-3">
      <ApnMode />
      <Profiles />
    </div>
  )
}
