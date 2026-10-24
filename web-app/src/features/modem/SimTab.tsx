import { useEffect, useRef, useState, type ReactNode } from 'react'
import { api, newOperationId } from '../../data/api'
import { ApiError } from '../../data/client'
import { usePoll } from '../../data/poll'
import { formatBytes } from '../../format'
import type { EsimProfile, EsimView } from '../../types'
import { Button, Field, Input } from '../../ui/controls'
import { confirm } from '../../ui/feedback'
import { Card, Chip, Empty, InlineStatus, Skeleton, Unavailable } from '../../ui/primitives'
import { AddEsimPanel } from './AddEsimPanel'
import {
  activateBlock,
  activateConfirm,
  canAddProfile,
  deactivateConfirm,
  deleteBlock,
  deleteConfirm,
  emptyState,
  IDLE_POLL_MS,
  inFlight,
  NICKNAME_MAX_BYTES,
  nicknameBytes,
  nicknameError,
  operationCopy,
  pollInterval,
  profileLabel,
  receiptsCopy,
  sortProfiles,
  staleCopy,
  writeErrorCopy,
  writesDisabled,
} from './simView'

/** Writes return the full view; a failed write is reported here and never retried automatically. */
type Write = () => Promise<EsimView>

export default function SimTab() {
  const [interval, setPollInterval] = useState(IDLE_POLL_MS)
  const poll = usePoll<EsimView>('esim:profiles', api.esimProfiles, interval)
  const view = poll.data
  const [writing, setWriting] = useState<string | null>(null)
  const [writeError, setWriteError] = useState<string | null>(null)
  const [refreshing, setRefreshing] = useState(false)
  const [dismissed, setDismissed] = useState<string | null>(null)
  /** The add panel is open (code step), or showing a download the user hasn't closed yet. */
  const [adding, setAdding] = useState(false)
  /** A row's inline rename is open: Add eSIM steps down from primary (one primary per view). */
  const [renaming, setRenaming] = useState<string | null>(null)

  // Fast polling only while a change settles on the card.
  const wanted = pollInterval(view)
  useEffect(() => {
    if (wanted !== interval) setPollInterval(wanted)
  }, [wanted, interval])
  // The scheduler reads the interval only when it sets its next timer, so a change that has just
  // started would otherwise wait out the slow 30 s timer. Read now; the next timer uses 3 s.
  const { refresh: refreshNow } = poll
  const lastInterval = useRef(interval)
  useEffect(() => {
    if (interval < lastInterval.current) refreshNow()
    lastInterval.current = interval
  }, [interval, refreshNow])

  /** Resolves true when the agent accepted the request. */
  async function write(key: string, call: Write): Promise<boolean> {
    if (writing) return false
    setWriting(key)
    setWriteError(null)
    try {
      poll.mutate(await call())
      return true
    } catch (e) {
      const status = e instanceof ApiError ? e.status : undefined
      setWriteError(writeErrorCopy(status, e instanceof Error ? e.message : 'The request failed.'))
      // A stale selection or a change elsewhere: show the card as it is now.
      if (status === 409 || status === 404) poll.refresh()
      return false
    } finally {
      setWriting(null)
    }
  }

  async function refresh() {
    if (refreshing) return
    setRefreshing(true)
    try {
      poll.mutate(await api.esimRefresh())
    } catch {
      poll.refresh()
    } finally {
      setRefreshing(false)
    }
  }

  if (poll.status === 'loading') {
    return (
      <Card title="eSIM card">
        <div className="space-y-2">
          <Skeleton className="h-10" />
          <Skeleton className="h-16" />
        </div>
      </Card>
    )
  }
  if (!view) {
    return (
      <Card title="eSIM card">
        <InlineStatus kind="error" action={{ label: 'Retry', onClick: poll.refresh, loading: poll.refreshing }}>
          The eSIM profiles could not be read{poll.error ? `: ${poll.error}` : '.'}
        </InlineStatus>
      </Card>
    )
  }

  const profiles = sortProfiles(view.profiles)
  const active = profiles.find((p) => p.state === 'enabled') ?? null
  const empty = emptyState(view)
  const op = view.operation
  const showOp = op && (!op.resolved || dismissed !== op.id)
  // A download is shown by the add panel, not by the generic status line above the list.
  const downloadOp = showOp && op?.kind === 'download' ? op : null
  const showPanel = adding || downloadOp !== null
  const codeStep = showPanel && downloadOp === null
  // One change at a time: the code step counts as a change being prepared.
  const blocked = writesDisabled(view) || writing !== null || codeStep

  function closePanel() {
    setAdding(false)
    if (downloadOp?.resolved) setDismissed(downloadOp.id)
  }
  function retryDownload() {
    if (downloadOp) setDismissed(downloadOp.id)
    setAdding(true)
  }

  async function remove(p: EsimProfile) {
    if (blocked || view?.revision == null || deleteBlock(p) !== null) return
    const frozen = { handle: p.handle, revision: view.revision, id: newOperationId() }
    if (!(await confirm({ ...deleteConfirm(p), kind: 'danger' }))) return
    await write(`delete:${p.handle}`, () => api.esimDelete(frozen.handle, frozen.revision, frozen.id))
  }

  async function activate(p: EsimProfile) {
    if (blocked || view?.revision == null) return
    // Freeze what the dialog shows and what is sent.
    const frozen = { handle: p.handle, revision: view.revision, id: newOperationId() }
    const copy = activateConfirm(active, p)
    if (!(await confirm({ ...copy, kind: 'connection' }))) return
    await write(`enable:${p.handle}`, () => api.esimEnable(frozen.handle, frozen.revision, frozen.id))
  }

  async function deactivate(p: EsimProfile) {
    if (blocked || view?.revision == null) return
    const frozen = { handle: p.handle, revision: view.revision, id: newOperationId() }
    if (!(await confirm({ ...deactivateConfirm(p), kind: 'connection' }))) return
    await write(`disable:${p.handle}`, () => api.esimDisable(frozen.handle, frozen.revision, frozen.id))
  }

  return (
    <>
      <Card
        title="eSIM card"
        action={
          <Button size="sm" variant="ghost" onClick={refresh} loading={refreshing} disabled={inFlight(view)}>
            Refresh
          </Button>
        }
      >
        <div className="space-y-3">
          {poll.status === 'stale' && (
            <InlineStatus kind="stale" action={{ label: 'Retry', onClick: poll.refresh, loading: poll.refreshing }}>
              The agent could not be reached. Showing the last profile list read.
            </InlineStatus>
          )}
          {view.freshness.stale && view.freshness.error && view.profiles.length > 0 && !inFlight(view) && (
            <InlineStatus kind="stale" action={{ label: 'Retry', onClick: refresh, loading: refreshing }}>
              {staleCopy(view.freshness.error.kind)}
            </InlineStatus>
          )}
          {view.card ? (
            <CardFacts
              view={view}
              sending={view.receipts.sending || writing === 'receipts'}
              disabled={blocked || view.receipts.sending}
              onSend={() => void write('receipts', api.esimSendReceipts)}
            />
          ) : (
            !empty && <Unavailable label="Card details unavailable" />
          )}
          <p className="text-meta text-ink3">
            “Active” is the card’s selected profile. Whether mobile data is connected is shown on Home.
            {!view.switching_validated && ' Switching profiles has not yet been validated on this hardware.'}
          </p>
        </div>
      </Card>

      <Card
        title="Profiles"
        action={
          !showPanel && view.status === 'ready' ? (
            <Button
              size="sm"
              variant={renaming ? 'outline' : 'primary'}
              disabled={!canAddProfile(view) || writing !== null || renaming !== null}
              onClick={() => {
                setWriteError(null)
                setAdding(true)
              }}
            >
              Add eSIM
            </Button>
          ) : undefined
        }
      >
        <div className="space-y-3">
          {showPanel && (
            <AddEsimPanel
              view={view}
              op={downloadOp}
              onView={poll.mutate}
              onClose={closePanel}
              onRetry={retryDownload}
              onActivate={(p) => void activate(p)}
            />
          )}
          {showOp && op && op.kind !== 'download' && (
            <OperationStatus
              tone={operationCopy(op).tone}
              resolved={op.resolved}
              onDismiss={() => setDismissed(op.id)}
            >
              {operationCopy(op).text}
            </OperationStatus>
          )}
          {writeError && (
            <InlineStatus kind="error" action={{ label: 'Dismiss', onClick: () => setWriteError(null) }}>
              {writeError}
            </InlineStatus>
          )}
          {empty ? (
            <Empty title={empty.title} body={empty.body} />
          ) : (
            <>
              {!active && (
                <InlineStatus kind="warn" live={false}>
                  No profile is active, so there is no cellular service. Activate a profile to restore it.
                </InlineStatus>
              )}
              <ul className="space-y-2" aria-label="eSIM profiles">
                {profiles.map((p) => (
                  <ProfileRow
                    key={p.handle}
                    profile={p}
                    blocked={blocked}
                    writing={writing}
                    revision={view.revision}
                    onActivate={() => void activate(p)}
                    onDeactivate={() => void deactivate(p)}
                    onLock={(locked) => void write(`lock:${p.handle}`, () => api.esimLock(p.handle, locked))}
                    onDelete={() => void remove(p)}
                    onEditing={(editing) => setRenaming(editing ? p.handle : null)}
                    onRename={(name, revision) =>
                      write(`nickname:${p.handle}`, () => api.esimNickname(p.handle, revision, newOperationId(), name))
                    }
                  />
                ))}
              </ul>
            </>
          )}
        </div>
      </Card>
    </>
  )
}

