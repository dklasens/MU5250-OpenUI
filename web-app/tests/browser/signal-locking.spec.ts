import type { Page } from '@playwright/test'
import { test, expect } from './support/harness'
import { advanceHeartbeat, freezeClock, openApp, VIEWPORTS } from './support/app'
import * as fx from './support/fixtures'

/**
 * Signal > Mode & Locking: band-lock drafts across heartbeats (R03), connection confirmations
 * (R11), network-mode radiogroup (U03), capability read states (R07), stacked serving cells (U07).
 * Every mutation is intercepted by the synthetic agent; nothing reaches a device.
 */

const LTE_SUPPORTED = [1, 2, 3, 4, 5, 7, 8, 18, 19, 20, 26, 28, 29, 32, 34, 38, 39, 40, 41, 42, 43, 48, 66, 71]

const lteBandsOf = (mask: string) => {
  const n = BigInt(mask)
  return Array.from({ length: 64 }, (_, i) => i + 1).filter((b) => (n >> BigInt(b - 1)) & 1n)
}

const pressedBands = async (page: Page, group: 'NR bands' | 'LTE bands') => {
  const texts = await page.getByRole('group', { name: group }).getByRole('button', { pressed: true }).allTextContents()
  return texts.map((t) => Number(t.replace(/\D/g, '')))
}

async function openLocking(page: Page, opts: { clock?: boolean } = {}) {
  await openApp(page, { group: 'signal', tab: 'Mode & Locking', clock: opts.clock })
  await expect(page.getByRole('group', { name: 'NR bands' })).toBeVisible()
  if (opts.clock) await freezeClock(page)
}

