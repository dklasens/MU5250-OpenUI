# Browser regression harness

Playwright (Chromium only) drives the real React app served by the Vite dev
server, against a **synthetic agent** implemented with request interception.
It exists for behaviour the Node suite (`tools/test-*.cjs`) cannot see: polling
races, focus handling, layout at real viewport sizes. Keep pure mappings and
classifiers in the Node tests.

## Run

```sh
cd web-app
npx playwright install chromium     # once; downloads the browser into the Playwright cache
npm run test:browser                # whole suite
PW_PORT=5210 npm run test:browser   # pick the dev-server port (default 5199)
npx playwright test smoke --headed  # a single spec, visible browser
npx playwright show-trace test-results/<test>/trace.zip
```

- `@playwright/test` is pinned to `1.63.0`, which uses Chromium revision 1243.
  CI installs it with `npx playwright install --with-deps chromium`.
- Playwright starts `vite` itself (dev server, no build) on `127.0.0.1:$PW_PORT`
  with `--strictPort`. `reuseExistingServer` is off: if the port is taken the
  run fails instead of testing someone else's server. Parallel runs (several
  agents, several worktrees) must use different `PW_PORT` values.
- Failure artefacts (screenshot, trace) go to `test-results/`, or to
  `test-results/port-<PW_PORT>/` when `PW_PORT` is set; the CI HTML report
  goes to `playwright-report/`. Both are gitignored. All of it is synthetic.
- The web server uses `tests/browser/support/vite.config.ts`, which extends the
  app config, strips React `StrictMode` from `src/main.tsx` (production does not
  double-run effects; dev would send every first poll twice and discard the
  first answer) and keeps a per-port dependency cache. `PW_STRICT_MODE=1` keeps
  StrictMode.

## Safety guarantees

Nothing in this suite can reach the router.

1. The page is served from `127.0.0.1:$PW_PORT`, so the app's `API_BASE` is
   `http://127.0.0.1:9090`. Every request goes through one `context.route`:
   - dev-server origin: passes through;
   - `http://127.0.0.1:9090`: answered by the synthetic agent, never forwarded;
   - anything else (including `192.168.0.1`, fonts, analytics, other ports):
     aborted and recorded in `agent.blocked()`.
2. An agent request with no registered handler is answered `501`, recorded in
   `agent.unhandled()`, and **fails the test** in teardown. So does any entry in
   `agent.blocked()`. Both checks run automatically (the `agent` fixture is
   `auto`); a test that deliberately provokes a blocked request calls
   `agent.acknowledgeBlocked()`.
3. WebSockets are routed too; only the dev server's (Vite HMR) is allowed.
4. Service workers are blocked (`serviceWorkers: 'block'`). Each test gets a fresh
   browser context: no profile, cookies or storage are reused. The session is a
   fake token (`synthetic-test-token`) seeded into `sessionStorage` once per
   tab, so a logout/401 test is not silently re-authenticated by a reload.
   `test.use({ authenticated: false })` starts at the login screen.
5. Fixtures are synthetic: placeholder identifiers, documentation address ranges
   (`192.0.2.0/24`, `2001:db8::/32`), `02:00:5E:...` MACs, invented SMS text.
   Never paste real IMEI/ICCID/IMSI/MSISDN/MAC/SMS content or SSIDs.

## Writing a spec

```ts
import { test, expect } from './support/harness'
import { openApp, VIEWPORTS } from './support/app'
import * as fx from './support/fixtures'

test('Cancel sends nothing', async ({ page, agent }) => {
  agent.on('GET', '/api/dashboard', { data: fx.dashboard({ network: 'NSA' }) })
  await openApp(page, { group: 'system', tab: 'Device', theme: 'dark' })
  // ... interact ...
  expect(agent.mutations()).toEqual([])
})
```

Default replies exist for every GET the app makes (SA network, healthy device),
plus the read-only `POST /api/sms/list` and `POST /api/auth/login`. There are no
defaults for other non-GET methods: register what a test needs with `on`/`once`.

### `support/harness.ts`

