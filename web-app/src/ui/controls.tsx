import type { ButtonHTMLAttributes, InputHTMLAttributes, ReactNode, SelectHTMLAttributes } from 'react'
import { IChevronDown } from '../icons'
import { Spinner } from './primitives'

// ── Buttons ───────────────────────────────────────────────────────────────────

type ButtonVariant = 'primary' | 'subtle' | 'ghost' | 'danger' | 'outline'

const BUTTON_VARIANTS: Record<ButtonVariant, string> = {
  primary: 'bg-accent text-onaccent hover:brightness-110 active:brightness-95',
  subtle: 'border border-line/12 bg-surface text-ink hover:bg-surface2 active:bg-line/10',
  ghost: 'text-ink2 hover:bg-surface2 hover:text-ink',
  danger: 'bg-danger text-onaccent hover:brightness-110 active:brightness-95',
  outline: 'border border-line/15 text-ink hover:border-accent/50 hover:bg-surface2',
}

export function Button({
  variant = 'subtle',
  size = 'md',
  loading = false,
  className = '',
  children,
  disabled,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: ButtonVariant
  size?: 'sm' | 'md'
  loading?: boolean
}) {
  const sizing = size === 'sm' ? 'h-8 px-3 text-meta' : 'h-9 px-3.5 text-body'
  return (
    <button
      className={`inline-flex items-center justify-center gap-1.5 whitespace-nowrap rounded-ctl font-semibold transition-[background-color,border-color,filter,opacity] disabled:pointer-events-none disabled:opacity-45 ${sizing} ${BUTTON_VARIANTS[variant]} ${className}`}
      disabled={disabled || loading}
      {...props}
    >
      {loading && <Spinner size={13} />}
      {children}
    </button>
  )
}

// ── Toggle (switch) ───────────────────────────────────────────────────────────

export function Toggle({
  checked,
  onChange,
  disabled = false,
  label,
}: {
  checked: boolean
  onChange: (next: boolean) => void
  disabled?: boolean
  label?: string
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={`relative inline-flex h-6 w-10 shrink-0 items-center rounded-full transition-colors disabled:opacity-40 ${
        checked ? 'bg-accent' : 'bg-line/20'
      }`}
    >
      <span
        className={`inline-block h-[18px] w-[18px] transform rounded-full bg-onaccent shadow-sm transition-transform ${
          checked ? 'translate-x-[21px]' : 'translate-x-[3px]'
        }`}
      />
    </button>
  )
}

// ── Form fields ───────────────────────────────────────────────────────────────

const CONTROL_CLS =
  'h-9 w-full rounded-ctl border border-line/15 bg-surface px-3 text-body text-ink outline-none transition-colors placeholder:text-ink3 focus:border-accent disabled:opacity-50'

export function Input(props: InputHTMLAttributes<HTMLInputElement>) {
  const { className = '', ...rest } = props
  return <input className={`${CONTROL_CLS} ${className}`} {...rest} />
}

export function Select(props: SelectHTMLAttributes<HTMLSelectElement>) {
  const { className = '', ...rest } = props
  return (
    <span className="relative block w-full">
      <select className={`${CONTROL_CLS} appearance-none pr-8 ${className}`} {...rest} />
      <IChevronDown
        size={14}
        className="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 text-ink3"
      />
    </span>
  )
}

export function Field({ label, children, hint }: { label: string; children: ReactNode; hint?: string }) {
  return (
    <label className="block">
      <span className="label mb-1 block">{label}</span>
      {children}
      {hint && <span className="mt-1 block text-caption text-ink3">{hint}</span>}
    </label>
  )
}

// ── Segmented control ─────────────────────────────────────────────────────────

export function Segmented<T extends string>({
  options,
  value,
  onChange,
  disabled = false,
}: {
  options: { value: T; label: string }[]
  value: T
  onChange: (v: T) => void
  disabled?: boolean
}) {
  return (
    <div className={`inline-flex rounded-ctl border border-line/12 bg-surface2 p-0.5 ${disabled ? 'opacity-50' : ''}`}>
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          disabled={disabled}
          onClick={() => onChange(o.value)}
          className={`whitespace-nowrap rounded-chip px-3 py-1.5 text-meta font-semibold transition-colors ${
            value === o.value ? 'bg-surface text-ink ring-1 ring-line/12' : 'text-ink2 hover:text-ink'
          }`}
        >
          {o.label}
        </button>
      ))}
    </div>
  )
}