test.describe('band lock drafts', () => {
  test('LTE and NR edits survive three equivalent heartbeats', async ({ page, agent }) => {
    let beat = 0
    agent.on('GET', '/api/dashboard', () => {
      // Equivalent observations in different encodings/orders: new arrays every time, same sets.
      beat++
      const flip = beat % 2 === 0
      return { data: fx.dashboard({ network: 'SA', signal: { lte_band_lock: flip ? '0x84' : '132', nr5g_sa_band_lock: flip ? '78,41' : '41,78' } }) }
    })
    await openLocking(page, { clock: true })
    await expect(page.getByText('B3, B8')).toBeVisible()
    expect(await pressedBands(page, 'LTE bands')).toEqual([3, 8])
    expect(await pressedBands(page, 'NR bands')).toEqual([41, 78])

    await page.getByRole('group', { name: 'LTE bands' }).getByRole('button', { name: 'B7', exact: true }).click()
    await page.getByRole('group', { name: 'NR bands' }).getByRole('button', { name: 'n1', exact: true }).click()
    await page.getByRole('group', { name: 'NR bands' }).getByRole('button', { name: 'n78', exact: true }).click()

    for (let i = 0; i < 3; i++) await advanceHeartbeat(page, agent)
    expect(beat).toBeGreaterThanOrEqual(4)
    expect(await pressedBands(page, 'LTE bands')).toEqual([3, 7, 8])
    expect(await pressedBands(page, 'NR bands')).toEqual([1, 41])
    // No conflict banner: the device never changed the setting.
    await expect(page.getByText(/while you were editing/)).toHaveCount(0)
    expect(agent.mutations()).toEqual([])
  })

  test('an external change while editing keeps the draft and offers the modem value; Cancel restores it', async ({ page, agent }) => {
    let nr = '41,78'
    agent.on('GET', '/api/dashboard', () => ({ data: fx.dashboard({ network: 'SA', signal: { nr5g_sa_band_lock: nr } }) }))
    await openLocking(page, { clock: true })
    await page.getByRole('group', { name: 'NR bands' }).getByRole('button', { name: 'n1', exact: true }).click()

    nr = '3,5'
    await advanceHeartbeat(page, agent)
    await expect(page.getByText(/changed to “n3, n5” while you were editing/)).toBeVisible()
    expect(await pressedBands(page, 'NR bands')).toEqual([1, 41, 78])
    await expect(page.getByText('Current lock:').filter({ hasText: 'n3, n5' })).toBeVisible()

    await page.locator('section', { has: page.getByRole('heading', { name: 'NR 5G band lock' }) }).getByRole('button', { name: 'Cancel' }).click()
    expect(await pressedBands(page, 'NR bands')).toEqual([3, 5])
    await expect(page.getByText(/while you were editing/)).toHaveCount(0)
    expect(agent.mutations()).toEqual([])
  })

  test('the SA control uses SA state only: known-empty SA with an NSA lock shows automatic, no bands', async ({ page, agent }) => {
    agent.on('GET', '/api/dashboard', {
      data: fx.dashboard({ network: 'SA', signal: { nr5g_sa_band_lock: '0', nr5g_nsa_band_lock: '78,41' } }),
    })
    await openLocking(page)
    await expect(page.getByText('Automatic (no band restriction)').first()).toBeVisible()
    expect(await pressedBands(page, 'NR bands')).toEqual([])
  })

  test('missing lock data is shown as unknown, not as automatic', async ({ page, agent }) => {
    agent.on('GET', '/api/dashboard', {
      data: fx.dashboard({ network: 'SA', signal: { nr5g_sa_band_lock: null, lte_band_lock: null } }),
    })
    await openLocking(page)
    await expect(page.getByText('Unknown (the modem did not report it)')).toHaveCount(2)
    expect(await pressedBands(page, 'NR bands')).toEqual([])
  })

  test('a lock covering every supported band reads "All bands"', async ({ page, agent }) => {
    const lteAll = LTE_SUPPORTED.reduce((m, b) => m | (1n << BigInt(b - 1)), 0n).toString()
    agent.on('GET', '/api/dashboard', { data: fx.dashboard({ network: 'SA', signal: { lte_band_lock: lteAll } }) })
    await openLocking(page)
    // NR: the fixture's SA lock lists every supported band. LTE: built from the capability list above.
    await expect(page.getByText('All bands')).toHaveCount(2)
  })

  test('Cancel in the confirmation sends nothing; Confirm sends one POST equal to the visible selection', async ({ page, agent }) => {
    let nr = '41,78'
    agent.on('GET', '/api/dashboard', () => ({ data: fx.dashboard({ network: 'SA', signal: { nr5g_sa_band_lock: nr } }) }))
    agent.on('POST', '/api/cell/band/nr', (req) => {
      nr = String((req.body as { nr5g_band: string }).nr5g_band)
      return { data: {} }
    })
    await openLocking(page, { clock: true })

    const nrGroup = page.getByRole('group', { name: 'NR bands' })
    await nrGroup.getByRole('button', { name: 'n1', exact: true }).click()
    await nrGroup.getByRole('button', { name: 'n78', exact: true }).click()
    const visible = await pressedBands(page, 'NR bands')
    expect(visible).toEqual([1, 41])

    const apply = page.getByRole('button', { name: 'Lock 2 bands' }).first()
    await apply.click()
    const dialog = page.getByRole('dialog', { name: 'Lock NR bands?' })
    await expect(dialog).toBeVisible()
    await expect(dialog).toContainText('n1, n41')
    await expect(dialog).toContainText('Internet')
    await expect(dialog).toContainText('Reset bands to automatic')
    await dialog.getByRole('button', { name: 'Cancel' }).click()
    await expect(dialog).toBeHidden()
    expect(agent.mutations()).toEqual([])
    expect(await pressedBands(page, 'NR bands')).toEqual([1, 41]) // the draft survives Cancel

    await apply.click()
    await page.getByRole('dialog', { name: 'Lock NR bands?' }).getByRole('button', { name: 'Apply lock' }).click()
    await expect.poll(() => agent.mutations().length).toBe(1)
    const [m] = agent.mutations()
    expect(`${m.method} ${m.path}`).toBe('POST /api/cell/band/nr')
    expect(m.body).toEqual({ nr5g_type: 'SA', nr5g_band: visible.join(',') })
    // Fresh read-back re-baselines: the device now reports the lock, the draft is not dirty.
    await expect(page.getByText('Current lock:').filter({ hasText: 'n1, n41' })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Lock 2 bands' }).first()).toBeDisabled()
    expect(agent.mutations()).toHaveLength(1)
  })

  test('LTE Confirm sends one POST whose mask equals the visible selection', async ({ page, agent }) => {
    agent.on('GET', '/api/dashboard', { data: fx.dashboard({ network: 'SA', signal: { lte_band_lock: '132' } }) })
    agent.on('POST', '/api/cell/band/lte', { data: {} })
    await openLocking(page)
    await page.getByRole('group', { name: 'LTE bands' }).getByRole('button', { name: 'B7', exact: true }).click()
    const visible = await pressedBands(page, 'LTE bands')
    expect(visible).toEqual([3, 7, 8])

    await page.getByRole('button', { name: 'Lock 3 bands' }).click()
    const dialog = page.getByRole('dialog', { name: 'Lock LTE bands?' })
    await expect(dialog).toContainText('B3, B7, B8')
    await dialog.getByRole('button', { name: 'Apply lock' }).click()
    await expect.poll(() => agent.mutations().length).toBe(1)
    const [m] = agent.mutations()
    expect(m.path).toBe('/api/cell/band/lte')
    const body = m.body as { lte_band_mask: string }
    expect(lteBandsOf(body.lte_band_mask)).toEqual(visible)
    expect(body).toEqual({ is_lte_band: '1', lte_band_mask: body.lte_band_mask, is_gw_band: '0', gw_band_mask: '0' })
  })

  test('a failed apply keeps the selection and unlocks the controls', async ({ page, agent }) => {
    agent.on('GET', '/api/dashboard', { data: fx.dashboard({ network: 'SA', signal: { nr5g_sa_band_lock: '41,78' } }) })
    agent.fail('POST', '/api/cell/band/nr', { status: 500, error: 'synthetic apply failure' })
    await openLocking(page)
    await page.getByRole('group', { name: 'NR bands' }).getByRole('button', { name: 'n1', exact: true }).click()
    await page.getByRole('button', { name: 'Lock 3 bands' }).click()
    await page.getByRole('dialog').getByRole('button', { name: 'Apply lock' }).click()
    await expect(page.getByText('synthetic apply failure')).toBeVisible()
    expect(agent.mutations()).toHaveLength(1)
    expect(await pressedBands(page, 'NR bands')).toEqual([1, 41, 78])
    await expect(page.getByRole('button', { name: 'Lock 3 bands' })).toBeEnabled()
  })

  test('Reset bands asks first: Cancel sends nothing, Confirm sends one reset', async ({ page, agent }) => {
    agent.on('POST', '/api/cell/band/reset', { data: {} })
    await openLocking(page)
    await page.getByRole('button', { name: 'Reset bands to automatic' }).click()
    const dialog = page.getByRole('dialog', { name: 'Reset band locks to automatic?' })
    await expect(dialog).toContainText('LTE and NR (5G SA) band locks')
    await dialog.getByRole('button', { name: 'Cancel' }).click()
    expect(agent.mutations()).toEqual([])
    await page.getByRole('button', { name: 'Reset bands to automatic' }).click()
    await page.getByRole('dialog').getByRole('button', { name: 'Reset bands' }).click()
    await expect.poll(() => agent.mutations().length).toBe(1)
    expect(agent.mutations()[0].path).toBe('/api/cell/band/reset')
  })
})

test.describe('network mode', () => {
  test('arrow keys change the draft only; nothing is sent until Apply and Confirm', async ({ page, agent }) => {
    agent.on('PUT', '/api/modem/network-mode', { data: {} })
    await openLocking(page)
    const group = page.getByRole('radiogroup', { name: 'Network mode' })
    await expect(group.getByRole('radio', { name: '5G SA' })).toBeChecked()
    const apply = page.getByRole('button', { name: 'Apply', exact: true })
    await expect(apply).toBeDisabled()

    await group.getByRole('radio', { name: '5G SA' }).focus()
    await page.keyboard.press('ArrowRight')
    await page.keyboard.press('ArrowRight')
    await expect(group.getByRole('radio', { name: '4G only' })).toBeChecked()
    await page.keyboard.press('ArrowLeft')
    await page.keyboard.press('ArrowRight')
    await page.waitForTimeout(150)
    expect(agent.mutations()).toEqual([])
    await expect(apply).toBeEnabled()

    await apply.click()
    const dialog = page.getByRole('dialog', { name: 'Change network mode?' })
    await expect(dialog).toContainText('4G only')
    await expect(dialog).toContainText('Internet')
    await dialog.getByRole('button', { name: 'Cancel' }).click()
    expect(agent.mutations()).toEqual([])

    await apply.click()
    await page.getByRole('dialog').getByRole('button', { name: 'Change mode' }).click()
    await expect.poll(() => agent.mutations().length).toBe(1)
    const [m] = agent.mutations()
    expect(`${m.method} ${m.path}`).toBe('PUT /api/modem/network-mode')
    expect(m.body).toEqual({ net_select: 'Only_LTE' })
  })
})

test.describe('capabilities (R07)', () => {
  test('a failed read shows Retry with controls unavailable, and recovers in place', async ({ page, agent }) => {
    agent.fail('GET', '/api/modem/capabilities', { times: 1, status: 503, error: 'synthetic capability outage' })
    await openApp(page, { group: 'signal', tab: 'Mode & Locking' })
    const alert = page.getByRole('alert').filter({ hasText: 'synthetic capability outage' })
    await expect(alert).toBeVisible()
    await expect(page.getByRole('radiogroup', { name: 'Network mode' })).toHaveCount(0)
    await expect(page.getByRole('group', { name: 'NR bands' })).toHaveCount(0)
    expect(agent.requests({ method: 'GET', path: '/api/modem/capabilities' })).toHaveLength(1)

    await alert.getByRole('button', { name: 'Retry' }).click()
    await expect(page.getByRole('group', { name: 'NR bands' })).toBeVisible()
    await expect(page.getByRole('radiogroup', { name: 'Network mode' })).toBeVisible()
    await expect(alert).toHaveCount(0)
    expect(agent.requests({ method: 'GET', path: '/api/modem/capabilities' })).toHaveLength(2)
  })

  test('no supported bands reported means no invented band buttons', async ({ page, agent }) => {
    agent.on('GET', '/api/modem/capabilities', { data: fx.modemCapabilities({ nr_sa_bands: [], lte_bands: [] }) })
    agent.on('GET', '/api/dashboard', {
      data: fx.dashboard({ network: 'SA', signal: { nr5g_sa_band_lock: '0', lte_band_lock: '0' } }),
    })
    await openApp(page, { group: 'signal', tab: 'Mode & Locking' })
    await expect(page.getByText(/did not report any supported NR bands/)).toBeVisible()
    await expect(page.getByRole('group', { name: 'NR bands' })).toHaveCount(0)
  })
})

test.describe('serving cells', () => {
  test('mobile 375px: stacked rows show every field, no horizontal overflow, and Lock confirms first', async ({ page, agent }) => {
    await page.setViewportSize(VIEWPORTS.phone375)
    agent.on('GET', '/api/dashboard', { data: fx.dashboard({ network: 'NSA' }) })
    agent.on('POST', '/api/cell/lock/lte', { data: {} })
    await openLocking(page)

    const list = page.getByRole('list', { name: 'Serving cells' })
    await expect(list).toBeVisible()
    const rows = list.getByRole('listitem')
    const count = await rows.count()
    expect(count).toBeGreaterThanOrEqual(3)
    for (let i = 0; i < count; i++) {
      const row = rows.nth(i)
      for (const term of ['PCI', 'BW', 'RSRP', 'SINR']) await expect(row.getByText(term, { exact: true })).toBeVisible()
      await expect(row.getByText(/^(NR-ARFCN|EARFCN)$/)).toBeVisible()
      await expect(row.getByRole('button', { name: /^Lock (LTE|NR)/ })).toBeVisible()
    }
    // The LTE primary cell: role, band, PCI and EARFCN from the fixture lteca.
    const lte = rows.filter({ hasText: 'B8' }).first()
    await expect(lte).toContainText('PCC')
    await expect(lte).toContainText('312')
    await expect(lte).toContainText('3650')

    const overflow = await page.evaluate(() => ({
      doc: document.documentElement.scrollWidth - window.innerWidth,
      main: (() => { const m = document.querySelector('main'); return m ? m.scrollWidth - m.clientWidth : 0 })(),
    }))
    expect(overflow.doc).toBeLessThanOrEqual(0)
    expect(overflow.main).toBeLessThanOrEqual(1)
    await expect(page.getByRole('table', { name: /serving/i })).toHaveCount(0) // table is hidden below sm

    // The Lock action names its tuple and never fires without the confirmation.
    const lockButton = lte.getByRole('button', { name: 'Lock LTE cell PCI 312, EARFCN 3650' })
    await lockButton.click()
    const dialog = page.getByRole('dialog', { name: 'Lock to this cell?' })
    await expect(dialog).toContainText('312')
    await expect(dialog).toContainText('3650')
    await dialog.getByRole('button', { name: 'Cancel' }).click()
    expect(agent.mutations()).toEqual([])

    await lockButton.click()
    await page.getByRole('dialog').getByRole('button', { name: 'Lock cell' }).click()
    await expect.poll(() => agent.mutations().length).toBe(1)
    const [m] = agent.mutations()
    expect(`${m.method} ${m.path}`).toBe('POST /api/cell/lock/lte')
    expect(m.body).toEqual({ lock_lte_pci: '312', lock_lte_earfcn: '3650' })
  })

  test('manual NR cell lock and cell reset confirm before sending', async ({ page, agent }) => {
    agent.on('POST', '/api/cell/lock/nr', { data: {} })
    agent.on('POST', '/api/cell/lock/reset', { data: {} })
    await openLocking(page)
    const card = page.locator('section', { has: page.getByRole('heading', { name: 'NR cell lock' }) })
    await card.getByLabel('PCI').fill('745')
    await card.getByLabel('NR-ARFCN').fill('643392')
    await card.getByLabel('Band').fill('78')
    await card.getByRole('button', { name: 'Lock', exact: true }).click()
    const dialog = page.getByRole('dialog', { name: 'Lock to this cell?' })
    await expect(dialog).toContainText('n78')
    await dialog.getByRole('button', { name: 'Cancel' }).click()
    expect(agent.mutations()).toEqual([])
    await card.getByRole('button', { name: 'Lock', exact: true }).click()
    await page.getByRole('dialog').getByRole('button', { name: 'Lock cell' }).click()
    await expect.poll(() => agent.mutations().length).toBe(1)
    expect(agent.mutations()[0].body).toEqual({ lock_nr_pci: '745', lock_nr_earfcn: '643392', lock_nr_cell_band: '78' })

    await page.getByRole('button', { name: 'Reset cell locks' }).click()
    await page.getByRole('dialog', { name: 'Reset cell locks to automatic?' }).getByRole('button', { name: 'Reset cells' }).click()
    await expect.poll(() => agent.mutations().length).toBe(2)
    expect(agent.mutations()[1].path).toBe('/api/cell/lock/reset')
  })
})


test.describe('cell lock state', () => {
  const card = (page: Page, rat: 'NR' | 'LTE') =>
    page.locator('section', { has: page.getByRole('heading', { name: `${rat} cell lock`, exact: true }) })

  test('a lock on the serving cell is shown with its cell and "on the locked cell"', async ({ page, agent }) => {
    // SA fixture serves NR PCI 745, NR-ARFCN 643392, n78. Serving values arrive as strings here.
    agent.on('GET', '/api/dashboard', {
      data: fx.dashboard({ network: 'SA', signal: { lock_nr_cell: '745,643392,78', lock_lte_cell: '0,0', nr5g_pci: '745', nr5g_action_channel: '643392' } }),
    })
    await openLocking(page)
    const nr = card(page, 'NR')
    await expect(nr.getByText('Current lock:')).toContainText('PCI 745, NR-ARFCN 643392, band n78')
    await expect(nr).toContainText('The modem is on the locked cell')
    await expect(nr.getByRole('status')).toHaveCount(0)
    await expect(card(page, 'LTE').getByText('Current lock:')).toContainText('None (automatic cell selection)')
    await expect(page.getByText('NR cell lock (raw):')).toContainText('745,643392,78')
  })

  test('a lock on another cell warns plainly; its Reset confirms, and the next read shows no lock', async ({ page, agent }) => {
    let locks = { lock_nr_cell: '431,423410,1', lock_lte_cell: '312,3650' }
    agent.on('GET', '/api/dashboard', () => ({ data: fx.dashboard({ network: 'SA', signal: locks }) }))
    agent.on('POST', '/api/cell/lock/reset', () => {
      locks = { lock_nr_cell: '0,0,0', lock_lte_cell: '0,0' } // the device's reset spelling
      return { data: {} }
    })
    await openLocking(page)

    const nr = card(page, 'NR')
    await expect(nr.getByText('Current lock:')).toContainText('PCI 431, NR-ARFCN 423410, band n1')
    const warning = nr.getByRole('status').filter({ hasText: 'not on the locked cell' })
    await expect(warning).toContainText('serving NR cell: PCI 745, NR-ARFCN 643392, band n78')
    await expect(warning).toContainText('usually means no mobile service')
    // SA: no LTE carrier is serving, so an LTE lock is not in use either.
    const lte = card(page, 'LTE')
    await expect(lte.getByText('Current lock:')).toContainText('PCI 312, EARFCN 3650')
    await expect(lte.getByRole('status')).toContainText('reports no serving LTE cell')

    await warning.getByRole('button', { name: 'Reset cell locks' }).click()
    const dialog = page.getByRole('dialog', { name: 'Reset cell locks to automatic?' })
    await dialog.getByRole('button', { name: 'Cancel' }).click()
    expect(agent.mutations()).toEqual([])
    // Nothing optimistic: the lock is still shown until the device says otherwise.
    await expect(nr.getByText('Current lock:')).toContainText('PCI 431')

    await warning.getByRole('button', { name: 'Reset cell locks' }).click()
    await page.getByRole('dialog').getByRole('button', { name: 'Reset cells' }).click()
    await expect.poll(() => agent.mutations().length).toBe(1)
    expect(agent.mutations()[0].path).toBe('/api/cell/lock/reset')
    await expect(nr.getByText('Current lock:')).toContainText('None (automatic cell selection)')
    await expect(lte.getByText('Current lock:')).toContainText('None (automatic cell selection)')
    await expect(page.getByText(/not on the locked cell/)).toHaveCount(0)
  })

  test('missing or malformed lock fields read unknown, not unlocked', async ({ page, agent }) => {
    agent.on('GET', '/api/dashboard', {
      data: fx.dashboard({ network: 'SA', signal: { lock_nr_cell: null, lock_lte_cell: '312,abc' } }),
    })
    await openLocking(page)
    for (const rat of ['NR', 'LTE'] as const) {
      await expect(card(page, rat).getByText('Current lock:')).toContainText('Unknown (the modem did not report it)')
      await expect(card(page, rat)).not.toContainText('locked cell')
    }
    await expect(page.getByText('NR cell lock (raw):')).toContainText('(not reported)')
  })
})
