import type { Page } from '@playwright/test'
import { fileURLToPath } from 'node:url'
import { test, expect, type MockAgent } from './support/harness'
import { openApp, VIEWPORTS } from './support/app'
import * as fx from './support/fixtures'

// Modem > SIM Management (design.md § Device-switching lists). Masked ids are synthetic (…0000-…0003).

type Json = Record<string, unknown>

function profile(over: Json): Json {
  return {
    handle: 'pa', display_name: 'Amaysim', nickname: null, name: 'Amaysim', provider: 'Amaysim',
    iccid_masked: '…0001', state: 'enabled', class: 'operational', locked: false, warnings: [], ...over,
  }
}

function view(over: Json = {}): Json {
  return {
    status: 'ready', available: true, diagnostic: null,
    card: { id: '…0000', firmware: '37.4.3', sgp22_version: '2.3.0', profile_version: '2.3.1',
      free_nvm_bytes: 1428096, free_ram_bytes: 32601, pending_receipts: 5 },
    profiles: [
      profile({}),
      profile({ handle: 'pb', display_name: 'Vodafone Fiji', nickname: 'Vodafone Fiji', name: null,
        provider: 'Vodafone Fiji', iccid_masked: '…0002', state: 'disabled', warnings: ['manual_apn', 'roaming_off'] }),
      profile({ handle: 'pc', display_name: null, name: null, provider: null, iccid_masked: '…0003',
        state: 'disabled', class: 'test' }),
    ],
    revision: 7,
    freshness: { age_ms: 1200, stale: false, error: null },
    operation: null,
    receipts: { pending: 0, last_error: null, gave_up: false, sending: false },
    actions: { enabled: true, blocked_by: null },
    hardware_validated: { switching: false },
    ...over,
  }
}

// ── Phase 2 fixtures (fake codes only) ─────────────────────────────────────────

const FAKE_CODE = 'LPA:1$smdp.example.com$TEST-MATCHING-ID'
const PREVIEW = { provider: 'Test Mobile', name: 'Travel 7 days', class: 'operational' }
const BUSY = { actions: { enabled: false, blocked_by: 'operation_in_progress' } }
const fixture = (name: string) => fileURLToPath(new URL(`./fixtures/${name}`, import.meta.url))

function download(state: string, over: Json = {}): Json {
  const inFlight = ['connecting', 'awaiting_confirmation', 'installing'].includes(state)
  return {
    id: 'op-dl-000001', kind: 'download', target: null, target_name: null, previous: null, state,
    resolved: !inFlight, message: null, step: null, preview: null,
    can_cancel: state === 'connecting' || state === 'awaiting_confirmation', preview_expires_in_ms: null, ...over,
  }
}

/** Serve a view the test can change between polls. */
function stateful(agent: MockAgent, initial: Json = view()) {
  const box = { current: initial }
  agent.on('GET', '/api/dashboard', { data: { ...fx.dashboard(), esim: { status: 'ready', available: true } } })
  agent.on('GET', '/api/esim/profiles', () => ({ data: box.current }))
  return box
}

function withEsim(agent: MockAgent, data: Json = view()) {
  agent.on('GET', '/api/dashboard', { data: { ...fx.dashboard(), esim: { status: 'ready', available: true } } })
  agent.on('GET', '/api/esim/profiles', { data })
}

const openSim = (page: Page, opts: { theme?: 'light' | 'dark' } = {}) =>
  openApp(page, { group: 'modem', tab: 'SIM Management', ...opts })

test('the tab is hidden for an agent or SIM without eSIM support', async ({ page }) => {
  await openApp(page, { group: 'modem' })
  await expect(page.getByRole('tab', { name: 'SMS' })).toBeVisible()
  await expect(page.getByRole('tab', { name: 'SIM Management' })).toHaveCount(0)
})

