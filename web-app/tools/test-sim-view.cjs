// SIM Management (design.md § Device-switching lists): the eSIM view mapper and the tab's presentation
// logic. Fixtures are synthetic: masked identifiers are …0000-…0003, handles are made up.
const { test } = require('node:test')
const assert = require('node:assert/strict')
const { loadTs } = require('./ts-loader.cjs')

const client = {
  get: async () => ({}), post: async () => ({}), put: async () => ({}), req: async () => ({}), readCsv: async () => '',
}
const api = loadTs('data/api.ts', { stubs: { client } })
const sim = loadTs('features/modem/simView.ts')

const RAW = {
  status: 'ready',
  available: true,
  diagnostic: null,
  card: { id: '…0000', firmware: '37.4.3', sgp22_version: '2.3.0', profile_version: '2.3.1',
    free_nvm_bytes: 1428096, free_ram_bytes: 32601, pending_receipts: 5 },
  profiles: [
    { handle: 'pa', display_name: 'Amaysim', nickname: null, name: 'Amaysim', provider: 'Amaysim',
      iccid_masked: '…0001', state: 'enabled', class: 'operational', locked: false, warnings: [] },
    { handle: 'pb', display_name: 'Vodafone Fiji', nickname: 'Vodafone Fiji', name: null, provider: 'Vodafone Fiji',
      iccid_masked: '…0002', state: 'disabled', class: 'operational', locked: false, warnings: ['manual_apn', 'roaming_off', 'future_warning'] },
    { handle: 'pc', display_name: null, nickname: null, name: null, provider: null,
      iccid_masked: '…0003', state: 'disabled', class: 'test', locked: false, warnings: [] },
  ],
  revision: 7,
  freshness: { age_ms: 1200, stale: false, error: null },
  operation: null,
  receipts: { pending: 0, last_error: null, gave_up: false },
  actions: { enabled: true, blocked_by: null },
  hardware_validated: { switching: false },
}

const view = (over = {}) => api.mapEsimView({ ...RAW, ...over })
const op = (state, over = {}) => ({
  id: 'op-test-0001', kind: 'enable', target: 'pb', target_name: 'Vodafone Fiji', previous: 'pa',
  state, resolved: !['pending', 'reloading', 'verifying'].includes(state), message: null, ...over,
})

// ── Mapper ────────────────────────────────────────────────────────────────────

test('the view maps the agent shape and drops unknown warnings', () => {
  const v = view()
  assert.equal(v.status, 'ready')
  assert.equal(v.card.free_nvm_bytes, 1428096)
  assert.equal(v.profiles[1].profile_class, 'operational')
  assert.deepEqual(v.profiles[1].warnings, ['manual_apn', 'roaming_off'])
  assert.equal(v.switching_validated, false)
  assert.equal(v.revision, 7)
})

test('malformed fields fail closed: no actions, unknown status, no fabricated profiles', () => {
  const v = api.mapEsimView({ status: 'weird', profiles: [{ display_name: 'no handle' }, 3], actions: { enabled: 'yes' } })
  assert.equal(v.status, null)
  assert.deepEqual(v.profiles, [])
  assert.equal(v.actions.enabled, false)
  assert.equal(v.card, null)
  assert.equal(v.revision, null)
})

test('an operation whose state is final counts as resolved even without the flag', () => {
  const v = view({ operation: { id: 'op-x-000001', kind: 'disable', target: 'pa', state: 'busy' } })
  assert.equal(v.operation.resolved, true)
  assert.equal(view({ operation: { id: 'op-x-000001', kind: 'disable', target: 'pa', state: 'bogus' } }).operation, null)
})

test('the heartbeat summary is capability only', () => {
  assert.deepEqual(api.mapEsimSummary({ status: 'ready', available: true }), { status: 'ready', available: true })
  assert.deepEqual(api.mapEsimSummary({ status: 'unsupported' }), { status: 'unsupported', available: false })
})

test('operation ids satisfy the agent pattern and do not repeat', () => {
  const a = api.newOperationId()
  assert.match(a, /^[A-Za-z0-9_-]{8,64}$/)
  assert.notEqual(a, api.newOperationId())
})

// ── Ordering and names ──────────────────────────────────────────────────────

test('active first, then by name, unnamed last', () => {
  const v = view()
  const shuffled = [v.profiles[2], v.profiles[1], v.profiles[0]]
  assert.deepEqual(sim.sortProfiles(shuffled).map((p) => p.handle), ['pa', 'pb', 'pc'])
  const none = shuffled.map((p) => ({ ...p, state: 'disabled' }))
  assert.deepEqual(sim.sortProfiles(none).map((p) => p.handle), ['pa', 'pb', 'pc'])
})

