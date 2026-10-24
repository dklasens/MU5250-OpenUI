import { useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from 'react'
import { api, newOperationId } from '../../data/api'
import { ApiError } from '../../data/client'
import { formatBytes } from '../../format'
import type { EsimOperation, EsimProfile, EsimView } from '../../types'
import { Button, Field, Input } from '../../ui/controls'
import { InlineStatus, Spinner, Unavailable } from '../../ui/primitives'
import {
  activateBlock,
  checkActivationCode,
  confirmationCodeError,
  connectingSteps,
  formatCountdown,
  NICKNAME_MAX_BYTES,
  nicknameBytes,
  nicknameError,
  operationCopy,
  writeErrorCopy,
} from './simView'

/**
 * Adding an eSIM (docs/AGENT.md § eSIM): code → connecting → preview → installing → result.
 *
 * The activation and confirmation codes are secrets. They live only in this component's state,
 * are sent once, and are cleared as soon as the agent accepts them (and on unmount, with the
 * state). They never reach storage, the URL, a log, a toast or an error message.
 */
export function AddEsimPanel({
  view,
  op,
  onView,
  onClose,
  onRetry,
  onActivate,
}: {
  view: EsimView
  /** The download being shown, or null for the code step. */
  op: EsimOperation | null
  onView: (v: EsimView) => void
  onClose: () => void
  onRetry: () => void
  onActivate: (p: EsimProfile) => void
}) {
  const headingId = useId()
  return (
    <section aria-labelledby={headingId} className="space-y-3 rounded-ctl border border-line/8 p-3">
      <h3 id={headingId} className="font-display text-body font-semibold tracking-[-0.01em] text-ink">
        Add an eSIM
      </h3>
      {!op ? (
        <CodeStep view={view} onView={onView} onClose={onClose} />
      ) : op.state === 'awaiting_confirmation' ? (
        <PreviewStep view={view} op={op} onView={onView} />
      ) : !op.resolved ? (
        <ProgressStep op={op} onView={onView} />
      ) : (
        <ResultStep view={view} op={op} onClose={onClose} onRetry={onRetry} onActivate={onActivate} />
      )}
    </section>
  )
}

function useRequestError() {
  const [error, setError] = useState<string | null>(null)
  const fail = (e: unknown) => {
    const status = e instanceof ApiError ? e.status : undefined
    setError(writeErrorCopy(status, e instanceof Error ? e.message : 'The request failed.'))
  }
  return { error, setError, fail }
}

// ── 1. Code ───────────────────────────────────────────────────────────────────

function CodeStep({ view, onView, onClose }: { view: EsimView; onView: (v: EsimView) => void; onClose: () => void }) {
  const [code, setCode] = useState('')
  const [reveal, setReveal] = useState(false)
  const [confirmation, setConfirmation] = useState('')
  const [touched, setTouched] = useState(false)
  const [busy, setBusy] = useState<'start' | 'qr' | null>(null)
  const [qrNote, setQrNote] = useState<string | null>(null)
  const { error, setError, fail } = useRequestError()
  const fileRef = useRef<HTMLInputElement>(null)

  const check = checkActivationCode(code)
  const needsConfirmation = check.ok && check.needsConfirmation
  const confError = needsConfirmation ? confirmationCodeError(confirmation, true) : null
  const codeError = touched && !check.ok ? check.error : undefined

  async function start(e: FormEvent) {
    e.preventDefault()
    setTouched(true)
    if (!check.ok || confError || busy) return
    setBusy('start')
    setError(null)
    try {
      const v = await api.esimDownload(code.trim(), needsConfirmation ? confirmation : null, newOperationId())
      // The agent has the code now; drop it from the page.
      setCode('')
      setConfirmation('')
      onView(v)
    } catch (err) {
      fail(err)
    } finally {
      setBusy(null)
    }
  }

  async function readQr(file: File | undefined) {
    if (!file) return
    setBusy('qr')
    setQrNote(null)
    setError(null)
    try {
      const { decodeQrImage } = await import('./qrDecode')
      const text = await decodeQrImage(file)
      if (!text) setQrNote('No QR code could be read from that image. Try a sharper screenshot, or paste the code.')
      else if (!checkActivationCode(text).ok)
        setQrNote('That QR code isn’t an eSIM activation code. Check you picked the eSIM QR from your provider.')
      else {
        setCode(text)
        setReveal(false)
        setTouched(true)
        setQrNote('QR code read. Check the server below, then download.')
      }
    } catch {
      setQrNote('That image couldn’t be opened. Try a PNG or JPEG screenshot.')
    } finally {
      setBusy(null)
      if (fileRef.current) fileRef.current.value = ''
    }
  }

  return (
    <form className="space-y-3" onSubmit={start} noValidate>
      <p className="text-meta text-ink2">
        Paste the activation code from your provider (it starts with <span className="font-mono">LPA:1$</span>), or
        upload a picture of the eSIM QR code. The image is read in this browser and isn’t uploaded. The download runs
        over the router’s own mobile data.
      </p>
      <div className="flex flex-wrap items-end gap-2">
        <div className="min-w-0 flex-1 basis-56">
          <Field label="Activation code" error={codeError} hint={check.ok ? `Server: ${check.host}` : undefined}>
            {(ids) => (
              <Input
                id={ids.id}
                type={reveal ? 'text' : 'password'}
                value={code}
                autoComplete="off"
                autoCapitalize="off"
                spellCheck={false}
                onChange={(e) => {
                  setCode(e.target.value)
                  setQrNote(null)
                }}
                onBlur={() => setTouched(code.length > 0)}
                aria-describedby={ids.describedBy}
                aria-invalid={ids.invalid || undefined}
              />
            )}
          </Field>
        </div>
        <div className="flex gap-1.5 pb-px">
          <Button type="button" size="sm" variant="ghost" aria-pressed={reveal} onClick={() => setReveal((r) => !r)}>
            {reveal ? 'Hide' : 'Show'}
          </Button>
          <Button
            type="button"
            size="sm"
            variant="outline"
            loading={busy === 'qr'}
            disabled={busy !== null}
            onClick={() => fileRef.current?.click()}
          >
            Upload QR image
          </Button>
          <input
            ref={fileRef}
            type="file"
            accept="image/*"
            className="sr-only"
            tabIndex={-1}
            aria-hidden="true"
            data-testid="qr-file"
            onChange={(e) => void readQr(e.target.files?.[0])}
          />
        </div>
      </div>
      {qrNote && (
        <InlineStatus kind={qrNote.startsWith('QR code read') ? 'ok' : 'warn'}>{qrNote}</InlineStatus>
      )}
      {needsConfirmation && (
        <Field
          label="Confirmation code"
          hint="Your provider gives this separately from the QR code."
          error={touched ? confError ?? undefined : undefined}
        >
          {(ids) => (
            <Input
              id={ids.id}
              type="password"
              value={confirmation}
              autoComplete="off"
              spellCheck={false}
              onChange={(e) => setConfirmation(e.target.value)}
              aria-describedby={ids.describedBy}
              aria-invalid={ids.invalid || undefined}
            />
          )}
        </Field>
      )}
      {error && (
        <InlineStatus kind="error" action={{ label: 'Dismiss', onClick: () => setError(null) }}>
          {error}
        </InlineStatus>
      )}
      <p className="text-meta text-ink3">
        Free storage on the card:{' '}
        <span className="font-mono">
          {view.card?.free_nvm_bytes != null ? formatBytes(view.card.free_nvm_bytes) : <Unavailable />}
        </span>
        . Nothing is installed until you review the profile and choose Install.
      </p>
      <div className="flex flex-wrap gap-1.5">
        <Button type="submit" size="sm" variant="primary" loading={busy === 'start'} disabled={busy !== null}>
          Download
        </Button>
        <Button type="button" size="sm" variant="ghost" onClick={onClose} disabled={busy === 'start'}>
          Cancel
        </Button>
      </div>
    </form>
  )
}

// ── 2. Connecting / installing ────────────────────────────────────────────────

function ProgressStep({ op, onView }: { op: EsimOperation; onView: (v: EsimView) => void }) {
  const [cancelling, setCancelling] = useState(false)
  const { error, setError, fail } = useRequestError()

  async function cancel() {
    setCancelling(true)
    setError(null)
    try {
      onView(await api.esimDownloadCancel(op.id))
    } catch (e) {
      fail(e)
    } finally {
      setCancelling(false)
    }
  }

  if (op.state === 'installing') {
    return <InlineStatus kind="info">{operationCopy(op).text}</InlineStatus>
  }
  return (
    <div className="space-y-3">
      <ol className="space-y-1" aria-label="Download progress">
        {connectingSteps(op.step).map((s) => (
          <li
            key={s.label}
            aria-current={s.status === 'current' ? 'step' : undefined}
            className={`flex items-center gap-2 text-meta ${s.status === 'todo' ? 'text-ink3' : 'text-ink'}`}
          >
            <StepMark status={s.status} />
            {s.label}
          </li>
        ))}
      </ol>
      <p className="sr-only" role="status">
        {operationCopy(op).text}
      </p>
      {error && (
        <InlineStatus kind="error" action={{ label: 'Dismiss', onClick: () => setError(null) }}>
          {error}
        </InlineStatus>
      )}
      {op.can_cancel && (
        <Button size="sm" variant="ghost" loading={cancelling} onClick={() => void cancel()}>
          Cancel download
        </Button>
      )}
    </div>
  )
}

function StepMark({ status }: { status: 'done' | 'current' | 'todo' }): ReactNode {
  if (status === 'current') return <Spinner size={12} />
  return (
    <span
      aria-hidden="true"
      className={`inline-block h-3 w-3 rounded-full border ${status === 'done' ? 'border-ok bg-ok' : 'border-line/20'}`}
    />
  )
}

// ── 3. Preview ────────────────────────────────────────────────────────────────

/**
 * Counts down locally from the agent's last reported remaining time. Each new report (every poll)
 * restarts the count from the agent's figure, so the display never drifts far from it.
 */
function useCountdown(ms: number | null): number | null {
  const [tick, setTick] = useState({ ms, elapsed: 0 })
  // A new report from the agent: adjust during render (as usePoll does), not in an effect.
  if (tick.ms !== ms) setTick({ ms, elapsed: 0 })
  useEffect(() => {
    if (ms === null) return
    const t = setInterval(() => setTick((s) => ({ ...s, elapsed: s.elapsed + 1000 })), 1000)
    return () => clearInterval(t)
  }, [ms])
  return ms === null ? null : Math.max(0, ms - tick.elapsed)
}

function PreviewStep({ view, op, onView }: { view: EsimView; op: EsimOperation; onView: (v: EsimView) => void }) {
  const [nickname, setNickname] = useState('')
  const [busy, setBusy] = useState<'install' | 'cancel' | null>(null)
  const { error, setError, fail } = useRequestError()
  const left = useCountdown(op.preview_expires_in_ms)
  const nameError = nicknameError(nickname)
  const bytes = nicknameBytes(nickname)
  const p = op.preview

  async function act(kind: 'install' | 'cancel') {
    if (busy || (kind === 'install' && nameError)) return
    setBusy(kind)
    setError(null)
    try {
      // Cancel postpones the order with the operator, which keeps a single-use QR code usable.
      onView(
        kind === 'install'
          ? await api.esimDownloadConfirm(op.id, true, nickname.trim() || undefined)
          : await api.esimDownloadCancel(op.id),
      )
    } catch (e) {
      fail(e)
    } finally {
      setBusy(null)
    }
  }

  const fact = (label: string, value: ReactNode) => (
    <div className="min-w-0">
      <dt className="label">{label}</dt>
      <dd className="mt-0.5 break-words text-body text-ink">{value}</dd>
    </div>
  )
  const countdown = formatCountdown(left)

  return (
    <form
      className="space-y-3"
      onSubmit={(e) => {
        e.preventDefault()
        void act('install')
      }}
    >
      <p className="text-meta text-ink2">
        The operator’s server says it will install this profile. Nothing has been written to the card yet.
        {countdown && (
          <>
            {' '}
            Answer within <span className="tnum font-mono text-ink">{countdown}</span>, or it is cancelled.
          </>
        )}
      </p>
      <dl className="grid grid-cols-2 gap-x-4 gap-y-3 sm:grid-cols-4">
        {fact('Provider', p?.provider ?? <Unavailable />)}
        {fact('Profile name', p?.name ?? <Unavailable />)}
        {fact('Type', p?.profile_class ?? <Unavailable />)}
        {fact(
          'Free storage',
          <span className="tnum font-mono">
            {view.card?.free_nvm_bytes != null ? formatBytes(view.card.free_nvm_bytes) : <Unavailable />}
          </span>,
        )}
      </dl>
      <Field
        label="Nickname (optional)"
        hint={nameError ? undefined : `${bytes} of ${NICKNAME_MAX_BYTES} bytes. You can rename it later.`}
        error={nameError ?? undefined}
      >
        {(ids) => (
          <Input
            id={ids.id}
            value={nickname}
            spellCheck={false}
            onChange={(e) => setNickname(e.target.value)}
            aria-describedby={ids.describedBy}
            aria-invalid={ids.invalid || undefined}
          />
        )}
      </Field>
      {error && (
        <InlineStatus kind="error" action={{ label: 'Dismiss', onClick: () => setError(null) }}>
          {error}
        </InlineStatus>
      )}
      <div className="flex flex-wrap gap-1.5">
        <Button type="submit" size="sm" variant="primary" loading={busy === 'install'} disabled={busy !== null || !!nameError}>
          Install
        </Button>
        <Button type="button" size="sm" variant="ghost" loading={busy === 'cancel'} disabled={busy !== null} onClick={() => void act('cancel')}>
          Cancel
        </Button>
      </div>
    </form>
  )
}

// ── 4. Result ─────────────────────────────────────────────────────────────────

function ResultStep({
  view,
  op,
  onClose,
  onRetry,
  onActivate,
}: {
  view: EsimView
  op: EsimOperation
  onClose: () => void
  onRetry: () => void
  onActivate: (p: EsimProfile) => void
}) {
  const copy = operationCopy(op)
  const installed = op.state === 'succeeded' ? view.profiles.find((p) => p.handle === op.target) ?? null : null
  const canActivate = !!installed && installed.state === 'disabled' && activateBlock(installed) === null
  return (
    <div className="space-y-3">
      <InlineStatus kind={copy.tone}>{copy.text}</InlineStatus>
      {op.state === 'succeeded' && (
        <p className="text-meta text-ink3">
          Activating it switches the router to this profile; your current profile is deactivated.
        </p>
      )}
      <div className="flex flex-wrap gap-1.5">
        {canActivate && installed && (
          <Button size="sm" variant="outline" onClick={() => onActivate(installed)}>
            Activate now
          </Button>
        )}
        {op.state !== 'succeeded' && (
          <Button size="sm" variant="outline" onClick={onRetry}>
            Try again
          </Button>
        )}
        <Button size="sm" variant="ghost" onClick={onClose}>
          {op.state === 'succeeded' ? 'Done' : 'Close'}
        </Button>
      </div>
    </div>
  )
}
