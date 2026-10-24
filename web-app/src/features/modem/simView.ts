// SIM Management presentation (design.md § Device-switching lists). Card state and connectivity are
// kept apart: "Active" is the card's selection, never a claim that data is flowing.

import type { EsimOperation, EsimProfile, EsimView, EsimWarning } from '../../types'

/** The card's nickname limit (SGP.22 profileNickname, UTF-8). */
export const NICKNAME_MAX_BYTES = 64

/** Poll the profile list at 30 s; every 3 s only while a change is settling on the card. */
export const IDLE_POLL_MS = 30_000
export const ACTIVE_POLL_MS = 3_000

export function nicknameBytes(text: string): number {
  return new TextEncoder().encode(text).length
}

/** Control characters (Cc) and invisible format characters such as bidi overrides (Cf). */
const HIDDEN_CHARS = /[\p{Cc}\p{Cf}]/u

/** null when the card will accept it; an empty name clears the nickname. */
export function nicknameError(text: string): string | null {
  if (HIDDEN_CHARS.test(text)) return 'Remove control or invisible formatting characters.'
  const bytes = nicknameBytes(text)
  if (bytes > NICKNAME_MAX_BYTES) return `Too long: ${bytes} of ${NICKNAME_MAX_BYTES} bytes.`
  return null
}

/** What a profile is called on screen. The agent already drops names that are identifiers. */
export function profileLabel(p: Pick<EsimProfile, 'display_name' | 'iccid_masked'>): string {
  return p.display_name ?? `Unnamed profile ${p.iccid_masked}`
}

/** Active first, then by name; unnamed profiles last. */
export function sortProfiles(profiles: EsimProfile[]): EsimProfile[] {
  return [...profiles].sort((a, b) => {
    const rank = (p: EsimProfile) => (p.state === 'enabled' ? 0 : 1)
    if (rank(a) !== rank(b)) return rank(a) - rank(b)
    if ((a.display_name === null) !== (b.display_name === null)) return a.display_name === null ? 1 : -1
    return profileLabel(a).localeCompare(profileLabel(b), undefined, { sensitivity: 'base' })
  })
}

export function inFlight(view: EsimView | null): boolean {
  return !!view?.operation && !view.operation.resolved
}

/** Every write control is off while a change is unresolved or the agent says actions are blocked. */
export function writesDisabled(view: EsimView | null): boolean {
  return !view || !view.actions.enabled || inFlight(view)
}

export function pollInterval(view: EsimView | null): number {
  return inFlight(view) ? ACTIVE_POLL_MS : IDLE_POLL_MS
}

export type ActivateBlock = 'locked' | 'not_operational' | null

/** Why Activate is unavailable for an inactive profile, independent of a running change. */
export function activateBlock(p: EsimProfile): ActivateBlock {
  if (p.locked) return 'locked'
  if (p.profile_class !== 'operational') return 'not_operational'
  return null
}

export const WARNING_COPY: Record<EsimWarning, string> = {
  manual_apn:
    'The router is in manual APN mode, so this profile will dial the APN set for the current one. Set the APN mode to automatic, or add this provider’s APN, first.',
  roaming_off:
    'Data roaming is off and this profile is from another country, so it will probably have no data until roaming is turned on.',
}

export type StatusTone = 'info' | 'ok' | 'warn' | 'error'