test('an unnamed profile is labelled with its masked id, never its raw name', () => {
  assert.equal(sim.profileLabel({ display_name: null, iccid_masked: '…0003' }), 'Unnamed profile …0003')
  assert.equal(sim.profileLabel({ display_name: 'Vodafone Fiji', iccid_masked: '…0002' }), 'Vodafone Fiji')
})

// ── Nickname validation ──────────────────────────────────────────────────────

test('the nickname limit is 64 bytes of UTF-8, not 64 characters', () => {
  assert.equal(sim.nicknameError(''), null, 'empty clears the nickname')
  assert.equal(sim.nicknameError('a'.repeat(64)), null)
  assert.match(sim.nicknameError('a'.repeat(65)), /65 of 64 bytes/)
  assert.equal(sim.nicknameBytes('é'.repeat(32)), 64)
  assert.equal(sim.nicknameError('é'.repeat(32)), null)
  assert.match(sim.nicknameError('é'.repeat(33)), /66 of 64/)
  assert.equal(sim.nicknameError('Travel 🌴'), null)
})

test('control and invisible formatting characters are rejected', () => {
  for (const bad of ['tab\there', 'line\nbreak', 'evil‮txt', 'zero​width', 'bom﻿']) {
    assert.match(sim.nicknameError(bad), /control or invisible/, JSON.stringify(bad))
  }
})

// ── Actions and polling ──────────────────────────────────────────────────────

test('writes are disabled while a change is unresolved or the agent blocks them', () => {
  assert.equal(sim.writesDisabled(view()), false)
  for (const s of ['pending', 'reloading', 'verifying']) assert.equal(sim.writesDisabled(view({ operation: op(s) })), true, s)
  for (const s of ['succeeded', 'busy', 'refused', 'failed', 'unknown']) assert.equal(sim.writesDisabled(view({ operation: op(s) })), false, s)
  assert.equal(sim.writesDisabled(view({ actions: { enabled: false, blocked_by: 'helper_missing' } })), true)
  assert.equal(sim.writesDisabled(null), true)
})

test('polling is 30 s at rest and 3 s only while a change settles', () => {
  assert.equal(sim.pollInterval(view()), 30000)
  assert.equal(sim.pollInterval(view({ operation: op('reloading') })), 3000)
  assert.equal(sim.pollInterval(view({ operation: op('succeeded') })), 30000)
})

test('locked and non-operational profiles cannot be activated', () => {
  const v = view()
  assert.equal(sim.activateBlock(v.profiles[1]), null)
  assert.equal(sim.activateBlock({ ...v.profiles[1], locked: true }), 'locked')
  assert.equal(sim.activateBlock(v.profiles[2]), 'not_operational')
})

// ── Copy ──────────────────────────────────────────────────────────────────────

test('every operation state has copy, and busy says nothing changed', () => {
  for (const s of ['pending', 'reloading', 'verifying', 'succeeded', 'busy', 'refused', 'failed', 'unknown']) {
    assert.ok(sim.operationCopy(op(s)).text.length > 10, s)
  }
  const busy = sim.operationCopy(op('busy'))
  assert.equal(busy.tone, 'warn')
  assert.match(busy.text, /Nothing changed/)
  assert.match(busy.text, /few minutes/)
  assert.equal(sim.operationCopy(op('failed')).tone, 'error')
  assert.match(sim.operationCopy(op('unknown')).text, /may or may not/)
  assert.match(sim.operationCopy(op('succeeded')).text, /Vodafone Fiji is active on the card/)
  assert.match(sim.operationCopy(op('succeeded', { kind: 'disable' })).text, /Cellular service has stopped/)
})

test('activation names both profiles, explains the interruption and carries the warnings', () => {
  const v = view()
  const c = sim.activateConfirm(v.profiles[0], v.profiles[1])
  assert.equal(c.title, 'Activate Vodafone Fiji?')
  assert.match(c.body, /from Amaysim to Vodafone Fiji/)
  assert.match(c.body, /manual APN mode/)
  assert.match(c.body, /Data roaming is off/)
  assert.deepEqual(c.details.map((d) => d.label), ['Current', 'Requested'])
  assert.match(c.consequence, /Cellular data drops/)
  assert.match(c.consequence, /does not mean connected/)
  assert.match(c.recovery, /Activate Amaysim again/)
})

test('deactivation says no profile will remain active and service stops', () => {
  const c = sim.deactivateConfirm(view().profiles[0])
  assert.match(c.body, /No profile will remain active/)
  assert.match(c.consequence, /Cellular service stops/)
})

