import { useState } from 'react'
import type { CellLockServing } from '../../../data/cellLock'
import type { CellLockState } from '../../../types'
import { Button, Field, Input } from '../../../ui/controls'
import { toast } from '../../../ui/feedback'
import { Card, InlineStatus } from '../../../ui/primitives'
import { describeCellLock, formatServing, type CellTuple, type Rat } from './confirmations'
import type { Ops } from './ops'

/**
 * Manual cell lock by PCI and ARFCN, plus the lock the modem reports. Validation is unchanged; the
 * shared flow confirms and submits. The observed lock is shown only as read from the device.
 */
export function CellLock({
  type,
  observed,
  serving,
  ops,
  onLock,
  onReset,
}: {
  type: Rat
  observed: CellLockState
  /** Comparison with the serving primary cell of this RAT; null unless a lock is set. */
  serving: CellLockServing | null
  ops: Ops
  onLock: (cell: CellTuple, successText: string) => Promise<void>
  onReset: () => void
}) {
  const rat = type === 'nr' ? 'NR' : 'LTE'
  const [pci, setPci] = useState('')
  const [earfcn, setEarfcn] = useState('')
  const [band, setBand] = useState('')

  async function apply() {
    if (!pci || !earfcn) {
      toast('PCI and ARFCN are required', 'err')
      return
    }
    if (type === 'nr' && !band) {
      toast('Band is required for NR cell lock', 'err')
      return
    }
    const cell: CellTuple = { tech: type, pci, earfcn, band: type === 'nr' ? band : undefined } // frozen copy
    await onLock(cell, `${type === 'nr' ? 'NR' : 'LTE'} cell locked (PCI ${pci})`)
  }

  return (
    <Card title={`${rat} cell lock`}>
      <p className="mb-2.5 text-meta text-ink2">
        Lock to a specific cell by PCI and {type === 'nr' ? 'NR-ARFCN' : 'EARFCN'}.
      </p>
      <p className="mb-3 text-meta text-ink2">
        Current lock: <span className="font-semibold text-ink">{describeCellLock(type, observed)}</span>
      </p>
      {serving?.kind === 'on' && (
        <p className="mb-3 text-meta text-ink2">
          The modem is on the locked cell. “Reset cell locks” below returns to automatic cell selection.
        </p>
      )}
      {serving?.kind === 'unknown' && (
        <p className="mb-3 text-meta text-ink2">
          Unknown whether the modem is on the locked cell: the serving {rat} cell has no valid PCI.
        </p>
      )}
      {serving?.kind === 'off' && (
        <InlineStatus kind="warn" className="mb-3" action={{ label: 'Reset cell locks', onClick: onReset }}>
          The modem is not on the locked cell
          {serving.serving ? ` (serving ${rat} cell: ${formatServing(type, serving.serving)})` : ` and reports no serving ${rat} cell`}. A lock
          to a cell the modem cannot use usually means no mobile service.
        </InlineStatus>
      )}
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
          <Button variant="primary" onClick={apply} disabled={ops.busy} className="w-full">
            Lock
          </Button>
        </div>
      </div>
    </Card>
  )
}