/** Inline copy for the latest change. Agent details are appended only where they explain a refusal. */
export function operationCopy(op: EsimOperation): { tone: StatusTone; text: string } {
  const who = op.target_name ?? 'the profile'
  switch (op.state) {
    case 'pending':
      return { tone: 'info', text: op.kind === 'delete' ? 'Deleting the profile…' : 'Sending the change to the card…' }
    case 'connecting':
      return { tone: 'info', text: `Contacting the operator’s server: ${stepLabel(op.step)}.` }
    case 'awaiting_confirmation':
      return { tone: 'info', text: 'Waiting for you to review the profile. Nothing has been written to the card yet.' }
    case 'installing':
      return { tone: 'info', text: 'Installing the profile on the card. This takes about 15–30 seconds and can’t be cancelled.' }
    case 'reloading':
      return {
        tone: 'info',
        text: 'The card accepted the change. The modem is reloading the SIM; the list updates once the card can be read again.',
      }
    case 'verifying':
      return { tone: 'info', text: 'Reading the card back to confirm what happened…' }
    case 'succeeded':
      if (op.kind === 'enable')
        return { tone: 'ok', text: `${who} is active on the card. Mobile data reconnects separately and can take a minute.` }
      if (op.kind === 'disable')
        return { tone: 'ok', text: 'No profile is active on the card. Cellular service has stopped.' }
      if (op.kind === 'download')
        return { tone: 'ok', text: `${op.target_name ?? 'The new profile'} is installed on the card, inactive.` }
      if (op.kind === 'delete') return { tone: 'ok', text: 'The profile was deleted from the card.' }
      return { tone: 'ok', text: 'Name saved on the card.' }
    case 'busy':
      return {
        tone: 'warn',
        text:
          'The card was busy and refused the change. Nothing changed; it usually accepts a retry within a few minutes. ' +
          'Right after the last profile was deactivated this can take about 4 minutes.',
      }
    case 'refused':
      return { tone: 'warn', text: op.message ?? 'The card refused the change. Nothing changed.' }
    case 'failed':
      if (op.kind === 'download')
        return { tone: 'error', text: `${op.message ?? 'The download failed.'} ${qrReuseCopy(op)}` }
      return { tone: 'error', text: op.message ?? 'The change failed.' }
    case 'cancelled':
      return { tone: 'warn', text: `${cancelledCopy(op.message)} Nothing was installed. ${qrReuseCopy(op)}` }
    case 'unknown':
      return {
        tone: 'warn',
        text: 'The outcome could not be confirmed: the card may or may not have changed. Refresh in a minute to see its current state.',
      }
  }
}

/** Copy for a failed read while the last good list is still shown. */
export function staleCopy(kind: string | undefined): string {
  switch (kind) {
    case 'sim_reloading':
      return 'The modem is reloading the SIM. Showing the last profile list read.'
    case 'busy':
      return 'The card is busy with another operation. Showing the last profile list read.'
    case 'management_unavailable':
      return 'The modem’s SIM service did not answer. Showing the last profile list read.'
    default:
      return 'The latest card read failed. Showing the last profile list read.'
  }
}

export type EmptyState = { title: string; body: string } | null

/** Whole-tab states that replace the profile list. */
export function emptyState(view: EsimView): EmptyState {
  switch (view.status) {
    case 'unsupported':
      return { title: 'Not an eSIM card', body: 'The SIM in the slot has no eSIM management application.' }
    case 'helper_missing':
      return {
        title: 'eSIM management unavailable',
        body: 'The eSIM helper is missing or incompatible with this agent. Redeploy the agent to restore it.',
      }
    case 'unavailable':
      return { title: 'SIM service not answering', body: view.diagnostic ?? 'The modem’s SIM service did not answer.' }
    case 'detecting':
      if (view.profiles.length === 0) return { title: 'Looking for an eSIM card', body: 'Checking the SIM slot…' }
      return null
    default:
      if (view.card && view.profiles.length === 0)
        return { title: 'No profiles installed', body: 'The card is readable but holds no profiles.' }
      return null
  }
}

export interface ConfirmCopy {
  title: string
  body: string
  details: { label: string; value: string }[]
  consequence: string
  recovery: string
  confirmLabel: string
}

export function activateConfirm(current: EsimProfile | null, target: EsimProfile): ConfirmCopy {
  const from = current ? profileLabel(current) : 'no active profile'
  const warnings = target.warnings.map((w) => WARNING_COPY[w])
  return {
    title: `Activate ${profileLabel(target)}?`,
    body: [`The card switches from ${from} to ${profileLabel(target)}.`, ...warnings].join(' '),
    details: [
      { label: 'Current', value: current ? `${profileLabel(current)} ${current.iccid_masked}` : 'None' },
      { label: 'Requested', value: `${profileLabel(target)} ${target.iccid_masked}` },
    ],
    consequence:
      'Cellular data drops while the modem reloads the SIM and attaches with the new profile. Wi-Fi and this dashboard stay up. Active on the card does not mean connected.',
    recovery: current
      ? `Activate ${profileLabel(current)} again from this tab. Switching works without internet.`
      : 'Activate another profile from this tab. Switching works without internet.',
    confirmLabel: 'Activate',
  }
}