| API | Meaning |
|---|---|
| `test`, `expect` | `test` is Playwright's `test` extended with `agent` (and the `authenticated` option). Importing it is what installs the interception. |
| `agent.on(method, path, reply)` | Replace the handler; persists. `reply` is `{data, status?}`, `{error, status?}` or `(req) => Reply \| Promise<Reply>`. Also clears an open-ended `fail()`. |
| `agent.once(method, path, reply)` | Next matching request only, then the `on` handler resumes. |
| `agent.defer(method, path)` | Hold the NEXT matching request. Returns `Deferred { waitForRequest(), resolve(data), reject(error, status?), abort() }`. Unsettled holds are aborted at teardown. |
| `agent.fail(method, path, {status?, error?, times?})` | Answer with the error envelope (default 500). Without `times`, until `on()` replaces it. |
| `agent.requests({method?, path?})` | Every request seen, in order, with `{method, path, query, body, headers}` (header names lower-case). |
| `agent.mutations()` | Non-GET requests except `/api/auth/login` and the read-only `POST /api/sms/list`. |
| `agent.unhandled()` | Requests with no registered reply. Must be empty at teardown. |
| `agent.blocked()` / `acknowledgeBlocked()` / `clearRequests()` | Extras: aborted external requests; forget them; reset the request log. |

Precedence per request: `defer` > `once`/`fail` (registration order) > `on`.
Replies are wrapped in the agent envelope (`{ok:true,data}` / `{ok:false,error}`).
Matching is exact on method and path; the query string is available as
`req.query` but not part of the match.

### `support/app.ts`

- `openApp(page, {group?, tab?, theme?, reducedMotion?, clock?})`: boots the app
  (default theme `light`, set through `localStorage['u60.theme']` and
  `prefers-color-scheme`), then clicks the real navigation, because groups and
  tabs are React state with no URL. `tab` is the visible tab label.
- `VIEWPORTS`: `phone320`, `phone375`, `phone414`, `tablet768`, `desktop`
  (1280 wide; the sidebar replaces the bottom tabs from 1024 up). Use
  `await page.setViewportSize(VIEWPORTS.phone375)` before `openApp`.
- Controlled time: `installClock`, `freezeClock`, `settleRender`,
  `advanceHeartbeat(page, agent, ms = 3000, path = '/api/dashboard')`.

### Controlled time

The heartbeat is a `setTimeout` chain: 3 s on Home, Signal and System → Metrics,
15 s elsewhere, re-armed only after each response. To test it without sleeping:

```ts
await openApp(page, { clock: true })            // fake clock installed before navigation
await expect(page.getByText('78').first()).toBeVisible()
await freezeClock(page)                          // timers now fire only when you say so
await advanceHeartbeat(page, agent)              // runFor(3000) + wait for one more /api/dashboard
```

- Freeze after the first render. React reveals lazily loaded content through a
  300 ms `setTimeout`; on a frozen clock a just-selected group stays blank until
  you call `settleRender(page)`.
- Responses still arrive in real time, so assert with the request log or
  `expect.poll`, not with fixed waits.
- A frozen clock never fires the 15 s request timeout either.

### `support/fixtures.ts`

Raw agent payloads (before the mappers in `src/data/api.ts`), each taking a
partial override that is deep-merged (arrays and `null` replace):
`dashboard({network: 'SA'|'NSA'|'LTE'|'disconnected', ...sections})`, `netinfo`,
`sources`, `freshness`, `dataUsage`, `device`, `cpu`, `memory`, `batteryDetail`,
`batteryInfo`, `thermalAll`, `charger`, `chargeControl`, `modemCapabilities`,
`simInfo`, `simImei`, `smsCapabilities`, `smsList`, `apnMode`, `apnProfiles`,
`wifiStatus`, `clients`, `dns`, `lan`, `ttlStatus`, `usbStatus`, `top`, `atPort`,
`loggerStatus`, CSV bodies and result envelopes. Each carries a provenance
comment ("synthetic" or "sanitised shape based on HK B04"). `dashboard()` takes
`null` for a section to model an unavailable source and
`sources: { signal: { stale: true, error: '...' } }` for freshness.

## Known limits

- Automation does not prove screen-reader usability; the manual assistive
  technology pass from PLAN2 still applies.
- The suite does not check the production bundle, only the Vite dev server.
- The first run after `npm ci` warms Vite's dependency cache; a rare cold-start
  reload can fail one test. Re-run once before suspecting a regression.
