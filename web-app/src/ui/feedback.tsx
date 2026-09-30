/* eslint-disable react-refresh/only-export-components */
import { useCallback, useEffect, useState } from 'react'
import { IAlert, ICheck } from '../icons'
import { Button } from './controls'

// ── Toasts ────────────────────────────────────────────────────────────────────

export interface ToastItem {
  id: number
  text: string
  kind: 'ok' | 'err'
}

let toastId = 0
let pushToast: ((t: ToastItem) => void) | null = null

/** Fire a toast from anywhere (imperative). */
export function toast(text: string, kind: 'ok' | 'err' = 'ok') {
  pushToast?.({ id: ++toastId, text, kind })
}

export function toastError(e: unknown, fallback = 'Something went wrong') {
  toast(e instanceof Error ? e.message : fallback, 'err')
}

export function Toaster() {
  const [items, setItems] = useState<ToastItem[]>([])

  useEffect(() => {
    pushToast = (t) => {
      setItems((prev) => [...prev.slice(-2), t])
      setTimeout(() => setItems((prev) => prev.filter((x) => x.id !== t.id)), 3500)
    }
    return () => {
      pushToast = null
    }
  }, [])

  if (items.length === 0) return null
  return (
    <div className="pointer-events-none fixed inset-x-0 bottom-[calc(4.5rem+env(safe-area-inset-bottom))] z-50 flex flex-col items-center gap-1.5 px-4 lg:bottom-6">
      {items.map((t) => (
        <div
          key={t.id}
          className={`pointer-events-auto flex max-w-sm items-center gap-2 rounded-ctl border px-3 py-2 text-body font-medium shadow-sm ${
            t.kind === 'ok'
              ? 'border-ok/25 bg-surface text-ink'
              : 'border-danger/30 bg-surface text-danger'
          }`}
        >
          {t.kind === 'ok' ? <ICheck size={15} className="shrink-0 text-ok" /> : <IAlert size={15} className="shrink-0" />}
          <span className="text-ink">{t.text}</span>
        </div>
      ))}
    </div>
  )
}

// ── Confirm dialog (promise-based) ────────────────────────────────────────────

export interface ConfirmOptions {
  title: string
  body?: string
  confirmLabel?: string
  danger?: boolean
}

let openConfirm: ((opts: ConfirmOptions, resolve: (ok: boolean) => void) => void) | null = null

/** Imperative confirmation dialog. Resolves false when dismissed. */
export function confirm(opts: ConfirmOptions): Promise<boolean> {
  return new Promise((resolve) => {
    if (!openConfirm) {
      resolve(false)
      return
    }
    openConfirm(opts, resolve)
  })
}

export function ConfirmHost() {
  const [opts, setOpts] = useState<ConfirmOptions | null>(null)
  const [resolve, setResolve] = useState<((ok: boolean) => void) | null>(null)

  useEffect(() => {
    openConfirm = (o, r) => {
      setOpts(o)
      setResolve(() => r)
    }
    return () => {
      openConfirm = null
    }
  }, [])

  const close = useCallback(
    (ok: boolean) => {
      resolve?.(ok)
      setOpts(null)
      setResolve(null)
    },
    [resolve],
  )

  // Bound to the open dialog rather than re-bound on every render.
  useEffect(() => {
    if (!opts) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close(false)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [opts, close])

  if (!opts) return null
  return (
    <div
      className="fixed inset-0 z-50 flex items-end justify-center bg-scrim/45 p-4 sm:items-center"
      onClick={() => close(false)}
    >
      <div
        className="w-full max-w-sm rounded-panel border border-line/12 bg-surface p-4"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
      >
        <h3 className="font-display text-sm font-semibold text-ink">{opts.title}</h3>
        {opts.body && <p className="mt-1.5 text-body leading-relaxed text-ink2">{opts.body}</p>}
        <div className="mt-4 flex justify-end gap-2">
          <Button variant="ghost" onClick={() => close(false)}>
            Cancel
          </Button>
          <Button variant={opts.danger ? 'danger' : 'primary'} onClick={() => close(true)} autoFocus>
            {opts.confirmLabel ?? 'Confirm'}
          </Button>
        </div>
      </div>
    </div>
  )
}
