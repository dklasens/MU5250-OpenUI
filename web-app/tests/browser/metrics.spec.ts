// System → Metrics: CPU and memory come from the heartbeat; thermal, battery
// detail and the fuel-gauge read each own their state (error + Retry, stale).
import type { Page } from '@playwright/test'
import { test, expect, type MockAgent } from './support/harness'
import { openApp, advanceHeartbeat, freezeClock, settleRender } from './support/app'
import * as fx from './support/fixtures'

const THERMAL = '/api/device/thermal/all'
const DETAIL = '/api/device/battery/detail'
const INFO = '/api/device/battery-info'

const card = (page: Page, title: string) =>
  page.locator('section', { has: page.getByRole('heading', { name: title, exact: true }) })

const count = (agent: MockAgent, path: string) =>
  agent.requests({ method: 'GET', path }).length

test('CPU and memory render from the heartbeat, which runs fast only while Metrics shows', async ({ page, agent }) => {
  agent.on('GET', '/api/dashboard', {
    data: fx.dashboard({ cpu: { overall: 42.5, cores: [50, 40, 41, 39] }, memory: { usage_pct: 37.4 } }),
  })
  await openApp(page, { group: 'system', tab: 'Metrics', clock: true })

  const cpu = card(page, 'CPU usage')
  await expect(cpu.getByText('42.5%')).toBeVisible()
  await expect(cpu.getByText('Core 3')).toBeVisible()
  await expect(card(page, 'Memory').getByText('(37%)')).toBeVisible()

  // Fast cadence on Metrics: one more heartbeat after 3 s.
  await freezeClock(page)
  await advanceHeartbeat(page, agent, 3000)
  await page.waitForTimeout(250) // real time: the reply lands and re-arms the 3 s timer

  // Leaving Metrics drops back to the idle cadence once the armed read has run.
  await page.getByRole('tab', { name: 'Tools', exact: true }).click()
  await settleRender(page)
  await advanceHeartbeat(page, agent, 3000)
  await page.waitForTimeout(250)
  const idle = count(agent, '/api/dashboard')
  await page.clock.runFor(11_000)
  await page.waitForTimeout(250) // real time: let a (wrongly) fired request reach the agent
  expect(count(agent, '/api/dashboard')).toBe(idle)
  await advanceHeartbeat(page, agent, 4000)

  expect(count(agent, '/api/cpu')).toBe(0)
  expect(count(agent, '/api/memory')).toBe(0)
})

test('a failing source shows its reason and Retry while the others render', async ({ page, agent }) => {
  agent.fail('GET', THERMAL, { status: 503, error: 'thermal zones unreadable' })
  await openApp(page, { group: 'system', tab: 'Metrics', clock: true })

  const thermal = card(page, 'Temperatures')
  await expect(thermal.getByRole('alert')).toContainText('Could not read the temperatures: thermal zones unreadable')
  await freezeClock(page) // only Retry reads thermal from here on
  // No fabricated readings in the failed card; the other sources still render.
  await expect(thermal.getByText('°C')).toHaveCount(0)
  await expect(card(page, 'Battery').getByText('78%')).toBeVisible()
  await expect(card(page, 'Battery').getByText('Hardware')).toBeVisible()
  await expect(card(page, 'CPU usage').getByText('23.4%')).toBeVisible()
  await expect(card(page, 'Battery health').getByText('Good')).toBeVisible()

  agent.on('GET', THERMAL, { data: fx.thermalAll() })
  const before = count(agent, THERMAL)
  await thermal.getByRole('button', { name: 'Retry' }).click()
  await expect(thermal.getByText('55.0°C')).toBeVisible()
  await expect(thermal.getByRole('alert')).toHaveCount(0)
  expect(count(agent, THERMAL)).toBe(before + 1)
})

test('a failed heartbeat explains CPU and memory without blanking the other cards', async ({ page, agent }) => {
  agent.fail('GET', '/api/dashboard', { status: 503, error: 'agent busy' })
  await openApp(page, { group: 'system', tab: 'Metrics' })

  const cpu = card(page, 'CPU usage')
  await expect(cpu.getByRole('alert')).toContainText('Could not read the CPU usage: agent busy')
  await expect(cpu.getByRole('button', { name: 'Retry' })).toBeVisible()
  await expect(card(page, 'Memory')).toContainText('Could not read the memory usage: agent busy')
  await expect(card(page, 'Temperatures').getByText('55.0°C')).toBeVisible()
  await expect(card(page, 'Battery').getByText('78%')).toBeVisible()
})

test('a later failure keeps the last good readings marked stale', async ({ page, agent }) => {
  await openApp(page, { group: 'system', tab: 'Metrics', clock: true })
  const battery = card(page, 'Battery')
  await expect(battery.getByText('78%')).toBeVisible()
  await expect(battery.getByText('Hardware')).toBeVisible()
  await freezeClock(page)

  agent.fail('GET', DETAIL, { status: 503, error: 'fuel gauge timeout' })
  agent.fail('GET', INFO, { status: 503, error: 'ubus timeout' })
  // Battery detail polls every 10 s, the fuel-gauge read every 30 s.
  await advanceHeartbeat(page, agent, 10_000, DETAIL)
  await expect(battery.getByText('Showing the last battery reading; refresh failed: fuel gauge timeout')).toBeVisible()
  await expect(card(page, 'Battery health').getByText('Showing the last battery health reading; refresh failed: fuel gauge timeout')).toBeVisible()
  await expect(battery.getByText('78%')).toBeVisible()
  await expect(card(page, 'Battery health').getByText('Good')).toBeVisible()

  // Two steps: one 20 s jump of the fake clock would outrun the client's 15 s
  // request timeout and abort an in-flight heartbeat read.
  await advanceHeartbeat(page, agent, 10_000, DETAIL)
  await advanceHeartbeat(page, agent, 10_000, INFO)
  await expect(battery.getByText('Showing the last fuel gauge reading; refresh failed: ubus timeout')).toBeVisible()
  await expect(battery.getByText('Hardware')).toBeVisible()
  // Unaffected sources stay current.
  await expect(card(page, 'Temperatures').locator('[data-kind="stale"]')).toHaveCount(0)
  await expect(card(page, 'CPU usage').locator('[data-kind="stale"]')).toHaveCount(0)

  agent.on('GET', DETAIL, { data: fx.batteryDetail({ capacity: 80 }) })
  await battery.locator('[data-kind="stale"]').filter({ hasText: 'battery reading' }).getByRole('button', { name: 'Retry' }).click()
  await expect(battery.getByText('80%')).toBeVisible()
  await expect(battery.getByText('Showing the last battery reading')).toHaveCount(0)
})