test('whole-tab states and stale copy', () => {
  assert.equal(sim.emptyState(view()), null)
  assert.match(sim.emptyState(view({ status: 'unsupported', profiles: [], card: null })).title, /Not an eSIM card/)
  assert.match(sim.emptyState(view({ status: 'helper_missing', profiles: [], card: null })).title, /unavailable/)
  assert.match(sim.emptyState(view({ profiles: [] })).title, /No profiles/)
  assert.match(sim.staleCopy('sim_reloading'), /reloading the SIM/)
  assert.match(sim.staleCopy('anything'), /last profile list/)
})

test('a PIN session refusal tells the user to sign in with the password', () => {
  assert.match(sim.writeErrorCopy(403, 'forbidden'), /password/)
  assert.match(sim.writeErrorCopy(423, 'locked'), /locked/)
})

// ── Phase 2: adding and deleting profiles ──────────────────────────────────────
// Codes are fake: smdp.example.com, TEST-MATCHING-ID.

const dl = (state, over = {}) => ({
  id: 'op-dl-000001', kind: 'download', target: null, target_name: null, previous: null, state,
  resolved: !['connecting', 'awaiting_confirmation', 'installing'].includes(state), message: null,
  step: null, preview: null, can_cancel: false, preview_expires_in_ms: null, ...over,
})

test('a download maps without a target and keeps only its own fields', () => {
  const v = view({
    operation: { id: 'op-dl-000001', kind: 'download', target: null, state: 'awaiting_confirmation', step: 'preview',
      preview: { provider: 'Test Mobile', name: 'Travel 7 days', class: 'operational' }, can_cancel: true,
      preview_expires_in_ms: 95000, resolved: true },
    receipts: { pending: 1, last_error: null, gave_up: false, sending: true },
  })
  const o = v.operation
  assert.equal(o.target, null)
  assert.equal(o.resolved, false, 'an in-flight state is never settled, whatever the flag says')
  assert.deepEqual(o.preview, { provider: 'Test Mobile', name: 'Travel 7 days', profile_class: 'operational' })
  assert.equal(o.can_cancel, true)
  assert.equal(o.preview_expires_in_ms, 95000)
  assert.equal(v.receipts.sending, true)
  // Only a download may lack a target; other kinds without one are dropped (fail closed).
  assert.equal(view({ operation: { id: 'op-x-000001', kind: 'enable', target: null, state: 'pending' } }).operation, null)
  // can_cancel is never trusted once installing.
  const installing = view({ operation: { id: 'op-dl-000001', kind: 'download', state: 'installing', can_cancel: true } })
  assert.equal(installing.operation.can_cancel, false)
  assert.equal(installing.operation.preview_expires_in_ms, null)
})

test('activation codes: shape, host and the $1 confirmation flag', () => {
  const ok = sim.checkActivationCode('  LPA:1$SMDP.example.com$TEST-MATCHING-ID  ')
  assert.deepEqual(ok, { ok: true, host: 'smdp.example.com', needsConfirmation: false })
  assert.equal(sim.checkActivationCode('lpa:1$smdp.example.com$TEST').ok, true, 'prefix is case-insensitive')
  assert.equal(sim.checkActivationCode('LPA:1$smdp.example.com:8443$TEST').ok, true)
  assert.equal(sim.checkActivationCode('LPA:1$smdp.example.com$TEST$1.3.6.1.4.1$1').needsConfirmation, true)
  assert.equal(sim.checkActivationCode('LPA:1$smdp.example.com$TEST$$1').needsConfirmation, true)
  assert.equal(sim.checkActivationCode('LPA:1$smdp.example.com$TEST$$0').needsConfirmation, false)
  for (const bad of ['', 'LPA:2$smdp.example.com$X', 'LPA:1$smdp.example.com', 'LPA:1$10.0.0.1$X',
    'LPA:1$localhost$X', 'LPA:1$evil.example/path$X', 'LPA:1$smdp.example.com:0$X', 'LPA:1$smdp.example.com$a‮b',
    `LPA:1$smdp.example.com$${'x'.repeat(600)}`]) {
    const r = sim.checkActivationCode(bad)
    assert.equal(r.ok, false, JSON.stringify(bad))
    assert.ok(!r.error.includes('TEST') && !r.error.includes('$X'), 'errors never quote the code')
  }
})

test('confirmation codes are required only when flagged, and bounded', () => {
  assert.equal(sim.confirmationCodeError('', false), null)
  assert.match(sim.confirmationCodeError('', true), /needs the confirmation code/)
  assert.equal(sim.confirmationCodeError('1234', true), null)
  assert.match(sim.confirmationCodeError('x'.repeat(65), true), /too long/)
  assert.match(sim.confirmationCodeError('12\t34', true), /control or invisible/)
})

