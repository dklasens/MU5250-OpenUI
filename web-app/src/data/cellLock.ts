// Cell-lock observation parsing. Pure helpers, no I/O.
//
// Verified firmware encodings (HK B04, nwinfo_get_netinfo):
//   NR   lock_nr_cell   '431,423410,1'   pci,nr-arfcn,band (band digits only, no 'n')
//   LTE  lock_lte_cell  '312,3650'       pci,earfcn
// The lock persists in uci `zte_nwinfo.cell_lock`. Unlocked has two spellings:
//   ''                   never locked since a reset of the config
//   '0,0,0' / '0,0'      after nwinfo_reset_band_cell_setting (POST /api/cell/lock/reset)
//
// Missing / non-string           -> unknown (absence is not proof of no lock)
// '' or every part zero          -> none
// right part count, all in range -> locked
// anything else                  -> unknown (never a guessed lock)
//
// Note: LTE '0,0' is read as unlocked even though PCI 0 / EARFCN 0 is a real B1
// cell, because that is the firmware's own reset value.

import type { CarrierComponent, CellLockState } from '../types'

export type { CellLockState }

const UNKNOWN: CellLockState = { kind: 'unknown' }
const NONE: CellLockState = { kind: 'none' }

const MAX_LTE_PCI = 503
const MAX_NR_PCI = 1007
const MAX_EARFCN = 262143
const MAX_NR_ARFCN = 3279165
const MAX_NR_BAND = 1024

function digitParts(raw: unknown, count: number): number[] | null | 'empty' {
  if (typeof raw !== 'string') return null
  const t = raw.trim()
  if (t === '') return 'empty'
  const parts = t.split(',').map((p) => p.trim())
  if (parts.length !== count || !parts.every((p) => /^\d+$/.test(p))) return null
  return parts.map(Number)
}

/** `lock_nr_cell`: "pci,nr-arfcn,band". */
export function parseNrCellLock(raw: unknown): CellLockState {
  const p = digitParts(raw, 3)
  if (p === null) return UNKNOWN
  if (p === 'empty' || p.every((n) => n === 0)) return NONE
  const [pci, arfcn, band] = p
  if (pci > MAX_NR_PCI || arfcn > MAX_NR_ARFCN || band < 1 || band > MAX_NR_BAND) return UNKNOWN
  return { kind: 'locked', pci, arfcn, band }
}

/** `lock_lte_cell`: "pci,earfcn". */
export function parseLteCellLock(raw: unknown): CellLockState {
  const p = digitParts(raw, 2)
  if (p === null) return UNKNOWN
  if (p === 'empty' || p.every((n) => n === 0)) return NONE
  const [pci, arfcn] = p
  if (pci > MAX_LTE_PCI || arfcn > MAX_EARFCN) return UNKNOWN
  return { kind: 'locked', pci, arfcn }
}

export type CellLockServing =
  /** The RAT's serving primary cell is the locked cell. */
  | { kind: 'on' }
  /** The modem is not on the locked cell; `serving` is that RAT's primary cell, if any. */
  | { kind: 'off'; serving?: CarrierComponent }
  /** A primary cell is reported but without a valid PCI, so it cannot be compared. */
  | { kind: 'unknown' }

/**
 * Compare a lock with the serving primary cell (PCC) of the same RAT, from validated
 * carriers. Returns null unless the lock is known to be set.
 */
export function cellLockServing(state: CellLockState, carriers: CarrierComponent[]): CellLockServing | null {
  if (state.kind !== 'locked') return null
  const pcc = carriers.find((c) => c.label === 'PCC')
  if (!pcc) return { kind: 'off' }
  if (pcc.pci === undefined) return { kind: 'unknown' }
  const pccBand = Number(pcc.band.replace(/\D/g, '')) || undefined
  const bandDiffers = state.band !== undefined && pccBand !== undefined && state.band !== pccBand
  if (pcc.pci === state.pci && pcc.earfcn === state.arfcn && !bandDiffers) return { kind: 'on' }
  return { kind: 'off', serving: pcc }
}