function OperationStatus({
  tone,
  resolved,
  onDismiss,
  children,
}: {
  tone: 'info' | 'ok' | 'warn' | 'error'
  resolved: boolean
  onDismiss: () => void
  children: ReactNode
}) {
  return (
    <InlineStatus kind={tone} action={resolved ? { label: 'Dismiss', onClick: onDismiss } : undefined}>
      <span className="inline-flex flex-wrap items-center gap-2">
        {!resolved && <Chip tone="default">In progress</Chip>}
        <span>{children}</span>
      </span>
    </InlineStatus>
  )
}

function CardFacts({
  view,
  sending,
  disabled,
  onSend,
}: {
  view: EsimView
  sending: boolean
  disabled: boolean
  onSend: () => void
}) {
  const c = view.card
  if (!c) return null
  const note = receiptsCopy(view)
  const fact = (label: string, value: ReactNode) => (
    <div className="min-w-0">
      <dt className="label">{label}</dt>
      <dd className="tnum mt-0.5 truncate font-mono text-body text-ink">{value}</dd>
    </div>
  )
  const text = (v: string | null, label: string) => v ?? <Unavailable label={label} />
  return (
    <div className="space-y-2">
      <dl className="grid grid-cols-2 gap-x-4 gap-y-3 sm:grid-cols-3">
        {fact('Card', text(c.id, 'Card id unavailable'))}
        {fact('Firmware', text(c.firmware, 'Firmware unavailable'))}
        {fact('SGP.22', text(c.sgp22_version, 'SGP.22 version unavailable'))}
        {fact('Profile package', text(c.profile_version, 'Profile package version unavailable'))}
        {fact('Free storage', c.free_nvm_bytes === null ? <Unavailable /> : formatBytes(c.free_nvm_bytes))}
        {fact('Receipts on card', c.pending_receipts === null ? <Unavailable /> : c.pending_receipts)}
      </dl>
      {(c.pending_receipts ?? 0) > 0 && (
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
          <Button size="sm" variant="outline" loading={sending} disabled={disabled} onClick={onSend}>
            Send receipts
          </Button>
          <p className="min-w-0 flex-1 basis-52 text-meta text-ink3">
            Delivers the card’s receipts to each operator’s server; each is removed from the card only once its server
            accepts it.
          </p>
        </div>
      )}
      {note && (
        <p className="text-meta text-ink2" role="status">
          {note}
        </p>
      )}
      {view.receipts.pending > 0 && (
        <p className="text-meta text-ink2">
          {view.receipts.pending === 1 ? '1 operator receipt' : `${view.receipts.pending} operator receipts`} from recent
          changes {view.receipts.gave_up ? 'could not be sent and stay on the card' : 'will be sent when mobile data is available'}.
          {view.receipts.last_error && <span className="text-ink3"> Last attempt: {view.receipts.last_error}</span>}
        </p>
      )}
    </div>
  )
}