test('download steps read in plain words, in order, up to the preview', () => {
  assert.equal(sim.stepLabel('es9p_get_bound_profile_package'), 'downloading the profile')
  assert.equal(sim.stepLabel('es9p_cancel_session'), 'telling the server it was cancelled')
  assert.equal(sim.stepLabel(null), 'starting')
  assert.equal(sim.stepLabel('something_new'), 'working')
  const steps = sim.connectingSteps('es10b_authenticate_server')
  assert.equal(steps.length, 5)
  assert.deepEqual(steps.map((s) => s.status), ['done', 'done', 'current', 'todo', 'todo'])
  assert.equal(sim.connectingSteps(null)[0].status, 'current')
})

test('download states have copy, and failures say whether the QR is reusable', () => {
  for (const s of ['connecting', 'awaiting_confirmation', 'installing', 'succeeded', 'cancelled', 'failed', 'unknown'])
    assert.ok(sim.operationCopy(dl(s)).text.length > 10, s)
  assert.match(sim.operationCopy(dl('installing')).text, /15–30 seconds/)
  assert.match(sim.operationCopy(dl('installing')).text, /can’t be cancelled/)
  assert.match(sim.operationCopy(dl('succeeded', { target_name: 'Holiday' })).text, /Holiday is installed on the card, inactive/)
  assert.match(sim.operationCopy(dl('cancelled', { message: 'cancelled' })).text, /likely still usable/)
  assert.match(sim.operationCopy(dl('cancelled', { message: 'cancelled: not confirmed in time' })).text, /within 2 minutes/)
  assert.match(sim.operationCopy(dl('cancelled', { message: 'cancelled: declined' })).text, /may or may not/)
  const consumed = sim.operationCopy(dl('failed', { step: 'es10b_load_bound_profile_package', message: 'es10b_load_bound_profile_package: install failed' }))
  assert.equal(consumed.tone, 'error')
  assert.match(consumed.text, /likely been used up/)
  assert.match(sim.operationCopy(dl('failed', { step: 'es9p_authenticate_client', message: 'The matchingID is not found' })).text, /may or may not/)
  assert.match(sim.operationCopy(op('succeeded', { kind: 'delete' })).text, /deleted/)
})

test('a download in flight blocks other writes and speeds up polling', () => {
  for (const s of ['connecting', 'awaiting_confirmation', 'installing']) {
    const v = view({ operation: dl(s), actions: { enabled: false, blocked_by: 'operation_in_progress' } })
    assert.equal(sim.writesDisabled(v), true, s)
    assert.equal(sim.canAddProfile(v), false, s)
    assert.equal(sim.pollInterval(v), 3000, s)
  }
  assert.equal(sim.canAddProfile(view()), true)
  assert.equal(sim.canAddProfile(view({ status: 'unsupported' })), false)
})

test('the preview countdown formats minutes and seconds', () => {
  assert.equal(sim.formatCountdown(120000), '2:00')
  assert.equal(sim.formatCountdown(95001), '1:36')
  assert.equal(sim.formatCountdown(-5), '0:00')
  assert.equal(sim.formatCountdown(null), null)
})

test('only inactive, unlocked profiles can be deleted, and the confirm says it is final', () => {
  const v = view()
  assert.equal(sim.deleteBlock(v.profiles[0]), 'active')
  assert.equal(sim.deleteBlock(v.profiles[1]), null)
  assert.equal(sim.deleteBlock({ ...v.profiles[1], locked: true }), 'locked')
  const c = sim.deleteConfirm(v.profiles[1])
  assert.equal(c.title, 'Delete Vodafone Fiji?')
  assert.match(c.body, /can’t be undone/)
  assert.match(c.recovery, /single-use/)
  assert.deepEqual(c.details, [{ label: 'Delete', value: 'Vodafone Fiji …0002' }])
})

test('receipt runs report progress without repeating the pending line', () => {
  const r = (over) => view({ receipts: { pending: 0, last_error: null, gave_up: false, sending: false, ...over } })
  assert.equal(sim.receiptsCopy(r({})), null)
  assert.match(sim.receiptsCopy(r({ sending: true })), /Sending receipts/)
  assert.match(sim.receiptsCopy(r({ last_error: 'rsp.example.com: HTTP 500' })), /Last attempt: rsp.example.com/)
  assert.equal(sim.receiptsCopy(r({ pending: 2, last_error: 'x' })), null, 'the pending line already shows it')
})

test('no mobile data and PIN sessions get actionable copy', () => {
  assert.match(sim.writeErrorCopy(409, 'the router has no mobile data connection'), /router’s own data/)
  assert.match(sim.writeErrorCopy(403, 'forbidden'), /add or delete/)
})