test('an ordinary SIM swapped in hides the tab, even after it was shown this session', async ({ page, agent }) => {
  const heartbeat = { esim: { status: 'ready', available: true } }
  agent.on('GET', '/api/dashboard', () => ({ data: { ...fx.dashboard(), ...heartbeat } }))
  agent.on('GET', '/api/esim/profiles', { data: view() })
  await openSim(page)
  await expect(page.getByRole('tab', { name: 'SIM Management' })).toBeVisible()
  // The agent now reports the SIM in the slot is not an eSIM card.
  heartbeat.esim = { status: 'unsupported', available: false }
  const nav = page.getByRole('navigation')
  await nav.getByRole('button', { name: 'Home', exact: true }).click()
  await page.waitForResponse((r) => r.url().endsWith('/api/dashboard'))
  await nav.getByRole('button', { name: 'Modem', exact: true }).click()
  await expect(page.getByRole('tab', { name: 'SMS' })).toBeVisible()
  await expect(page.getByRole('tab', { name: 'SIM Management' })).toHaveCount(0)
})

test('card facts and profiles: active first, masked ids, no invented capacity', async ({ page, agent }) => {
  withEsim(agent)
  await openSim(page)
  const rows = page.getByRole('list', { name: 'eSIM profiles' }).getByRole('listitem')
  await expect(rows).toHaveCount(3)
  await expect(rows.nth(0)).toContainText('Amaysim')
  await expect(rows.nth(0)).toContainText('Active')
  await expect(rows.nth(1)).toContainText('Vodafone Fiji')
  await expect(rows.nth(2)).toContainText('Unnamed profile …0003')
  await expect(page.getByText('1.4 MB')).toBeVisible()
  await expect(page.getByText('%')).toHaveCount(0)
  await expect(page.getByText(/not yet been validated on this hardware/)).toBeVisible()
  await expect(rows.nth(2).getByRole('button', { name: /Activate/ })).toBeDisabled()
  expect(agent.mutations()).toEqual([])
})

test('activation confirms with Cancel focused, names both profiles, warns, and Cancel sends nothing', async ({ page, agent }) => {
  withEsim(agent)
  await openSim(page)
  await page.getByRole('button', { name: 'Activate Vodafone Fiji' }).click()
  const dialog = page.getByRole('dialog')
  await expect(dialog).toContainText('Activate Vodafone Fiji?')
  await expect(dialog).toContainText('from Amaysim to Vodafone Fiji')
  await expect(dialog).toContainText('manual APN mode')
  await expect(dialog).toContainText('Data roaming is off')
  await expect(dialog).toContainText('does not mean connected')
  await expect(dialog.getByRole('button', { name: 'Cancel' })).toBeFocused()
  await dialog.getByRole('button', { name: 'Cancel' }).click()
  expect(agent.mutations()).toEqual([])
})

test('a confirmed activation sends one guarded request and shows the settling state', async ({ page, agent }) => {
  // Like the agent, later reads report the change the POST started.
  const box = stateful(agent)
  const settling = view({
    operation: { id: 'op-x', kind: 'enable', target: 'pb', target_name: 'Vodafone Fiji', previous: 'pa',
      state: 'reloading', resolved: false, message: null },
    actions: { enabled: false, blocked_by: 'operation_in_progress' },
  })
  agent.on('POST', '/api/esim/enable', () => {
    box.current = settling
    return { data: settling, status: 202 }
  })
  await openSim(page)
  await page.getByRole('button', { name: 'Activate Vodafone Fiji' }).click()
  await page.getByRole('dialog').getByRole('button', { name: 'Activate' }).click()
  await expect(page.getByText(/modem is reloading the SIM/)).toBeVisible()
  const sent = agent.requests({ method: 'POST', path: '/api/esim/enable' })
  expect(sent).toHaveLength(1)
  expect(sent[0].headers['x-confirm']).toBe('true')
  expect(sent[0].body).toMatchObject({ handle: 'pb', revision: 7 })
  await expect(page.getByRole('button', { name: 'Deactivate Amaysim' })).toBeDisabled()
})

