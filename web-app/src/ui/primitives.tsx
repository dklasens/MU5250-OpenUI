import type { ReactNode } from 'react'

// ── Card ──────────────────────────────────────────────────────────────────────

export function Card({
  title,
  action,
  children,
  className = '',
  pad = true,
}: {
  title?: ReactNode
  action?: ReactNode
  children: ReactNode
  className?: string
  pad?: boolean
}) {
  return (
    <section className={`rounded-panel border border-line/8 bg-surface ${className}`}>
      {title != null && (
        <header className="flex items-center justify-between gap-2 border-b border-line/8 px-4 py-2.5">
          <h2 className="font-display text-sm font-semibold tracking-[-0.01em] text-ink">{title}</h2>
          {action}
        </header>
      )}
      <div className={pad ? 'p-4' : ''}>{children}</div>
    </section>
  )
}

// ── Stat / Row / Chip ─────────────────────────────────────────────────────────

export function Stat({
  label,
  value,
  sub,
  tone = 'text-ink',
}: {
  label: string
  value: ReactNode
  sub?: ReactNode
  tone?: string
}) {
  return (
    <div className="min-w-0">
      <p className="label">{label}</p>
      <p className={`tnum mt-1 truncate font-mono text-xl font-medium ${tone}`}>{value}</p>
      {sub != null && <p className="mt-0.5 truncate text-meta text-ink3">{sub}</p>}
    </div>
  )
}

export function Row({
  label,
  value,
  mono = false,
  wrap = false,
}: {
  label: string
  value: ReactNode
  mono?: boolean
  wrap?: boolean
}) {
  return (
    <div className="flex items-baseline justify-between gap-3 py-0.5 text-body">
      <span className="shrink-0 text-ink2">{label}</span>
      <span
        className={`min-w-0 text-right text-ink ${mono ? 'tnum font-mono text-meta' : 'font-medium'} ${
          wrap ? 'break-all' : 'truncate'
        }`}
      >
        {value}
      </span>
    </div>
  )
}

export type ChipTone = 'default' | 'lte' | 'nr' | 'ok' | 'warn' | 'danger' | 'accent'

const CHIP_TONES: Record<ChipTone, string> = {
  default: 'border-line/12 bg-transparent text-ink2',
  lte: 'border-accent/30 bg-accent/8 text-accent',
  nr: 'border-nr/30 bg-nr/8 text-nr',
  ok: 'border-ok/25 bg-ok/10 text-ok',
  warn: 'border-warn/25 bg-warn/10 text-warn',
  danger: 'border-danger/25 bg-danger/10 text-danger',
  accent: 'border-accent/30 bg-accent/8 text-accent',
}

export function Chip({ children, tone = 'default' }: { children: ReactNode; tone?: ChipTone }) {
  return (
    <span
      className={`inline-flex items-center gap-1 whitespace-nowrap rounded-chip border px-1.5 py-px font-mono text-caption font-medium ${CHIP_TONES[tone]}`}
    >
      {children}
    </span>
  )
}

// ── Loading / empty states ────────────────────────────────────────────────────

export function Spinner({ size = 16, className = '' }: { size?: number; className?: string }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      className={`animate-spin ${className}`}
      aria-hidden="true"
    >
      <circle cx={12} cy={12} r={9} stroke="currentColor" strokeOpacity={0.2} strokeWidth={2.5} />
      <path d="M21 12a9 9 0 0 0-9-9" stroke="currentColor" strokeWidth={2.5} strokeLinecap="round" />
    </svg>
  )
}

export function Skeleton({ className = '' }: { className?: string }) {
  return <div className={`animate-pulse rounded-ctl bg-surface2 ${className}`} />
}

export function Empty({ icon, title, body }: { icon?: ReactNode; title: string; body?: string }) {
  return (
    <div className="flex flex-col items-center justify-center gap-1.5 py-10 text-center">
      {icon && <div className="mb-1 text-ink3">{icon}</div>}
      <p className="text-sm font-medium text-ink2">{title}</p>
      {body && <p className="max-w-xs text-xs text-ink3">{body}</p>}
    </div>
  )
}

// ── Progress / meters ─────────────────────────────────────────────────────────

export function Meter({
  pct,
  tone = 'bg-accent',
  className = '',
}: {
  pct: number
  tone?: string
  className?: string
}) {
  const clamped = Math.max(0, Math.min(100, pct))
  return (
    <div className={`h-1.5 overflow-hidden rounded-full bg-surface2 ${className}`}>
      <div
        className={`h-full rounded-full transition-[width] duration-500 ${tone}`}
        style={{ width: `${clamped}%` }}
      />
    </div>
  )
}

/** Signal-quality style bars (1–5). */
export function SignalBars({ bars, large = false }: { bars?: number; large?: boolean }) {
  const n = bars ?? 0
  const color = n >= 4 ? 'bg-ok' : n >= 2 ? 'bg-warn' : 'bg-danger'
  const heights = large ? [10, 16, 22, 28, 34] : [4, 7, 10, 13, 16]
  const width = large ? 'w-1.5' : 'w-1'
  return (
    <div className="flex items-end gap-[3px]" aria-label={`${n} of 5 bars`}>
      {heights.map((h, i) => (
        <div
          key={i}
          className={`${width} rounded-[2px] ${i < n ? color : 'bg-line/15'}`}
          style={{ height: `${h}px` }}
        />
      ))}
    </div>
  )
}
