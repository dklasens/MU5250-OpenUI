import { useCallback, useRef, useState } from 'react'
import { useHome } from '../../app/HomeContext'
import { api } from '../../data/api'
import { cellLockServing } from '../../data/cellLock'
import { useResource } from '../../data/poll'
import type { BandLockState } from '../../types'
import { Button } from '../../ui/controls'
import { confirm, toast, toastError } from '../../ui/feedback'
import { Card, InlineStatus, Loading, Skeleton } from '../../ui/primitives'
import { BandLock } from './locking/BandLock'
import { CellLock } from './locking/CellLock'
import { bandResetConfirm, cellLockConfirm, cellResetConfirm, type CellTuple } from './locking/confirmations'
import { bandSpec, lteObservation, modeSpec, nrSaObservation } from './locking/draft'
import { NetworkMode } from './locking/NetworkMode'
import type { Ops } from './locking/ops'
import { ServingCells } from './locking/ServingCells'
import { useDraft } from './locking/useDraft'

function describeLockState(state: BandLockState): string {
  if (state.kind === 'locked') return state.bands.join(', ')
  return state.kind === 'automatic' ? '(automatic)' : '(unknown)'
}

// ── Group ─────────────────────────────────────────────────────────────────────

export default function Locking() {
  // Shares the home poll rather than fetching /api/network/signal separately;
  // `refresh` re-reads that batch after a change so the read-back arrives promptly.
  const { data: home, status: homeStatus, refresh } = useHome()
  const signal = home?.signal ?? null
  const caps = useResource('modem-capabilities', api.modemCapabilities)
  const capabilities = caps.data

  // Observations come from the heartbeat; drafts are owned here so one reset can re-baseline both locks.
  // The SA control reads SA state only (never the NSA lock).
  const [lte, dispatchLte] = useDraft(bandSpec, lteObservation(signal), signal)
  const [nr, dispatchNr] = useDraft(bandSpec, nrSaObservation(signal), signal)
  const [mode, dispatchMode] = useDraft(modeSpec, signal?.net_select, signal)

  // One device operation at a time; the ref stops a repeated click from starting a second one.
  const [busy, setBusy] = useState(false)
  const busyRef = useRef(false)
  const run = useCallback(async (fn: () => Promise<void>) => {
    if (busyRef.current) return
    busyRef.current = true
    setBusy(true)
    try {
      await fn()
    } finally {
      busyRef.current = false
      setBusy(false)
    }
  }, [])
  const ops: Ops = { busy, run }

  /** Confirm, then submit exactly the reviewed tuple once (request semantics unchanged). */
  async function lockCell(cell: CellTuple, successText: string) {
    const frozen: CellTuple = { ...cell }
    await run(async () => {
      const ok = await confirm(cellLockConfirm(frozen))
      if (!ok) return
      try {
        if (frozen.tech === 'nr') await api.cellLockNr(frozen.pci, frozen.earfcn, frozen.band ?? '')
        else await api.cellLockLte(frozen.pci, frozen.earfcn)
        toast(successText)
        refresh()
      } catch (e) {
        toastError(e, 'Cell lock failed')
      }
    })
  }

  const lockServingCell = (cell: CellTuple) => lockCell(cell, `Locked to ${cell.tech === 'nr' ? 'NR' : 'LTE'} cell PCI ${cell.pci}`)

  async function resetBands() {
    await run(async () => {
      const ok = await confirm(bandResetConfirm())
      if (!ok) return
      dispatchLte({ type: 'submit', snapshot: [] })
      dispatchNr({ type: 'submit', snapshot: [] })
      try {
        await api.bandLockReset()
        dispatchLte({ type: 'applied', snapshot: [] })
        dispatchNr({ type: 'applied', snapshot: [] })
        toast('Band reset request accepted')
        refresh()
      } catch (e) {
        dispatchLte({ type: 'failed' })
        dispatchNr({ type: 'failed' })
        toastError(e, 'Reset failed')
      }
    })
  }

  async function resetCells() {
    await run(async () => {
      const ok = await confirm(cellResetConfirm())
      if (!ok) return
      try {
        await api.cellLockReset()
        toast('Cell lock reset request accepted')
        refresh()
      } catch (e) {
        toastError(e, 'Reset failed')
      }
    })
  }

  if (!signal) {
    return (
      <Loading label="Loading radio state" className="space-y-3">
        <Skeleton className="h-40" />
        <Skeleton className="h-56" />
      </Loading>
    )
  }

  return (
    <div className="space-y-3">
      {homeStatus === 'stale' && (
        <InlineStatus kind="stale" action={{ label: 'Retry', onClick: refresh }}>
          Radio state could not be refreshed; showing the last values received.
        </InlineStatus>
      )}

      {capabilities ? (
        <>
          {caps.status === 'stale' && (
            <InlineStatus kind="stale" action={{ label: 'Retry', onClick: caps.refresh, loading: caps.refreshing }}>
              Firmware capabilities could not be refreshed; using the last values received.
            </InlineStatus>
          )}
          <NetworkMode modes={capabilities.network_modes} state={mode} dispatch={dispatchMode} ops={ops} onApplied={refresh} />
        </>
      ) : caps.status === 'error' ? (
        <Card title="Network mode and bands">
          <InlineStatus kind="error" action={{ label: 'Retry', onClick: caps.refresh, loading: caps.refreshing }}>
            Firmware capability data could not be read{caps.error ? ` (${caps.error})` : ''}. Radio mode and band changes are unavailable until it loads.
          </InlineStatus>
        </Card>
      ) : (
        <Loading label="Loading firmware capabilities">
          <Skeleton className="h-40" />
        </Loading>
      )}

      <ServingCells signal={signal} ops={ops} onLock={lockServingCell} />

      {capabilities && (
        <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
          <BandLock
            title="NR 5G band lock"
            description="Allowed NR bands. Works in 5G SA mode only — firmware does not support NSA band locking."
            supported={capabilities.nr_sa_bands}
            type="nr"
            state={nr}
            dispatch={dispatchNr}
            ops={ops}
            onApplied={refresh}
          />
          <BandLock
            title="LTE band lock"
            description="Allowed LTE bands."
            supported={capabilities.lte_bands}
            type="lte"
            state={lte}
            dispatch={dispatchLte}
            ops={ops}
            onApplied={refresh}
          />
        </div>
      )}

      <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
        <CellLock
          type="nr"
          observed={signal.nr_cell_lock_state}
          serving={cellLockServing(signal.nr_cell_lock_state, signal.nr_carriers)}
          ops={ops}
          onLock={lockCell}
          onReset={resetCells}
        />
        <CellLock
          type="lte"
          observed={signal.lte_cell_lock_state}
          serving={cellLockServing(signal.lte_cell_lock_state, signal.lte_carriers)}
          ops={ops}
          onLock={lockCell}
          onReset={resetCells}
        />
      </div>

      <Card title="Reset locks">
        <p className="mb-3 text-meta text-ink2">
          Remove all band and cell locks; the modem returns to automatic selection.
        </p>
        <div className="flex flex-wrap gap-2">
          <Button variant="outline" onClick={resetBands} disabled={busy}>
            Reset bands to automatic
          </Button>
          <Button variant="outline" onClick={resetCells} disabled={busy}>
            Reset cell locks
          </Button>
        </div>
      </Card>

      <Card title="Diagnostics">
        <div className="tnum space-y-1 break-all font-mono text-caption text-ink3">
          <p>
            LTE lock (raw): <span className="text-ink">{signal.raw_lte_band_lock || '(empty)'}</span>
          </p>
          <p>
            NR lock (raw): <span className="text-ink">{signal.raw_nr_band_lock || '(empty)'}</span>
          </p>
          <p>
            LTE lock (parsed): <span className="text-ink">{describeLockState(signal.lte_band_lock_state)}</span>
          </p>
          <p>
            NR SA lock (parsed): <span className="text-ink">{describeLockState(signal.nr_sa_band_lock_state)}</span>
          </p>
          <p>
            NR cell lock (raw): <span className="text-ink">{signal.raw_nr_cell_lock === undefined ? '(not reported)' : signal.raw_nr_cell_lock || '(empty)'}</span>
          </p>
          <p>
            LTE cell lock (raw): <span className="text-ink">{signal.raw_lte_cell_lock === undefined ? '(not reported)' : signal.raw_lte_cell_lock || '(empty)'}</span>
          </p>
          <p>
            Network mode: <span className="text-ink">{signal.net_select || '(unknown)'}</span>
          </p>
        </div>
      </Card>
    </div>
  )
}