test('a PIN session is told to sign in with the password', async ({ page, agent }) => {
  withEsim(agent)
  agent.on('POST', '/api/esim/disable', { status: 403, error: 'this action needs a session signed in with the password, not the PIN' })
  await openSim(page)
  await page.getByRole('button', { name: 'Deactivate Amaysim' }).click()
  const dialog = page.getByRole('dialog')
  await expect(dialog).toContainText('No profile will remain active')
  await dialog.getByRole('button', { name: 'Deactivate' }).click()
  await expect(page.getByText(/Sign in with the password/)).toBeVisible()
})

test('a busy refusal says nothing changed', async ({ page, agent }) => {
  withEsim(agent, view({
    operation: { id: 'op-y', kind: 'enable', target: 'pb', target_name: 'Vodafone Fiji', previous: 'pa',
      state: 'busy', resolved: true, message: null },
  }))
  await openSim(page)
  await expect(page.getByText(/Nothing changed; it usually accepts a retry within a few minutes/)).toBeVisible()
})

test('rename counts bytes, refuses invisible characters, and keeps the draft on failure', async ({ page, agent }) => {
  withEsim(agent)
  agent.on('POST', '/api/esim/nickname', { status: 409, error: 'the profile list changed; refresh and try again' })
  await openSim(page)
  await page.getByRole('button', { name: 'Rename Amaysim' }).click()
  const field = page.getByLabel('Nickname for Amaysim')
  await field.fill('é'.repeat(33))
  await expect(page.getByText('Too long: 66 of 64 bytes.')).toBeVisible()
  await expect(page.getByRole('button', { name: 'Save' })).toBeDisabled()
  await field.fill('a‮b')
  await expect(page.getByText(/invisible formatting/)).toBeVisible()
  await field.fill('Home')
  await page.getByRole('button', { name: 'Save' }).click()
  await expect(page.getByText(/The profile list changed/)).toBeVisible()
  await expect(field).toHaveValue('Home')
})

test('a stale read keeps the list and says why', async ({ page, agent }) => {
  withEsim(agent, view({ freshness: { age_ms: 90000, stale: true, error: { kind: 'sim_reloading', message: 'x' } } }))
  await openSim(page)
  await expect(page.getByText(/reloading the SIM. Showing the last profile list read/)).toBeVisible()
  await expect(page.getByRole('list', { name: 'eSIM profiles' }).getByRole('listitem')).toHaveCount(3)
})

test('unsupported card and missing helper replace the list', async ({ page, agent }) => {
  withEsim(agent, view({ status: 'helper_missing', available: false, card: null, profiles: [],
    actions: { enabled: false, blocked_by: 'helper_missing' } }))
  await openSim(page)
  await expect(page.getByText('eSIM management unavailable')).toBeVisible()
})

for (const [name, viewport] of Object.entries(VIEWPORTS)) {
  for (const theme of ['light', 'dark'] as const) {
    test.describe(`layout ${name} ${theme}`, () => {
      test.use({ viewport, hasTouch: name !== 'desktop', isMobile: name !== 'desktop' })
      test('no horizontal overflow; tabs wrap whole', async ({ page, agent }) => {
        withEsim(agent)
        await openSim(page, { theme })
        await expect(page.getByRole('list', { name: 'eSIM profiles' })).toBeVisible()
        const over = await page.evaluate(() => {
          const vw = document.documentElement.clientWidth
          const out: string[] = []
          for (const el of document.querySelectorAll('main *')) {
            const r = el.getBoundingClientRect()
            if (r.width > 0 && r.right > vw + 1) out.push(`${el.tagName} right=${Math.round(r.right)}`)
          }
          for (const tab of document.querySelectorAll('[role="tab"]')) {
            const lh = parseFloat(getComputedStyle(tab).lineHeight) || 20
            if (tab.getBoundingClientRect().height > lh * 2 + 24) out.push(`tab wrapped: ${tab.textContent}`)
          }
          return out
        })
        expect(over).toEqual([])
        await page.screenshot({ path: test.info().outputPath(`sim-${name}-${theme}.png`), fullPage: true })
      })
    })
  }
}

