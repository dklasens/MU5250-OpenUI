/* eslint-disable react-refresh/only-export-components */
import { Suspense, useEffect, useState, type ReactNode } from 'react'
import { useAlerts } from './HomeContext'
import { IGauge, IGlobe, IHome, ISim, ISignal, IX, IMoon, ISun } from '../icons'
import { Mark } from '../ui/Mark'
import { Spinner } from '../ui/primitives'

export type Group = 'home' | 'signal' | 'network' | 'modem' | 'system'

export const NAV: { id: Group; label: string; icon: (p: { size?: number; className?: string }) => ReactNode }[] = [
  { id: 'home', label: 'Home', icon: (p) => <IHome {...p} /> },
  { id: 'signal', label: 'Signal', icon: (p) => <ISignal {...p} /> },
  { id: 'network', label: 'Network', icon: (p) => <IGlobe {...p} /> },
  { id: 'modem', label: 'Modem', icon: (p) => <ISim {...p} /> },
  { id: 'system', label: 'System', icon: (p) => <IGauge {...p} /> },
]

const GROUP_TITLES: Record<Group, string> = {
  home: 'Home',
  signal: 'Signal',
  network: 'Network',
  modem: 'Modem',
  system: 'System',
}

// ── Alert banner (fed by the home poll — zero extra requests) ─────────────────

function AlertBanner() {
  const alerts = useAlerts()
  const [dismissed, setDismissed] = useState<Set<string>>(new Set())

  const visible = alerts.filter((a) => !dismissed.has(a.message))
  if (visible.length === 0) return null

  return (
    <div className="mb-4 space-y-1.5">
      {visible.map((a) => (
        <div
          key={a.message}
          className={`flex items-center gap-2.5 rounded-ctl border px-3 py-2 text-body font-medium ${
            a.level === 'error'
              ? 'border-danger/25 bg-danger/8 text-danger'
              : 'border-warn/25 bg-warn/8 text-warn'
          }`}
          role="alert"
        >
          <span className="min-w-0 flex-1">{a.message}</span>
          <button
            onClick={() => setDismissed((prev) => new Set(prev).add(a.message))}
            className="shrink-0 opacity-60 transition-opacity hover:opacity-100"
            aria-label="Dismiss"
          >
            <IX size={14} />
          </button>
        </div>
      ))}
    </div>
  )
}

// ── Shell ─────────────────────────────────────────────────────────────────────

export default function Shell({
  group,
  onNavigate,
  theme,
  onToggleTheme,
  children,
}: {
  group: Group
  onNavigate: (g: Group) => void
  theme: 'light' | 'dark'
  onToggleTheme: () => void
  children: ReactNode
}) {
  // Lock body scroll while nothing needs it; keeps mobile address bar behavior sane.
  useEffect(() => {
    document.body.style.overflow = ''
  }, [])

  const themeIcon =
    theme === 'dark' ? <ISun size={17} /> : <IMoon size={17} />

  return (
    <div className="flex h-full bg-bg">
      {/* Desktop sidebar */}
      <aside className="hidden w-56 shrink-0 flex-col border-r border-line/8 bg-surface lg:flex">
        <div className="flex items-center gap-2.5 px-5 pb-5 pt-6">
          <Mark size={22} className="shrink-0 text-ink" />
          <div className="min-w-0">
            <p className="truncate font-display text-sm font-semibold tracking-[-0.01em] text-ink">U60 Pro</p>
            <p className="tnum truncate font-mono text-caption text-ink3">{window.location.hostname}</p>
          </div>
        </div>

        <nav className="flex-1 space-y-0.5 px-3">
          {NAV.map((item) => (
            <button
              key={item.id}
              onClick={() => onNavigate(item.id)}
              className={`flex w-full items-center gap-2.5 whitespace-nowrap rounded-ctl px-3 py-2 text-body font-semibold transition-colors ${
                group === item.id
                  ? 'bg-surface2 text-ink [&>svg]:text-accent'
                  : 'text-ink2 hover:bg-surface2 hover:text-ink'
              }`}
              aria-current={group === item.id ? 'page' : undefined}
            >
              {item.icon({ size: 17 })}
              {item.label}
            </button>
          ))}
        </nav>

        <div className="border-t border-line/8 px-5 py-3">
          <button
            onClick={onToggleTheme}
            className="flex items-center gap-2 text-meta font-medium text-ink2 transition-colors hover:text-ink"
          >
            {themeIcon}
            {theme === 'dark' ? 'Light mode' : 'Dark mode'}
          </button>
        </div>
      </aside>

      {/* Main column */}
      <div className="flex min-w-0 flex-1 flex-col">
        {/* Mobile top bar */}
        <header
          className="flex min-h-12 shrink-0 items-center justify-between border-b border-line/8 bg-surface px-4 lg:hidden"
          style={{ paddingTop: 'env(safe-area-inset-top)' }}
        >
          <span className="font-display text-base font-semibold tracking-[-0.01em] text-ink">{GROUP_TITLES[group]}</span>
          <button
            onClick={onToggleTheme}
            className="flex h-8 w-8 items-center justify-center rounded-ctl text-ink2 transition-colors hover:bg-surface2 hover:text-ink"
            aria-label="Toggle theme"
          >
            {themeIcon}
          </button>
        </header>

        <main className="min-h-0 flex-1 overflow-y-auto">
          <div className="mx-auto w-full max-w-shell px-4 pb-24 pt-4 lg:px-6 lg:pb-10 lg:pt-6">
            <AlertBanner />
            <Suspense
              fallback={
                <div className="flex justify-center py-20 text-ink3">
                  <Spinner size={22} />
                </div>
              }
            >
              {children}
            </Suspense>
          </div>
        </main>

        {/* Mobile bottom tabs */}
        <nav
          className="fixed inset-x-0 bottom-0 z-30 border-t border-line/8 bg-surface lg:hidden"
          style={{ paddingBottom: 'env(safe-area-inset-bottom)' }}
        >
          <div className="mx-auto flex max-w-md items-stretch justify-around">
            {NAV.map((item) => {
              const active = group === item.id
              return (
                <button
                  key={item.id}
                  onClick={() => onNavigate(item.id)}
                  className={`-mt-px flex flex-1 flex-col items-center gap-0.5 border-t-2 pb-1.5 pt-2 text-caption font-semibold transition-colors ${
                    active ? 'border-accent text-ink [&>svg]:text-accent' : 'border-transparent text-ink3 hover:text-ink2'
                  }`}
                  aria-current={active ? 'page' : undefined}
                >
                  {item.icon({ size: 20 })}
                  {item.label}
                </button>
              )
            })}
          </div>
        </nav>
      </div>
    </div>
  )
}