function ProfileRow({
  profile: p,
  blocked,
  writing,
  revision,
  onActivate,
  onDeactivate,
  onLock,
  onRename,
  onDelete,
  onEditing,
}: {
  profile: EsimProfile
  blocked: boolean
  writing: string | null
  revision: number | null
  onActivate: () => void
  onDeactivate: () => void
  onLock: (locked: boolean) => void
  onRename: (name: string, revision: number) => Promise<boolean>
  onDelete: () => void
  onEditing: (editing: boolean) => void
}) {
  const [draft, setDraftState] = useState<string | null>(null)
  const setDraft = (d: string | null) => {
    setDraftState(d)
    onEditing(d !== null)
  }
  const canDelete = deleteBlock(p) === null
  const label = profileLabel(p)
  const active = p.state === 'enabled'
  const why = activateBlock(p)
  const provider = p.provider && p.provider !== p.display_name ? p.provider : null

  return (
    <li
      className={`flex flex-wrap items-center justify-between gap-x-3 gap-y-2 rounded-ctl border px-3 py-2 ${
        active ? 'border-accent/30 bg-accent/4' : 'border-line/8'
      }`}
    >
      <div className="min-w-0 flex-1 basis-52">
        <p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-body font-semibold text-ink">
          <span className="min-w-0 break-words">{label}</span>
          {active && <Chip tone="ok">Active</Chip>}
          {p.locked && <Chip tone="default">Locked</Chip>}
          {p.profile_class !== 'operational' && <Chip tone="default">{p.profile_class}</Chip>}
        </p>
        {/* An unnamed profile already carries its masked id in the label. */}
        {(provider || p.display_name !== null) && (
          <p className="tnum mt-0.5 truncate font-mono text-meta text-ink2">
            {provider ? `${provider}${p.display_name !== null ? ' · ' : ''}` : ''}
            {p.display_name !== null ? p.iccid_masked : ''}
          </p>
        )}
        {!active && why === 'locked' && (
          <p className="mt-1 text-meta text-ink3">Locked against activation. Unlock it to activate.</p>
        )}
        {!active && why === 'not_operational' && (
          <p className="mt-1 text-meta text-ink3">Only operational profiles can be activated here.</p>
        )}
      </div>

      {draft === null ? (
        <div className="flex flex-wrap gap-1.5">
          {active ? (
            <Button
              size="sm"
              variant="outline"
              disabled={blocked}
              loading={writing === `disable:${p.handle}`}
              onClick={onDeactivate}
              aria-label={`Deactivate ${label}`}
            >
              Deactivate
            </Button>
          ) : (
            <Button
              size="sm"
              variant="outline"
              disabled={blocked || why !== null}
              loading={writing === `enable:${p.handle}`}
              onClick={onActivate}
              aria-label={`Activate ${label}`}
            >
              Activate
            </Button>
          )}
          <Button
            size="sm"
            variant="ghost"
            disabled={blocked}
            onClick={() => setDraft(p.nickname ?? '')}
            aria-label={`Rename ${label}`}
          >
            Rename
          </Button>
          {!active && (
            <Button
              size="sm"
              variant="ghost"
              disabled={writing !== null}
              loading={writing === `lock:${p.handle}`}
              aria-pressed={p.locked}
              onClick={() => onLock(!p.locked)}
              aria-label={p.locked ? `Unlock ${label}` : `Lock ${label} against activation`}
            >
              {p.locked ? 'Unlock' : 'Lock'}
            </Button>
          )}
          {canDelete && (
            <Button
              size="sm"
              variant="ghost"
              className="text-danger hover:text-danger"
              disabled={blocked}
              loading={writing === `delete:${p.handle}`}
              onClick={onDelete}
              aria-label={`Delete ${label}`}
            >
              Delete
            </Button>
          )}
        </div>
      ) : (
        <RenameForm
          initial={draft}
          label={label}
          saving={writing === `nickname:${p.handle}`}
          disabled={blocked || revision == null}
          onCancel={() => setDraft(null)}
          onSave={async (name) => {
            if (revision == null) return
            // Keep the form and the draft open when the card did not take it.
            if (await onRename(name, revision)) setDraft(null)
          }}
        />
      )}
    </li>
  )
}