export function deactivateConfirm(target: EsimProfile): ConfirmCopy {
  return {
    title: `Deactivate ${profileLabel(target)}?`,
    body: 'No profile will remain active on the card.',
    details: [{ label: 'Deactivate', value: `${profileLabel(target)} ${target.iccid_masked}` }],
    consequence: 'Cellular service stops completely until a profile is activated again. Wi-Fi and this dashboard stay up.',
    recovery: 'Activate a profile from this tab. It works without internet.',
    confirmLabel: 'Deactivate',
  }
}

/** Write errors the user can act on, in their words. */
export function writeErrorCopy(status: number | undefined, message: string): string {
  if (status === 403) return 'Sign in with the password to switch, lock, add or delete profiles. A PIN session can’t.'
  if (status === 409 && /mobile data/i.test(message))
    return 'The router has no mobile data connection. The download runs over the router’s own data, so activate a profile that has data first.'
  if (status === 423) return 'This profile is locked against activation. Unlock it first.'
  if (status === 409) return `${message.charAt(0).toUpperCase()}${message.slice(1)}.`.replace(/\.\.$/, '.')
  return message
}

// ── Adding a profile (phase 2) ────────────────────────────────────────────────

/** Agent and card limits mirrored client-side; the agent re-checks everything. */
export const ACTIVATION_CODE_MAX = 512
export const CONFIRMATION_CODE_MAX_BYTES = 64

export type CodeCheck =
  | { ok: true; host: string; needsConfirmation: boolean }
  | { ok: false; error: string }

/**
 * Shape check for an SGP.22 activation code, `LPA:1$<server>$<matching ID>[$<OID>[$1]]`.
 * Errors never quote the code: it is a secret.
 */
