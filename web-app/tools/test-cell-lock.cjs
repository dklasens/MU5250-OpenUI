// Cell-lock parsing. Formats verified on HK B04 (2026-11-14): lock_nr_cell
// "pci,nr-arfcn,band", lock_lte_cell "pci,earfcn"; unlocked is '' or all zeros.
const { test } = require('node:test')
const assert = require('node:assert/strict')
const { loadTs } = require('./ts-loader.cjs')
const cl = loadTs('data/cellLock.ts')
const api = loadTs('data/api.ts', { stubs: { client: { get: async () => ({}), post: async () => ({}), put: async () => ({}), req: async () => ({}), readCsv: async () => '' } } })

const UNKNOWN = { kind: 'unknown' }
const NONE = { kind: 'none' }

test('NR: device lock string parses to pci, arfcn and band', () => {
  assert.deepEqual(cl.parseNrCellLock('431,423410,1'), { kind: 'locked', pci: 431, arfcn: 423410, band: 1 })
  assert.deepEqual(cl.parseNrCellLock(' 0,643392,78 '), { kind: 'locked', pci: 0, arfcn: 643392, band: 78 }, 'PCI 0 is valid')
})

test('LTE: device lock string parses to pci and earfcn', () => {
  assert.deepEqual(cl.parseLteCellLock('312,3650'), { kind: 'locked', pci: 312, arfcn: 3650 })
  assert.deepEqual(cl.parseLteCellLock('0,3650'), { kind: 'locked', pci: 0, arfcn: 3650 })
})

test('both unlocked spellings are none: empty and all zeros (after reset)', () => {
  for (const v of ['', '  ', '0,0,0', '0, 0, 0']) assert.deepEqual(cl.parseNrCellLock(v), NONE, JSON.stringify(v))
  for (const v of ['', '0,0', '00,0']) assert.deepEqual(cl.parseLteCellLock(v), NONE, JSON.stringify(v))
})

test('absent or non-string values are unknown, never none', () => {
  for (const v of [undefined, null, 0, 431, true, {}, ['431', '423410', '1']]) {
    assert.deepEqual(cl.parseNrCellLock(v), UNKNOWN, String(v))
    assert.deepEqual(cl.parseLteCellLock(v), UNKNOWN, String(v))
  }
})

test('malformed values are unknown, not a guessed lock', () => {
  const nrBad = ['431,423410', '431,423410,1,2', '431,423410,n1', '431,,1', 'a,b,c', '-1,423410,1', '431.5,423410,1',
    '1008,423410,1', '431,9999999,1', '431,423410,0', '0,0', '0,0,0,0', ',,']
  for (const v of nrBad) assert.deepEqual(cl.parseNrCellLock(v), UNKNOWN, v)
  const lteBad = ['312', '312,3650,8', '312,abc', '504,3650', '312,262144', '0,0,0', ',']
  for (const v of lteBad) assert.deepEqual(cl.parseLteCellLock(v), UNKNOWN, v)
})

// ── Comparison with the serving primary cell ───────────────────────────────────

const pcc = (over) => ({ label: 'PCC', band: 'n1', pci: 431, earfcn: 423410, bandwidth: '20 MHz', ...over })

test('cellLockServing: only a known lock is compared', () => {
  assert.equal(cl.cellLockServing(UNKNOWN, [pcc()]), null)
  assert.equal(cl.cellLockServing(NONE, [pcc()]), null)
})

test('cellLockServing: on when PCI, ARFCN and band match the PCC; off otherwise', () => {
  const lock = cl.parseNrCellLock('431,423410,1')
  assert.deepEqual(cl.cellLockServing(lock, [pcc({ label: 'SCC0', pci: 1 }), pcc()]), { kind: 'on' })
  for (const other of [pcc({ pci: 432 }), pcc({ earfcn: 423411 }), pcc({ band: 'n3' })]) {
    assert.deepEqual(cl.cellLockServing(lock, [other]), { kind: 'off', serving: other })
  }
  // An SCC on the locked cell does not count: only the primary cell is compared.
  assert.deepEqual(cl.cellLockServing(lock, [pcc({ label: 'SCC0' })]), { kind: 'off' })
  assert.deepEqual(cl.cellLockServing(lock, []), { kind: 'off' }, 'no serving cell of this RAT')
  assert.deepEqual(cl.cellLockServing(lock, [pcc({ pci: undefined })]), { kind: 'unknown' })
  const lte = cl.parseLteCellLock('312,3650')
  assert.deepEqual(cl.cellLockServing(lte, [pcc({ band: 'B8', pci: 312, earfcn: 3650 })]), { kind: 'on' })
})

// ── mapSignal wiring ─────────────────────────────────────────────────────────

test('mapSignal: absent fields are unknown; raw strings are kept for diagnostics', () => {
  const s = api.mapSignal({ network_type: 'SA' })
  assert.deepEqual(s.nr_cell_lock_state, UNKNOWN)
  assert.deepEqual(s.lte_cell_lock_state, UNKNOWN)
  assert.equal(s.raw_nr_cell_lock, undefined)
  const r = api.mapSignal({ network_type: 'SA', lock_nr_cell: '0,0,0', lock_lte_cell: '' })
  assert.deepEqual(r.nr_cell_lock_state, NONE)
  assert.deepEqual(r.lte_cell_lock_state, NONE)
  assert.equal(r.raw_nr_cell_lock, '0,0,0')
  assert.equal(r.raw_lte_cell_lock, '')
})

test('mapSignal + cellLockServing: numeric and string serving values compare the same', () => {
  const numeric = { network_type: 'SA', lock_nr_cell: '431,423410,1', nr5g_pci: 431, nr5g_action_channel: 423410, nr5g_action_band: 'n1' }
  const strings = { ...numeric, nr5g_pci: '431', nr5g_action_channel: '423410', nr5g_action_band: '1' }
  for (const raw of [numeric, strings]) {
    const s = api.mapSignal(raw)
    assert.deepEqual(s.nr_cell_lock_state, { kind: 'locked', pci: 431, arfcn: 423410, band: 1 })
    assert.deepEqual(cl.cellLockServing(s.nr_cell_lock_state, s.nr_carriers), { kind: 'on' })
  }
  const elsewhere = api.mapSignal({ ...strings, nr5g_pci: '745' })
  assert.equal(cl.cellLockServing(elsewhere.nr_cell_lock_state, elsewhere.nr_carriers).kind, 'off')
})
