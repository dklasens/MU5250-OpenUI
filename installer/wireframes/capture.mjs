import { chromium } from '@playwright/test'
import { mkdir } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
const destination = fileURLToPath(new URL('../../docs/images/installer-v2.5/', import.meta.url))
await mkdir(destination, { recursive: true })
const browser = await chromium.launch()
try {
  for (const [scene, filename] of [['connect', '01-connect.png'], ['checked', '02-checked.png'], ['unlock', '03-unlock.png'], ['complete', '04-complete.png'], ['recovery', '05-recovery.png']]) {
    const page = await browser.newPage({ viewport: { width: 1100, height: 900 }, deviceScaleFactor: 2, colorScheme: 'light' })
    await page.route('**/*', (route) => route.request().url().startsWith('http://127.0.0.1:1431/') ? route.continue() : route.abort())
    const errors = []
    page.on('pageerror', (error) => errors.push(error.message))
    await page.goto(`http://127.0.0.1:1431/?scene=${scene}`)
    await page.getByText('ZTE U60 Pro / MU5250', { exact: true }).waitFor()
    if (scene === 'unlock') {
      for (const label of ['Router admin password', 'Backup-key suffix', 'Dashboard password', 'Confirm dashboard password']) await page.getByLabel(label, { exact: true }).fill('Example only')
    }
    if (['checked', 'complete', 'unlock'].includes(scene)) {
      await page.getByRole('button', { name: 'Check device', exact: true }).click()
      const next = page.getByRole('button', { name: scene === 'unlock' ? '3. Install' : '3. Update', exact: true })
      await next.waitFor()
      if (scene !== 'checked') await next.click()
    }
    if (['complete', 'unlock'].includes(scene)) await page.getByRole('dialog').waitFor()
    await page.addStyleTag({ content: `
      html { filter: grayscale(1); } body { background: #f4f4f4; }
      .app-shell { padding-top: 72px; } .panel, .modal { box-shadow: none; border-color: #9d9d9d; }
      .brand-mark { background: #4d4d4d; box-shadow: none; } .primary-button { box-shadow: none; }
      *, *::before, *::after { animation: none !important; transition: none !important; }
      .wireframe-label { position: fixed; z-index: 100; top: 18px; left: 24px; right: 24px; padding: 12px; background: white; border: 1px solid #999; font: 12px system-ui; letter-spacing: .04em; }
    ` })
    await page.evaluate(() => { const label = document.createElement('div'); label.className = 'wireframe-label'; label.textContent = 'OPEN U60 PRO · v2.5 INSTALLER WIREFRAME · SAMPLE DATA · NO MODEM CONNECTED'; document.body.appendChild(label) })
    if (errors.length) throw new Error(errors.join('\n'))
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth)
    if (overflow) throw new Error(`${scene} has horizontal overflow`)
    await page.screenshot({ path: `${destination}${filename}`, fullPage: true })
    console.log(`Captured ${filename}`)
    await page.close()
  }
} finally { await browser.close() }
