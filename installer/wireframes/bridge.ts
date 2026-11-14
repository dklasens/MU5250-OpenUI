// Sample-only bridge for documentation captures. Never included in desktop builds.
const listeners = new Map<string, (event: { payload: unknown }) => void>()
const scene = new URLSearchParams(window.location.search).get('scene') ?? 'connect'
let finishInstall: ((value: unknown) => void) | undefined
const result = { result: 'success', title: 'Installation complete', message: 'The dashboard and secure access are ready.', operation: 'install', deviceModel: 'ZTE MU5250 · Sample device', firmware: 'Example compatible firmware', release: 'v2.5', dashboardUrl: 'http://192.168.0.1:8080', apiUrl: 'http://192.168.0.1:9090', sshAddress: 'Managed SSH identity · port 2222', diagnosticPath: null }
export async function invoke<T>(command: string, args?: Record<string, any>): Promise<T> {
  if (command === 'startup_mode') return false as T
  if (command === 'detect_device') return { detectionId: 'sample-plan', gateway: '192.168.0.1', adbPath: 'Bundled ADB', adbDevices: [], selectedAdbSerial: null, selectionRequired: false, mode: scene === 'unlock' || scene === 'connect' ? 'unlock' : 'ssh', operation: scene === 'unlock' || scene === 'connect' ? 'install' : 'update', services: { web: true, agent: scene !== 'connect', ssh: scene !== 'connect', adb: false }, connectionSummary: 'ZTE U60 Pro / MU5250', planSummary: 'Join the modem Wi-Fi and connect the USB cable. Check the device before installing.', ready: scene !== 'recovery', problems: [], recovery: scene === 'recovery' ? { id: '00000000-0000-4000-8000-000000000000', incomplete: false } : null } as T
  if (command === 'run_install') {
    if (args?.request.dryRun) return { ...result, result: 'dryRun', title: 'Device checks passed', message: 'Verified files, identity and installation prerequisites.' } as T
    if (scene === 'unlock') {
      return new Promise((resolve) => {
        finishInstall = resolve
        listeners.get('installer-progress')?.({ payload: { kind: 'confirm' } })
      }) as Promise<T>
    }
    return result as T
  }
  if (command === 'confirm_unlock') { finishInstall?.(result); return undefined as T }
  return undefined as T
}
export async function listen(name: string, callback: (event: { payload: unknown }) => void) { listeners.set(name, callback); return () => { if (listeners.get(name) === callback) listeners.delete(name) } }
export async function open() { return null }
