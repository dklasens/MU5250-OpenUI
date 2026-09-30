import { useEffect, useState } from 'react'
import { useHome } from '../../app/HomeContext'
import { api } from '../../data/api'
import type { ModemCapabilities, SignalInfo } from '../../types'
import { Button, Field, Input } from '../../ui/controls'
import { toast, toastError, confirm } from '../../ui/feedback'
import { Card, Chip, Skeleton } from '../../ui/primitives'

// ── Network mode ──────────────────────────────────────────────────────────────

function NetworkMode({ currentMode, modes, onApplied }: { currentMode: string; modes: ModemCapabilities['network_modes']; onApplied: () => void }) {
  const [selected, setSelected] = useState(currentMode)
  const [busy, setBusy] = useState(false)

  useEffect(() => setSelected(currentMode), [currentMode])

  async function apply() {
    setBusy(true)
    try {
      await api.networkModeSet(selected)
      toast('Network mode applied — connection may briefly drop')
      onApplied()
    } catch (e) {
      toastError(e, 'Failed to set network mode')
    } finally {
      setBusy(false)
    }
  }

  return (
    <Card title="Network mode">
      <p className="mb-3 text-meta text-ink2">
        Preferred network technology. The modem reconnects after a change.
      </p>
      <div className="flex flex-wrap gap-1.5">
        {modes.map((m) => (
          <button
            key={m.value}
            onClick={() => setSelected(m.value)}
            className={`rounded-ctl px-3 py-1.5 text-body font-semibold transition-colors ${
              selected === m.value
                ? 'bg-accent text-onaccent'
                : 'bg-surface2 text-ink2 hover:bg-line/10 hover:text-ink'
            }`}
          >
            {m.label}
          </button>
        ))}
      </div>
      <div className="mt-3 flex items-center gap-3">
        <Button variant="primary" onClick={apply} loading={busy} disabled={selected === currentMode}>
          Apply
        </Button>
        {selected !== currentMode && (
          <span className="text-meta text-ink3">
            Current: {modes.find((m) => m.value === currentMode)?.label ?? currentMode}
          </span>
        )}
      </div>
    </Card>
  )
}

// ── Serving cells with one-tap lock ───────────────────────────────────────────

function ServingCells({ signal, onLock }: { signal: SignalInfo; onLock: (type: 'nr' | 'lte', pci: number, earfcn: number, band?: string) => void }) {
  const { lte_carriers, nr_carriers } = signal
  if (lte_carriers.length === 0 && nr_carriers.length === 0) return null

  const renderRows = (carriers: typeof nr_carriers, tech: 'nr' | 'lte') =>
    carriers.map((c, i) => {
      const bandNum = c.band.replace(/\D/g, '')
      return (
        <tr key={i} className="border-b border-line/6 last:border-0">
          <td className="py-1.5 pr-3">
            <Chip tone={c.label === 'PCC' ? (tech === 'nr' ? 'nr' : 'lte') : 'default'}>{c.label}</Chip>
          </td>
          <td className={`py-1.5 pr-3 font-semibold ${tech === 'nr' ? 'text-nr' : 'text-accent'}`}>
            {c.band}
          </td>
          <td className="tnum font-mono py-1.5 pr-3">{c.pci}</td>
          <td className="tnum font-mono py-1.5 pr-3">{c.earfcn}</td>
          <td className="tnum font-mono hidden py-1.5 pr-3 text-ink2 sm:table-cell">{c.bandwidth}</td>
          <td className="tnum font-mono py-1.5 pr-3">{c.rsrp ?? '\u2014'}</td>
          <td className="tnum font-mono hidden py-1.5 pr-3 sm:table-cell">{c.sinr ?? '\u2014'}</td>
          <td className="py-1.5 text-right">
            <Button size="sm" variant="outline" onClick={() => onLock(tech, c.pci, c.earfcn, tech === 'nr' ? bandNum : undefined)}>
              Lock
            </Button>
          </td>
        </tr>
      )
    })

  return (
    <Card title="Serving cells">
      <p className="mb-3 text-meta text-ink2">Active cells — lock directly from this list.</p>
      <div className="overflow-x-auto">
        <table className="w-full text-left text-body">
          <thead>
            <tr className="label border-b border-line/8">
              <th className="pb-1.5 pr-3 font-semibold">Type</th>
              <th className="pb-1.5 pr-3 font-semibold">Band</th>
              <th className="pb-1.5 pr-3 font-semibold">PCI</th>
              <th className="pb-1.5 pr-3 font-semibold">ARFCN</th>
              <th className="hidden pb-1.5 pr-3 font-semibold sm:table-cell">BW</th>
              <th className="pb-1.5 pr-3 font-semibold">RSRP</th>
              <th className="hidden pb-1.5 pr-3 font-semibold sm:table-cell">SINR</th>
              <th className="pb-1.5" />
            </tr>
          </thead>
          <tbody>
            {nr_carriers.length > 0 && renderRows(nr_carriers, 'nr')}
            {lte_carriers.length > 0 && renderRows(lte_carriers, 'lte')}
          </tbody>
        </table>
      </div>
    </Card>
  )
}