// ── Phase 2: adding a profile ───────────────────────────────────────────────────

test('add by paste: masked code, preview with countdown and nickname, install, then Activate now', async ({ page, agent }) => {
  const box = stateful(agent)
  // Like the agent, the operation carries the client's operation_id.
  let id = ''
  agent.on('POST', '/api/esim/download', (req) => {
    id = (req.body as Json).operation_id as string
    box.current = view({ operation: download('connecting', { id, step: 'es9p_initiate_authentication' }), ...BUSY })
    return { data: box.current, status: 202 }
  })
  agent.on('POST', '/api/esim/download/confirm', () => {
    box.current = view({ operation: download('installing', { id, step: 'es10b_load_bound_profile_package', preview: PREVIEW }), ...BUSY })
    return { data: box.current }
  })
  await openSim(page)
  await page.getByRole('button', { name: 'Add eSIM' }).click()
  const code = page.getByLabel('Activation code')
  await code.fill(FAKE_CODE)
  await expect(code).toHaveAttribute('type', 'password')
  await expect(page.getByText('Server: smdp.example.com')).toBeVisible()
  await page.getByRole('button', { name: 'Download' }).click()
  await expect(page.getByRole('list', { name: 'Download progress' })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Activate Vodafone Fiji' })).toBeDisabled()

  box.current = view({
    operation: download('awaiting_confirmation', { id, step: 'preview', preview: PREVIEW, preview_expires_in_ms: 95000 }), ...BUSY,
  })
  await expect(page.getByText('Test Mobile')).toBeVisible({ timeout: 10_000 })
  await expect(page.getByText(/Answer within 1:3\d/)).toBeVisible()
  await page.getByLabel('Nickname (optional)').fill('Holiday')
  await page.getByRole('button', { name: 'Install' }).click()
  await expect(page.getByText(/can’t be cancelled/)).toBeVisible()

  const installed = profile({ handle: 'pd', display_name: 'Holiday', nickname: 'Holiday', name: 'Travel 7 days',
    provider: 'Test Mobile', iccid_masked: '…0004', state: 'disabled' })
  const base = view()
  box.current = view({
    profiles: [...(base.profiles as Json[]), installed], revision: 8,
    operation: download('succeeded', { id, target: 'pd', target_name: 'Holiday', step: 'es10b_load_bound_profile_package' }),
  })
  await expect(page.getByText(/Holiday is installed on the card, inactive/)).toBeVisible({ timeout: 10_000 })
  await page.getByRole('button', { name: 'Activate now' }).click()
  const dialog = page.getByRole('dialog')
  await expect(dialog).toContainText('Activate Holiday?')
  await expect(dialog.getByRole('button', { name: 'Cancel' })).toBeFocused()
  await dialog.getByRole('button', { name: 'Cancel' }).click()

  const started = agent.requests({ method: 'POST', path: '/api/esim/download' })
  expect(started).toHaveLength(1)
  expect(started[0].body).toMatchObject({ activation_code: FAKE_CODE })
  expect(started[0].body).not.toHaveProperty('confirmation_code')
  const confirmed = agent.requests({ method: 'POST', path: '/api/esim/download/confirm' })
  expect(confirmed[0].body).toEqual({ operation_id: id, install: true, nickname: 'Holiday' })
  expect(agent.requests({ method: 'POST', path: '/api/esim/enable' })).toHaveLength(0)
  // The code never lands in storage or the URL.
  const kept = await page.evaluate(() => JSON.stringify({ ...localStorage }) + JSON.stringify({ ...sessionStorage }) + location.href)
  expect(kept).not.toContain('TEST-MATCHING-ID')
})

test('a QR image is decoded in the browser into the masked field; other QR codes are refused', async ({ page, agent }) => {
  stateful(agent)
  await openSim(page)
  await page.getByRole('button', { name: 'Add eSIM' }).click()
  const file = page.getByTestId('qr-file')
  await file.setInputFiles(fixture('not-esim-qr.png'))
  await expect(page.getByText(/isn’t an eSIM activation code/)).toBeVisible()
  await expect(page.getByLabel('Activation code')).toHaveValue('')
  await file.setInputFiles(fixture('esim-test-qr.png'))
  await expect(page.getByText(/QR code read/)).toBeVisible()
  await expect(page.getByLabel('Activation code')).toHaveValue(FAKE_CODE)
  await expect(page.getByLabel('Activation code')).toHaveAttribute('type', 'password')
  await expect(page.getByText('Server: smdp.example.com')).toBeVisible()
  expect(agent.mutations()).toEqual([])
})

test('a $1 code asks for the confirmation code before anything is sent', async ({ page, agent }) => {
  stateful(agent)
  await openSim(page)
  await page.getByRole('button', { name: 'Add eSIM' }).click()
  await page.getByLabel('Activation code').fill(`${FAKE_CODE}$$1`)
  await page.getByRole('button', { name: 'Download' }).click()
  await expect(page.getByText(/needs the confirmation code/)).toBeVisible()
  expect(agent.requests({ method: 'POST', path: '/api/esim/download' })).toHaveLength(0)
})

test('cancelling while connecting postpones it and says the QR is likely reusable', async ({ page, agent }) => {
  const box = stateful(agent)
  agent.on('POST', '/api/esim/download', () => {
    box.current = view({ operation: download('connecting', { step: 'es10b_authenticate_server' }), ...BUSY })
    return { data: box.current, status: 202 }
  })
  agent.on('POST', '/api/esim/download/cancel', () => {
    box.current = view({ operation: download('cancelled', { message: 'cancelled', step: 'es9p_cancel_session' }) })
    return { data: box.current }
  })
  await openSim(page)
  await page.getByRole('button', { name: 'Add eSIM' }).click()
  await page.getByLabel('Activation code').fill(FAKE_CODE)
  await page.getByRole('button', { name: 'Download' }).click()
  await page.getByRole('button', { name: 'Cancel download' }).click()
  await expect(page.getByText(/likely still usable/)).toBeVisible()
  await page.getByRole('button', { name: 'Try again' }).click()
  await expect(page.getByLabel('Activation code')).toHaveValue('')
})

test('no mobile data: the refusal explains why and keeps the code for a retry', async ({ page, agent }) => {
  stateful(agent)
  agent.on('POST', '/api/esim/download', { status: 409, error: 'the router has no mobile data connection' })
  await openSim(page)
  await page.getByRole('button', { name: 'Add eSIM' }).click()
  await page.getByLabel('Activation code').fill(FAKE_CODE)
  await page.getByRole('button', { name: 'Download' }).click()
  await expect(page.getByText(/router’s own data/)).toBeVisible()
  await expect(page.getByLabel('Activation code')).toHaveValue(FAKE_CODE)
})

test('a failed install says the QR has likely been used up', async ({ page, agent }) => {
  stateful(agent, view({ operation: download('failed', { step: 'es10b_load_bound_profile_package',
    message: 'es10b_load_bound_profile_package: installFailedDueToIccidAlreadyExistsOnEuicc' }) }))
  await openSim(page)
  await expect(page.getByText(/likely been used up/)).toBeVisible()
})

// ── Phase 2: deleting a profile and sending receipts ─────────────────────────────

test('delete: inactive unlocked rows only, danger confirm with Cancel focused, one guarded request', async ({ page, agent }) => {
  const base = view()
  const locked = (base.profiles as Json[]).map((p) => (p.handle === 'pb' ? { ...p, locked: true } : p))
  const box = stateful(agent, view({ profiles: locked }))
  agent.on('POST', '/api/esim/delete', () => {
    box.current = view({ profiles: locked, ...BUSY, operation: { id: 'op-del', kind: 'delete', target: 'pc',
      target_name: null, previous: null, state: 'pending', resolved: false, message: null } })
    return { status: 202, data: box.current }
  })
  await openSim(page)
  await expect(page.getByRole('button', { name: 'Delete Amaysim' })).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Delete Vodafone Fiji' })).toHaveCount(0)
  await page.getByRole('button', { name: 'Delete Unnamed profile …0003' }).click()
  const dialog = page.getByRole('dialog')
  await expect(dialog).toContainText('can’t be undone')
  await expect(dialog).toContainText('single-use')
  await expect(dialog.getByRole('button', { name: 'Cancel' })).toBeFocused()
  await dialog.getByRole('button', { name: 'Cancel' }).click()
  expect(agent.mutations()).toEqual([])
  await page.getByRole('button', { name: 'Delete Unnamed profile …0003' }).click()
  await page.getByRole('dialog').getByRole('button', { name: 'Delete' }).click()
  await expect(page.getByText('Deleting the profile…')).toBeVisible()
  const sent = agent.requests({ method: 'POST', path: '/api/esim/delete' })
  expect(sent).toHaveLength(1)
  expect(sent[0].headers['x-confirm']).toBe('true')
  expect(sent[0].body).toMatchObject({ handle: 'pc', revision: 7 })
})

test('Send receipts runs on request and shows progress', async ({ page, agent }) => {
  withEsim(agent)
  agent.on('POST', '/api/esim/receipts/send', {
    status: 202, data: view({ receipts: { pending: 0, last_error: null, gave_up: false, sending: true } }),
  })
  await openSim(page)
  await page.getByRole('button', { name: 'Send receipts' }).click()
  await expect(page.getByText(/Sending receipts to the operators’ servers/)).toBeVisible()
  expect(agent.requests({ method: 'POST', path: '/api/esim/receipts/send' })).toHaveLength(1)
})

for (const [name, viewport] of Object.entries(VIEWPORTS)) {
  for (const theme of ['light', 'dark'] as const) {
    test.describe(`add flow layout ${name} ${theme}`, () => {
      test.use({ viewport, hasTouch: name !== 'desktop', isMobile: name !== 'desktop' })
      test('code step and preview fit without horizontal overflow', async ({ page, agent }) => {
        const box = stateful(agent)
        await openSim(page, { theme })
        await page.getByRole('button', { name: 'Add eSIM' }).click()
        await expect(page.getByLabel('Activation code')).toBeVisible()
        const overflow = () =>
          page.evaluate(() => {
            const vw = document.documentElement.clientWidth
            const out: string[] = []
            for (const el of document.querySelectorAll('main *')) {
              const r = el.getBoundingClientRect()
              if (r.width > 0 && r.right > vw + 1) out.push(`${el.tagName} right=${Math.round(r.right)}`)
            }
            for (const b of document.querySelectorAll('main button')) {
              const lh = parseFloat(getComputedStyle(b).lineHeight) || 20
              if (b.getBoundingClientRect().height > Math.max(48, lh * 2)) out.push(`button wrapped: ${b.textContent}`)
            }
            return out
          })
        expect(await overflow()).toEqual([])
        await page.screenshot({ path: test.info().outputPath(`sim-add-${name}-${theme}.png`), fullPage: true })
        box.current = view({
          operation: download('awaiting_confirmation', { step: 'preview', preview: PREVIEW, preview_expires_in_ms: 95000 }), ...BUSY,
        })
        agent.on('POST', '/api/esim/refresh', () => ({ data: box.current }))
        await page.getByRole('button', { name: 'Refresh' }).click()
        await expect(page.getByText('Test Mobile')).toBeVisible()
        expect(await overflow()).toEqual([])
        await page.screenshot({ path: test.info().outputPath(`sim-preview-${name}-${theme}.png`), fullPage: true })
      })
    })
  }
}