function RenameForm({
  initial,
  label,
  saving,
  disabled,
  onCancel,
  onSave,
}: {
  initial: string
  label: string
  saving: boolean
  disabled: boolean
  onCancel: () => void
  onSave: (name: string) => Promise<void>
}) {
  const [text, setText] = useState(initial)
  const error = nicknameError(text)
  const bytes = nicknameBytes(text)

  return (
    <form
      className="flex w-full flex-wrap items-end gap-2"
      onSubmit={(e) => {
        e.preventDefault()
        if (!error && !disabled) void onSave(text.trim())
      }}
      onKeyDown={(e) => {
        if (e.key === 'Escape') onCancel()
      }}
    >
      <div className="min-w-0 flex-1 basis-48">
        <Field
          label={`Nickname for ${label}`}
          hint={error ? undefined : `${bytes} of ${NICKNAME_MAX_BYTES} bytes. Leave empty to clear it.`}
          error={error ?? undefined}
        >
          {(ids) => (
            <Input
              id={ids.id}
              value={text}
              autoFocus
              spellCheck={false}
              onChange={(e) => setText(e.target.value)}
              aria-describedby={ids.describedBy}
              aria-invalid={ids.invalid || undefined}
            />
          )}
        </Field>
      </div>
      <div className="flex gap-1.5 pb-px">
        <Button type="submit" size="sm" variant="primary" loading={saving} disabled={!!error || disabled}>
          Save
        </Button>
        <Button type="button" size="sm" variant="ghost" onClick={onCancel} disabled={saving}>
          Cancel
        </Button>
      </div>
    </form>
  )
}