// ── Band lock grid ────────────────────────────────────────────────────────────

function BandLock({
  title,
  description,
  bands,
  type,
  lockedBands,
  onApplied,
}: {
  title: string
  description: string
  bands: number[]
  type: 'nr' | 'lte'
  lockedBands?: number[]
  onApplied: () => void
}) {
  const [selected, setSelected] = useState<Set<number>>(new Set())
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    if (lockedBands) setSelected(new Set(lockedBands))
  }, [lockedBands])

  function toggle(band: number) {
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(band)) next.delete(band)
      else next.add(band)
      return next
    })
  }

  async function apply() {
    if (selected.size === 0) {
      toast('Select at least one band', 'err')
      return
    }
    setBusy(true)
    try {
      const sorted = Array.from(selected).sort((a, b) => a - b)
      if (type === 'nr') await api.bandLockNr(sorted.join(','))
      else await api.bandLockLte(sorted)
      toast(`${type === 'nr' ? 'NR' : 'LTE'} locked to ${sorted.map((b) => (type === 'nr' ? `n${b}` : `B${b}`)).join(', ')}`)
      onApplied()
    } catch (e) {
      toastError(e, 'Band lock failed')
    } finally {
      setBusy(false)
    }
  }

  const activeCls = type === 'nr' ? 'bg-nr text-onaccent' : 'bg-accent text-onaccent'

  return (
    <Card title={title}>
      <p className="mb-2.5 text-meta text-ink2">{description}</p>
      {lockedBands && lockedBands.length > 0 && (
        <p className="mb-2 text-meta font-medium text-ok">
          Locked: {[...lockedBands].sort((a, b) => a - b).map((b) => (type === 'nr' ? `n${b}` : `B${b}`)).join(', ')}
        </p>
      )}
      <div className="mb-2 flex gap-3 text-meta font-semibold">
        <button className="text-accent hover:underline" onClick={() => setSelected(new Set(bands))}>
          Select all
        </button>
        <button className="text-ink3 hover:text-ink hover:underline" onClick={() => setSelected(new Set())}>
          Clear
        </button>
      </div>
      <div className="flex flex-wrap gap-1.5">
        {bands.map((b) => (
          <button
            key={b}
            onClick={() => toggle(b)}
            className={`tnum font-mono rounded-ctl px-2.5 py-1.5 text-meta font-semibold transition-colors ${
              selected.has(b) ? activeCls : 'bg-surface2 text-ink2 hover:bg-line/10 hover:text-ink'
            }`}
          >
            {type === 'nr' ? `n${b}` : `B${b}`}
          </button>
        ))}
      </div>
      <div className="mt-3">
        <Button variant="primary" onClick={apply} loading={busy} disabled={selected.size === 0}>
          Lock {selected.size} band{selected.size !== 1 ? 's' : ''}
        </Button>
      </div>
    </Card>
  )
}

// ── Manual cell lock ──────────────────────────────────────────────────────────

function CellLock({ type, onApplied }: { type: 'nr' | 'lte'; onApplied: () => void }) {
  const [pci, setPci] = useState('')
  const [earfcn, setEarfcn] = useState('')
  const [band, setBand] = useState('')
  const [busy, setBusy] = useState(false)

  async function apply() {
    if (!pci || !earfcn) {
      toast('PCI and ARFCN are required', 'err')
      return
    }
    if (type === 'nr' && !band) {
      toast('Band is required for NR cell lock', 'err')
      return
    }
    setBusy(true)
    try {
      if (type === 'nr') await api.cellLockNr(pci, earfcn, band)
      else await api.cellLockLte(pci, earfcn)
      toast(`${type === 'nr' ? 'NR' : 'LTE'} cell locked (PCI ${pci})`)
      onApplied()
    } catch (e) {
      toastError(e, 'Cell lock failed')
    } finally {
      setBusy(false)
    }
  }

  return (
    <Card title={`${type === 'nr' ? 'NR' : 'LTE'} cell lock`}>
      <p className="mb-3 text-meta text-ink2">
        Lock to a specific cell by PCI and {type === 'nr' ? 'NR-ARFCN' : 'EARFCN'}.
      </p>
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
        <Field label="PCI">
          <Input type="number" inputMode="numeric" value={pci} onChange={(e) => setPci(e.target.value)} placeholder="30" />
        </Field>
        <Field label={type === 'nr' ? 'NR-ARFCN' : 'EARFCN'}>
          <Input type="number" inputMode="numeric" value={earfcn} onChange={(e) => setEarfcn(e.target.value)} placeholder={type === 'nr' ? '630912' : '3650'} />
        </Field>
        {type === 'nr' && (
          <Field label="Band">
            <Input type="number" inputMode="numeric" value={band} onChange={(e) => setBand(e.target.value)} placeholder="78" />
          </Field>
        )}
        <div className="flex items-end">
          <Button variant="primary" onClick={apply} loading={busy} className="w-full">
            Lock
          </Button>
        </div>
      </div>
    </Card>
  )
}