export function checkActivationCode(raw: string): CodeCheck {
  const code = raw.trim()
  if (!code) return { ok: false, error: 'Paste the activation code, or upload the QR image.' }
  if (code.length > ACTIVATION_CODE_MAX) return { ok: false, error: 'That is too long to be an activation code.' }
  if (HIDDEN_CHARS.test(code)) return { ok: false, error: 'The code contains control or invisible characters.' }
  const fields = code.split('$')
  if (fields[0].toUpperCase() !== 'LPA:1' || fields.length < 3)
    return { ok: false, error: 'An activation code starts with LPA:1$ followed by the server and a matching ID.' }
  const host = fields[1].trim()
  const [name, port] = host.split(':')
  const labels = name.split('.')
  const validHost =
    host.split(':').length <= 2 &&
    labels.length >= 2 &&
    labels.every((l) => /^[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?$/.test(l)) &&
    !/^\d+$/.test(labels[labels.length - 1]) &&
    (port === undefined || (/^\d{1,5}$/.test(port) && Number(port) > 0 && Number(port) < 65536))
  if (!validHost) return { ok: false, error: 'The server in this activation code is not a valid host name.' }
  return { ok: true, host: name.toLowerCase(), needsConfirmation: fields[4] === '1' }
}

export function confirmationCodeError(text: string, required: boolean): string | null {
  if (!text) return required ? 'This eSIM needs the confirmation code from your provider.' : null
  if (HIDDEN_CHARS.test(text)) return 'Remove control or invisible formatting characters.'
  if (new TextEncoder().encode(text).length > CONFIRMATION_CODE_MAX_BYTES) return 'That is too long to be a confirmation code.'
  return null
}

/** The helper's download steps, in order, in plain words. */
export const DOWNLOAD_STEPS: readonly { id: string; label: string }[] = [
  { id: 'es10b_get_euicc_challenge_and_info', label: 'Preparing the card' },
  { id: 'es9p_initiate_authentication', label: 'Contacting the operator’s server' },
  { id: 'es10b_authenticate_server', label: 'Card checking the server' },
  { id: 'es9p_authenticate_client', label: 'Server checking the card' },
  { id: 'es8p_metadata_parse', label: 'Reading the profile details' },
  { id: 'preview', label: 'Waiting for your review' },
  { id: 'es10b_prepare_download', label: 'Preparing the install' },
  { id: 'es9p_get_bound_profile_package', label: 'Downloading the profile' },
  { id: 'es10b_load_bound_profile_package', label: 'Installing on the card' },
]

const EXTRA_STEPS: Record<string, string> = {
  es10b_cancel_session: 'Cancelling on the card',
  es9p_cancel_session: 'Telling the server it was cancelled',
}

export function stepLabel(step: string | null): string {
  if (!step) return 'starting'
  const known = DOWNLOAD_STEPS.find((s) => s.id === step)?.label ?? EXTRA_STEPS[step]
  return (known ?? 'working').replace(/^./, (c) => c.toLowerCase())
}

/** Progress through the steps before the preview: done / current / todo, for the step list. */
export function connectingSteps(step: string | null): { label: string; status: 'done' | 'current' | 'todo' }[] {
  const shown = DOWNLOAD_STEPS.slice(0, DOWNLOAD_STEPS.findIndex((s) => s.id === 'preview'))
  const at = step ? shown.findIndex((s) => s.id === step) : -1
  return shown.map((s, i) => ({ label: s.label, status: i < at ? 'done' : i === at ? 'current' : at < 0 && i === 0 ? 'current' : 'todo' }))
}

function cancelledCopy(message: string | null): string {
  if (message?.includes('not confirmed in time')) return 'The download was cancelled because the profile wasn’t confirmed within 2 minutes.'
  if (message?.includes('declined')) return 'You declined the profile.'
  return 'The download was cancelled.'
}

/**
 * Whether the QR / activation code can probably be used again. A download stopped before the
 * install starts is postponed with the operator; one that failed while installing has usually
 * consumed the code. Anything else is honest uncertainty.
 */
export function qrReuseCopy(op: Pick<EsimOperation, 'state' | 'step' | 'message'>): string {
  if (op.state === 'cancelled' && !op.message?.includes('declined'))
    return 'The QR code is likely still usable.'
  if (op.step === 'es10b_load_bound_profile_package')
    return 'The QR code has likely been used up; you may need a new eSIM from your provider.'
  return 'The QR code may or may not still be usable; your provider can tell you.'
}

/** "1:45" for the preview countdown; null when unknown. */
export function formatCountdown(ms: number | null): string | null {
  if (ms === null) return null
  const total = Math.max(0, Math.ceil(ms / 1000))
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`
}

/** Add eSIM is available only when the agent allows actions and nothing is in flight. */
export function canAddProfile(view: EsimView | null): boolean {
  return !!view && view.status === 'ready' && !writesDisabled(view)
}

// ── Deleting a profile (phase 2) ──────────────────────────────────────────────

export type DeleteBlock = 'active' | 'locked' | null

/** Only inactive, unlocked profiles can be deleted (the agent enforces the same rule). */
export function deleteBlock(p: EsimProfile): DeleteBlock {
  if (p.state !== 'disabled') return 'active'
  if (p.locked) return 'locked'
  return null
}

export function deleteConfirm(p: EsimProfile): ConfirmCopy {
  return {
    title: `Delete ${profileLabel(p)}?`,
    body: 'The profile is removed from the card. This can’t be undone.',
    details: [{ label: 'Delete', value: `${profileLabel(p)} ${p.iccid_masked}` }],
    consequence: 'The active profile and your connection are not affected.',
    recovery:
      'Providers’ QR codes are often single-use, so adding this profile again may need a new eSIM from the provider.',
    confirmLabel: 'Delete',
  }
}

// ── Receipts ──────────────────────────────────────────────────────────────────

/**
 * The card-panel line about a receipt run. The pending count and its last error are already shown
 * by the "from recent changes" line, so this only adds what that line can't: a run in progress, or
 * the outcome of a run that left nothing pending.
 */
export function receiptsCopy(view: EsimView): string | null {
  const r = view.receipts
  if (r.sending) return 'Sending receipts to the operators’ servers…'
  if (r.pending === 0 && r.last_error) return `Some receipts were not accepted. Last attempt: ${r.last_error}`
  return null
}
