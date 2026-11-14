// R11 confirmation content for connection-affecting radio changes. Pure builders: the caller freezes
// the reviewed values, builds the options from them, and submits exactly those values after Confirm.
// Nothing here reads live state or secrets.

import { describeBandLock } from '../../../data/bands'
import type { BandLockState, CarrierComponent, CellLockState } from '../../../types'
import type { ConfirmOptions } from '../../../ui/feedback'

export type Rat = 'nr' | 'lte'

export const ratName = (t: Rat) => (t === 'nr' ? 'NR (5G SA)' : 'LTE')
export const bandName = (t: Rat, band: number) => (t === 'nr' ? `n${band}` : `B${band}`)
export const bandList = (t: Rat, bands: number[]) => bands.map((b) => bandName(t, b)).join(', ')

/** Plain-language rendering of an observed lock, using the capability band list. */
export function describeObserved(type: Rat, state: BandLockState, supported: number[]): string {
  const d = describeBandLock(state, supported)
  switch (d.kind) {
    case 'unknown':
      return 'Unknown (the modem did not report it)'
    case 'automatic':
      return 'Automatic (no band restriction)'
    case 'all':
      return 'All bands'
    case 'subset':
      return bandList(type, d.bands)
  }
}

const arfcnName = (t: Rat) => (t === 'nr' ? 'NR-ARFCN' : 'EARFCN')

/** "PCI 431, NR-ARFCN 423410, band n1" — the same order the lock buttons and confirmation use. */
export function formatCell(type: Rat, pci: number | string, arfcn: number | string, band?: number | string): string {
  const b = type === 'nr' && band !== undefined && band !== '' ? `, band n${band}` : ''
  return `PCI ${pci}, ${arfcnName(type)} ${arfcn}${b}`
}

/** Plain-language rendering of an observed cell lock. */
export function describeCellLock(type: Rat, state: CellLockState): string {
  switch (state.kind) {
    case 'unknown':
      return 'Unknown (the modem did not report it)'
    case 'none':
      return 'None (automatic cell selection)'
    case 'locked':
      return formatCell(type, state.pci, state.arfcn, state.band)
  }
}

/** A serving carrier in the same format; its band string is already "n78" / "B8". */
export function formatServing(type: Rat, c: CarrierComponent): string {
  const band = c.band.replace(/\D/g, '')
  return formatCell(type, c.pci ?? 'unavailable', c.earfcn, band || undefined) + (type === 'lte' && c.band ? `, band ${c.band}` : '')
}

/** Internet (WAN) can drop; the dashboard is served by the router and stays reachable over its LAN. */
const WAN_CONSEQUENCE =
  'The modem reconnects, so mobile Internet (WAN) may drop for a while. This dashboard is served by the router itself, so it stays reachable over USB or Wi-Fi LAN; only the Internet connection is interrupted.'

export function networkModeConfirm(next: { value: string; label: string }, current: { value?: string; label?: string }): ConfirmOptions {
  return {
    title: 'Change network mode?',
    confirmLabel: 'Change mode',
    kind: 'connection',
    details: [
      { label: 'Operation', value: 'Change network mode' },
      { label: 'New mode', value: next.label },
      { label: 'Current mode', value: current.label ?? current.value ?? 'Unknown' },
    ],
    consequence: WAN_CONSEQUENCE,
    recovery: current.label
      ? `To undo, select "${current.label}" and apply again, or use "Reset bands to automatic" if a band lock is limiting service.`
      : 'If service does not return, choose a broader mode and apply again, or use "Reset bands to automatic".',
  }
}

export function bandLockConfirm(type: Rat, bands: number[]): ConfirmOptions {
  return {
    title: `Lock ${type === 'nr' ? 'NR' : 'LTE'} bands?`,
    confirmLabel: 'Apply lock',
    kind: 'connection',
    details: [
      { label: 'Operation', value: `Lock ${ratName(type)} bands` },
      { label: 'Bands', value: bandList(type, bands) },
    ],
    consequence: `${WAN_CONSEQUENCE} If none of the selected bands has coverage here, there will be no mobile service until the lock is removed.`,
    recovery: 'Use "Reset bands to automatic" on this page to return to automatic band selection.',
  }
}

export function bandResetConfirm(): ConfirmOptions {
  return {
    title: 'Reset band locks to automatic?',
    confirmLabel: 'Reset bands',
    kind: 'connection',
    details: [
      { label: 'Operation', value: 'Reset band locks' },
      { label: 'Affected', value: 'LTE and NR (5G SA) band locks' },
    ],
    consequence: WAN_CONSEQUENCE,
    recovery: 'Lock bands again from the band cards below if you still want a restriction.',
  }
}

export interface CellTuple {
  tech: Rat
  pci: string
  earfcn: string
  /** NR band number (digits only); not used for LTE. */
  band?: string
}

export function cellLockConfirm(cell: CellTuple): ConfirmOptions {
  const details = [
    { label: 'Operation', value: `Lock ${ratName(cell.tech)} cell` },
    { label: 'PCI', value: cell.pci },
    { label: cell.tech === 'nr' ? 'NR-ARFCN' : 'EARFCN', value: cell.earfcn },
  ]
  if (cell.tech === 'nr' && cell.band) details.push({ label: 'Band', value: `n${cell.band}` })
  return {
    title: 'Lock to this cell?',
    confirmLabel: 'Lock cell',
    kind: 'connection',
    details,
    consequence: `${WAN_CONSEQUENCE} If this cell cannot be used, there will be no mobile service until the lock is removed.`,
    recovery: 'Use "Reset cell locks" on this page to return to automatic cell selection.',
  }
}

export function cellResetConfirm(): ConfirmOptions {
  return {
    title: 'Reset cell locks to automatic?',
    confirmLabel: 'Reset cells',
    kind: 'connection',
    details: [
      { label: 'Operation', value: 'Reset cell locks' },
      { label: 'Affected', value: 'LTE and NR cell locks' },
    ],
    consequence: WAN_CONSEQUENCE,
    recovery: 'Lock a cell again from the serving cells list or the cell lock forms if you still want one.',
  }
}
