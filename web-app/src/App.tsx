import { useState, useEffect } from 'react'
import { AUTH_EXPIRED_EVENT, login, setToken, clearToken, hasToken } from './api'
import Sidebar from './components/Sidebar'
import AlertBanner from './components/AlertBanner'
import DashboardPage from './pages/DashboardPage'
import SignalPage from './pages/SignalPage'
import NetworkPage from './pages/NetworkPage'
import WiFiPage from './pages/WiFiPage'
import RouterPage from './pages/RouterPage'
import BandLockPage from './pages/BandLockPage'
import SettingsPage from './pages/SettingsPage'
import MetricsPage from './pages/MetricsPage'
import ModemPage from './pages/ModemPage'
import AdvancedPage from './pages/AdvancedPage'

export type Page = 'dashboard' | 'signal' | 'network' | 'wifi' | 'router' | 'modem' | 'bandlock' | 'metrics' | 'advanced' | 'settings'
type LoginMode = 'password' | 'pin'

function isMobilePinClient() {
  const ua = navigator.userAgent
  return /Android|webOS|iPhone|iPad|iPod|BlackBerry|IEMobile|Opera Mini|Mobile/i.test(ua)
    || (navigator.maxTouchPoints > 1 && /Macintosh/i.test(ua))
}

export default function App() {
  const [authed, setAuthed] = useState(hasToken())
  const [page, setPage] = useState<Page>('dashboard')
  const [loginErr, setLoginErr] = useState('')
  const [pw, setPw] = useState('')
  const [pin, setPin] = useState('')
  const [loginMode, setLoginMode] = useState<LoginMode>(() => isMobilePinClient() ? 'pin' : 'password')
  const [loading, setLoading] = useState(false)
  const [sidebarOpen, setSidebarOpen] = useState(false)
  const mobilePinClient = isMobilePinClient()

  useEffect(() => {
    document.body.style.overflow = sidebarOpen ? 'hidden' : ''
    return () => { document.body.style.overflow = '' }
  }, [sidebarOpen])

  useEffect(() => {
    const handleAuthExpired = () => {
      setAuthed(false)
      setPw('')
      setPin('')
      setLoginMode(isMobilePinClient() ? 'pin' : 'password')
      setLoginErr('Session expired. Sign in again.')
    }

    window.addEventListener(AUTH_EXPIRED_EVENT, handleAuthExpired)
    return () => window.removeEventListener(AUTH_EXPIRED_EVENT, handleAuthExpired)
  }, [])

  async function handleLogin(e: React.FormEvent) {
    e.preventDefault()
    setLoading(true)
    setLoginErr('')
    try {
      const { token } = await login(loginMode === 'pin' ? { pin } : { password: pw })
      setToken(token)
      setAuthed(true)
      setPw('')
      setPin('')
    } catch (error) {
      setLoginErr(error instanceof Error ? error.message : 'Sign in failed')
    } finally {
      setLoading(false)
    }
  }

  function handleLogout() {
    clearToken()
    setAuthed(false)
    setPw('')
    setPin('')
    setLoginMode(isMobilePinClient() ? 'pin' : 'password')
  }

  if (!authed) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-slds-bg p-8">
        <div className="bg-white w-full max-w-sm rounded-2xl shadow-macos-xl border border-black/5 p-8">
          <div className="mb-8 text-center">
            <div className="mx-auto mb-4 flex h-14 w-14 items-center justify-center rounded-2xl bg-slds-blue shadow-sm">
              <svg className="h-7 w-7 text-white" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8.111 16.404a5.5 5.5 0 017.778 0M12 20h.01m-7.08-7.071c3.904-3.905 10.236-3.905 14.141 0M1.394 9.393c5.857-5.857 15.355-5.857 21.213 0" />
              </svg>
            </div>
            <h1 className="text-xl font-bold text-gray-900">ZTE U60 Pro</h1>
            <p className="mt-1 text-xs text-gray-500">Dashboard</p>
          </div>
          <form onSubmit={handleLogin} className="space-y-4">
            {mobilePinClient && (
              <div className="grid grid-cols-2 gap-1 rounded-xl bg-gray-100 p-1">
                {(['pin', 'password'] as const).map(mode => (
                  <button
                    key={mode}
                    type="button"
                    onClick={() => {
                      setLoginMode(mode)
                      setLoginErr('')
                    }}
                    className={`rounded-lg px-3 py-2 text-sm font-bold transition-all ${
                      loginMode === mode ? 'bg-white text-gray-900 shadow-sm' : 'text-gray-500 hover:text-gray-900'
                    }`}
                  >
                    {mode === 'pin' ? 'PIN' : 'Password'}
                  </button>
                ))}
              </div>
            )}
            {loginMode === 'pin' && mobilePinClient ? (
              <div>
                <label className="text-[10px] font-bold text-gray-500 uppercase tracking-widest mb-1.5 block">PIN</label>
                <input
                  type="password"
                  inputMode="numeric"
                  pattern="[0-9]*"
                  maxLength={6}
                  value={pin}
                  onChange={e => setPin(e.target.value.replace(/\D/g, '').slice(0, 6))}
                  className="w-full px-3.5 py-3 bg-gray-50 border border-gray-200 rounded-xl text-center text-2xl font-bold text-gray-900 placeholder-gray-400 focus:ring-0 focus:shadow-macos-focus focus:border-slds-blue outline-none transition-all"
                  placeholder="••••••"
                  autoFocus
                  autoComplete="one-time-code"
                  enterKeyHint="done"
                  aria-label="PIN"
                />
              </div>
            ) : (
              <div>
                <label className="text-[10px] font-bold text-gray-500 uppercase tracking-widest mb-1.5 block">Agent Password</label>
                <input
                  type="password"
                  value={pw}
                  onChange={e => setPw(e.target.value)}
                  className="w-full px-3.5 py-3 bg-gray-50 border border-gray-200 rounded-xl text-base text-gray-900 placeholder-gray-400 focus:ring-0 focus:shadow-macos-focus focus:border-slds-blue outline-none transition-all"
                  placeholder="Enter your agent password"
                  autoFocus
                  autoComplete="current-password"
                />
              </div>
            )}
            {loginErr && <p className="text-xs text-red-500">{loginErr}</p>}
            <button
              type="submit"
              disabled={loading || (loginMode === 'pin' && mobilePinClient ? pin.length !== 6 : !pw)}
              className="w-full bg-slds-blue text-white py-3.5 rounded-2xl font-bold shadow-sm hover:bg-slds-blueHover active:scale-[0.98] disabled:opacity-40 transition-all"
            >
              {loading ? 'Signing in\u2026' : 'Sign in'}
            </button>
          </form>
        </div>
      </div>
    )
  }

  let pageContent: React.ReactNode = null
  switch (page) {
    case 'dashboard': pageContent = <DashboardPage />; break
    case 'signal':    pageContent = <SignalPage />; break
    case 'network':   pageContent = <NetworkPage />; break
    case 'wifi':      pageContent = <WiFiPage />; break
    case 'router':    pageContent = <RouterPage />; break
    case 'modem':     pageContent = <ModemPage />; break
    case 'bandlock':  pageContent = <BandLockPage />; break
    case 'metrics':   pageContent = <MetricsPage />; break
    case 'advanced':  pageContent = <AdvancedPage />; break
    case 'settings':  pageContent = <SettingsPage onLogout={handleLogout} />; break
  }

  return (
    <div className="flex h-screen overflow-hidden bg-slds-bg">
      {sidebarOpen && (
        <div className="fixed inset-0 z-20 bg-gray-900/20 backdrop-blur-sm lg:hidden" onClick={() => setSidebarOpen(false)} />
      )}

      <Sidebar
        page={page}
        onNavigate={p => { setPage(p); setSidebarOpen(false) }}
        open={sidebarOpen}
      />

      <div className="flex flex-1 flex-col min-w-0">
        <header className="flex shrink-0 items-center gap-3 border-b border-black/5 bg-white/70 backdrop-blur-xl px-4 py-3 lg:hidden" style={{ paddingLeft: 'max(1rem, env(safe-area-inset-left, 1rem))' }}>
          <button onClick={() => setSidebarOpen(true)} className="p-2 -ml-2 text-gray-500 hover:text-gray-900 transition-colors">
            <svg className="h-6 w-6" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6h16M4 12h16M4 18h16" />
            </svg>
          </button>
          <span className="font-bold text-gray-900">ZTE U60 Pro</span>
        </header>

        <main className="flex-1 overflow-y-auto p-4 lg:p-6">
          <AlertBanner />
          {pageContent}
        </main>
      </div>
    </div>
  )
}