// ── Group ─────────────────────────────────────────────────────────────────────

export default function Locking() {
  // Shares the home poll rather than fetching /api/network/signal separately;
  // `refresh` re-runs that batch after a lock is applied.
  const { data: home, refresh } = useHome()
  const signal = home?.signal ?? null
  const [capabilities, setCapabilities] = useState<ModemCapabilities | null | undefined>(undefined)

  useEffect(() => {
    api.modemCapabilities().then(setCapabilities).catch(() => setCapabilities(null))
  }, [])

  async function handleLockCell(type: 'nr' | 'lte', pci: number, earfcn: number, band?: string) {
    try {
      if (type === 'nr') await api.cellLockNr(String(pci), String(earfcn), band ?? '')
      else await api.cellLockLte(String(pci), String(earfcn))
      toast(`Locked to ${type === 'nr' ? 'NR' : 'LTE'} cell PCI ${pci}`)
      refresh()
    } catch (e) {
      toastError(e, 'Lock failed')
    }
  }

  async function resetBands() {
    const ok = await confirm({
      title: 'Reset band locks?',
      body: 'The modem will use automatic band selection again.',
      confirmLabel: 'Reset',
      danger: true,
    })
    if (!ok) return
    try {
      await api.bandLockReset()
      toast('All band locks cleared')
      refresh()
    } catch (e) {
      toastError(e, 'Reset failed')
    }
  }

  async function resetCells() {
    const ok = await confirm({
      title: 'Reset cell locks?',
      body: 'The modem will use automatic cell selection again.',
      confirmLabel: 'Reset',
      danger: true,
    })
    if (!ok) return
    try {
      await api.cellLockReset()
      toast('All cell locks cleared')
      refresh()
    } catch (e) {
      toastError(e, 'Reset failed')
    }
  }

  if (!signal) {
    return (
      <div className="space-y-3">
        <Skeleton className="h-40" />
        <Skeleton className="h-56" />
      </div>
    )
  }

  return (
    <div className="space-y-3">
      {capabilities ? (
        <NetworkMode currentMode={signal.net_select ?? 'WL_AND_5G'} modes={capabilities.network_modes} onApplied={refresh} />
      ) : capabilities === undefined ? (
        <Skeleton className="h-40" />
      ) : (
        <Card title="Network mode and bands">
          <p className="text-meta text-warn">Firmware capability data is unavailable, so radio mode and band changes are disabled.</p>
        </Card>
      )}

      <ServingCells signal={signal} onLock={handleLockCell} />

      {capabilities && <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
        <BandLock
          title="NR 5G band lock"
          description="Allowed NR bands. Works in 5G SA mode only — firmware does not support NSA band locking."
          bands={capabilities.nr_sa_bands}
          type="nr"
          lockedBands={signal.nr_band_lock}
          onApplied={refresh}
        />
        <BandLock
          title="LTE band lock"
          description="Allowed LTE bands."
          bands={capabilities.lte_bands}
          type="lte"
          lockedBands={signal.lte_band_lock}
          onApplied={refresh}
        />
      </div>}

      <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
        <CellLock type="nr" onApplied={refresh} />
        <CellLock type="lte" onApplied={refresh} />
      </div>

      <Card title="Reset locks">
        <p className="mb-3 text-meta text-ink2">
          Remove all band and cell locks; the modem returns to automatic selection.
        </p>
        <div className="flex flex-wrap gap-2">
          <Button variant="outline" onClick={resetBands}>
            Reset band locks
          </Button>
          <Button variant="outline" onClick={resetCells}>
            Reset cell locks
          </Button>
        </div>
      </Card>

      <Card title="Diagnostics">
        <div className="tnum space-y-1 font-mono text-caption text-ink3">
          <p>
            LTE lock (raw): <span className="text-ink">{signal.raw_lte_band_lock || '(empty)'}</span>
          </p>
          <p>
            NR lock (raw): <span className="text-ink">{signal.raw_nr_band_lock || '(empty)'}</span>
          </p>
          <p>
            LTE lock (parsed): <span className="text-ink">{signal.lte_band_lock?.join(', ') || '(none)'}</span>
          </p>
          <p>
            NR lock (parsed): <span className="text-ink">{signal.nr_band_lock?.join(', ') || '(none)'}</span>
          </p>
          <p>
            Network mode: <span className="text-ink">{signal.net_select || '(unknown)'}</span>
          </p>
        </div>
      </Card>
    </div>
  )
}
