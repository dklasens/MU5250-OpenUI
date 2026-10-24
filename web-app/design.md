# Design — U60 Pro dashboard

A locked design system for the on-device dashboard. Every screen follows this
file; change the system here first, never per screen. Tokens live in
`src/index.css` (`:root` / `[data-theme='dark']`) and are exposed to Tailwind in
`tailwind.config.js`.

## Brief
- **Audience:** the device owner, technically fluent, usually on a phone next to the hotspot; sometimes a desktop over USB-C.
- **Use:** glance at link health (signal, carriers, throughput, battery), occasionally change radio, Wi-Fi or modem settings.
- **Tone:** technical — an instrument panel, calm and precise. Not a marketing page.

## Genre and theme
- Genre: **modern-minimal**. Theme: **Cobalt**, adapted for an app (no reveals, no code hero, no ⌘K palette).
- One light theme and one graphite dark theme; both follow the OS by default (Auto / Light / Dark in System → Settings).

## Macrostructure (app pages)
- **Readout band + panels.** Home opens with one graphite *readout band* (the page's single dark beat in light mode): large mono readouts separated by hairlines. Everything else is hairline-bordered panels on cool paper.
- Group pages: title (desktop only — the phone header already names the group), one-line subtitle, underline tabs, then panels.
- Tables collapse to stacked rows below `sm`. Nothing scrolls sideways on a phone.

## Colour
All values are OKLCH, stored as sRGB channels for Tailwind's `<alpha-value>`.

| Token | Light | Dark | Role |
|---|---|---|---|
| `--bg` | oklch(97% .005 250) | oklch(16.5% .012 260) | page paper |
| `--surface` | oklch(99.3% .002 250) | oklch(20% .014 260) | panels |
| `--surface-2` | oklch(95.2% .006 252) | oklch(24.5% .016 260) | wells, hovers, tracks |
| `--line` | ink | oklch(93% .01 255) | hairlines, used at /6–/15 |
| `--text` | oklch(24% .02 258) | oklch(94% .008 255) | ink |
| `--text-2` | oklch(42% .018 257) | oklch(76% .014 257) | secondary |
| `--text-3` | oklch(52% .014 257) | oklch(64% .014 257) | labels, meta (≥ 5:1) |
| `--accent` | oklch(55% .20 258) | oklch(70% .16 256) | the one cobalt signal; LTE |
| `--nr` | oklch(52% .20 305) | oklch(72% .15 305) | 5G NR only |
| `--ok` / `--warn` / `--danger` | status, never decoration |
| `--band*` | graphite readout band + its ink and accent |

Rules: accent covers < 5 % of any view — active nav, primary button, focus ring, LTE marks. No gradients, glass, glows or drop shadows beyond `shadow-sm` on the active segmented option. Pure `#fff` / `#000` are banned.

## Typography
- **Display:** Space Grotesk (variable, Latin subset, self-hosted) — page titles, panel titles, readout units. Weight 600, tracking −0.01em. Always roman.
- **Body:** system UI stack — zero bytes, native on every phone.
- **Mono:** JetBrains Mono (variable, Latin subset, self-hosted) — every measured value (dBm, Mbps, %, GB), identifiers (IP, MAC, PCI, ARFCN), and *labels*.
- **Labels:** mono, UPPERCASE, 11 px, `0.06em` tracking, `--text-3` — the machine-readout voice. Only on metric/field/table-column labels, never above section headings.
- **Scale:** `caption` 11 px · `meta` 12 px · `body` 13 px · `sm` 14 px · `xl`/`2xl`/`4xl` for titles and readouts. No arbitrary pixel sizes; nothing below 11 px.

## Shape and space
- Radii: panels 10 px (`rounded-panel`), controls 6 px (`rounded-ctl`), chips 4 px (`rounded-chip`).
- Hairlines do the work: 1 px `line/8` borders; dividers `line/6`.
- Spacing: Tailwind's 4-pt scale; panels `p-4`, gaps `3`.

## Controls
- **Primary button:** solid accent, 6 px, one per view. Others: hairline outline or ghost. Danger is solid danger and always behind a confirm.
- **Tabs:** underline tabs — ink label + 2 px accent underline when active. When a group's tabs don't fit, **whole tabs wrap onto another row**; a label never breaks and the strip never scrolls sideways. Semantics in § Selection.
- **Nav:** desktop sidebar with a hairline edge; phones get a bottom bar whose active item shows a 2 px accent rule on top.
- Clickable labels never wrap (`whitespace-nowrap`).

## Motion and microinteractions
- No entrance or scroll animation. Colour transitions only (`transition-colors`), meters animate width.
- Focus ring: see § Focus. Instant, never animated.
- Toasts only for async device actions whose effect isn't visible; confirms only for irreversible or connection-dropping actions (see § Feedback and § Dialogs).
- Respect `prefers-reduced-motion`: spinners and meters stop animating.

## Focus
One treatment for every focusable element, set once in `src/index.css` (`:focus-visible`): **2 px solid `--accent`, 2 px offset, instant, no layout shift.** Fields keep their `focus:border-accent` but never `outline-none`.
- The 2 px offset is part of the rule. Accent against its own fill is 1.0:1, so the gap — which shows the parent surface — is what separates ring from a filled primary/danger button. Never use zero offset on a filled control.
- Unfilled controls inside an `overflow` container (tab strip, bottom nav) use `.focus-inset` (offset −2 px) so the ring is not clipped.
- Inside `.band` the tokens are re-scoped, so the ring becomes `--band-accent` automatically. Do not hard-code a ring colour.
- Compound controls (switch) draw the ring on the visible track, not on the larger touch hit area.

Contrast of the ring against what it touches (WCAG relative luminance of the stored sRGB channels; need ≥ 3:1):

| Ring on | Light | Dark |
|---|---|---|
| `--bg` | 4.55 | 7.17 |
| `--surface` | 4.88 | 6.74 |
| `--surface-2` | 4.31 | 6.01 |
| graphite readout band (`--band-accent` on `--band`) | 6.42 | 5.94 |
| filled accent / danger button (ring sits in the 2 px gap on the parent surface; against the fill itself 1.0 / 1.07 light, 1.0 / 1.06 dark) | via gap | via gap |

The old 55 %-opacity ring measured 2.2–2.9:1 and is gone. `--text` as ring colour was rejected: 2.25:1 against the dark accent button and 1.05:1 on the light-mode band.

## Touch targets
Under `(pointer: coarse)` every interactive control has a hit area of **≥ 44 × 44 CSS px** (Tailwind variant `coarse:`): buttons (both sizes), tabs, segmented items, toggle chips, switches, nav items, icon-only buttons, toast/alert dismiss. Fine pointers keep the compact sizes (button 32/36 px, switch 24 × 40).
- Achieve it with real box size (`coarse:min-h-11 coarse:min-w-11`), or a wrapper button that contains the compact visual (the switch track stays 24 × 40 inside a 44 × 44 button). No invisible overlapping padding: neighbours keep their own hit areas, with a gap between them.
- Labels stay on one line; nothing may add horizontal overflow at 320 px.
- Icon-only controls always carry an `aria-label`.

## Dialogs
- One shared host (`confirm()` in `ui/feedback.tsx`) built on native `<dialog>` + `showModal()`. No per-screen modal copies. Background is inert; Tab and Shift+Tab wrap inside.
- `aria-labelledby` = title, `aria-describedby` = the body/details/consequence/recovery block.
- Initial focus is **Cancel** for `kind: 'danger' | 'connection'`, Confirm for `default`. Escape, Cancel and backdrop click resolve `false`; a click inside never dismisses; Confirm resolves `true` once.
- Layout, top to bottom: title; plain body; compact mono **details** list (operation, affected radio / profile / cell — never secrets); **consequence** (what will be interrupted, and whether that is Internet or dashboard access); **recovery** (the practical way back).
- Only one confirmation at a time. A second `confirm()` while one is open resolves `false` immediately; it never replaces the open one.
- Focus returns to the opener if it is still in the document, otherwise to `main`.
- The reviewed payload is frozen by the caller before `confirm()`; the dialog never reads live state.

## Selection
- **Tabs** (`Tabs` + `TabPanel`): named `tablist`, `aria-selected`, `aria-controls` / `aria-labelledby`, one tab stop, Left/Right wrap, Home/End, **manual activation** (arrows move focus, Enter/Space activates). Only the active panel is mounted. Use tab semantics only where a tabpanel exists, not for navigation.
- **Exclusive choice** (`Segmented`): named `radiogroup` of `role="radio"` items. By default arrows move selection and call `onChange`, like native radios, so a device-changing choice must be a **draft + Apply** (or separate action buttons with a confirm). `activation="manual"` keeps arrows as focus-only.
- **Multi-select** (`ToggleChip`): `aria-pressed` buttons. Never use fill colour as the only state — pressed adds a border and weight change, and the disabled state is a real `disabled`.
- **Switch** (`Toggle`): `role="switch"`; an accessible name is mandatory (`label` or `labelledBy`).

## Fields
- Every field has a persistent visible label tied to its control (`Field` wrapping, or `htmlFor` / `useFieldIds()` for ranges and number inputs). A placeholder or adjacent `<p>` is not a label.
- Hints and errors are linked with `aria-describedby`; errors use `text-danger`, `aria-invalid`, and are not `role="alert"` (the live-region policy below handles announcement).
- Units belong in the label or hint ("Timeout (seconds)").

## Feedback
- A persistent, mounted live-region host: a polite `role="status"` for success, an assertive `role="alert"` for errors. Both exist empty at load so additions are announced.
- **Errors persist** until dismissed (named dismiss button). Success toasts auto-dismiss after ~5 s. Identical consecutive messages collapse into one with a count, and do not re-announce.
- Toast only when the result is not otherwise visible. No success toast when the UI visibly shows it (a row disappearing after delete, a value updating).
- Reads that failed, stale data, and "accepted, verifying" states are **inline**, not toasts: `InlineStatus` (info / ok / warn / error / stale, optional retry). Do not announce the same fact through both an inline status and a toast.
- Empty values render `—` with a screen-reader label (`Unavailable`); loading uses `Skeleton` (decorative) inside a `Loading` region with an sr-only label.

## Signal ratings
- One policy, defined once in `src/data/signalQuality.ts`; classifiers, legends, Home and the Signal tables all derive from it. No thresholds in JSX.
- Boundaries are inclusive at the lower edge and identical for LTE and NR:

  | Metric | Excellent | Good | Fair | Poor |
  |---|---|---|---|---|
  | RSRP, dBm | ≥ −80 | −90 to −80 | −100 to −90 | < −100 |
  | RSRQ, dB | ≥ −10 | −15 to −10 | −20 to −15 | < −20 |
  | SINR, dB | ≥ 20 | 10 to 20 | 0 to 10 | < 0 |

- Excellent and Good use `--ok`, Fair `--warn`, Poor `--danger`. RSSI is never rated (it mixes signal, interference and noise). Unknown or unmeasured values are neutral `--text-3` with "Unavailable"; SINR 0 is a real reading.
- Every rating shows its word as well as its colour. Ratings are approximate link indicators, not throughput predictions, and the legend says so.
- Raw readings keep the firmware's value and units; a rating never alters or hides one.

## Device-switching lists (SIM Management)
The pattern for lists whose actions change what the modem is attached to (eSIM profiles; reuse it for anything similar).
- **Two panels:** a compact facts panel (only what the device reports, mono values, `Unavailable` for missing ones, no derived capacity or percentages) with Refresh in its header, then the list panel.
- **Rows on desktop, stacked on phones** with the same `flex-wrap` row as APN profiles: name + chips on the left, actions on the right, wrapping under the name at narrow widths. The selected item comes first and gets the `accent/30` border and `accent/4` wash.
- **Selection is not connectivity.** The `Active` chip (`ok`) means selected on the device; a muted line says where connectivity is shown. Never word it as "Connected".
- **Actions are outline buttons, not primary.** A switch that drops the connection always goes through `confirm({kind: 'connection'})` (Cancel focused) naming the current and requested item, the interruption, read-only warnings in plain words, and the way back. Inline rename uses a labelled field with a live byte count, Save (primary, only while the form is open) and Cancel; Escape cancels.
- **One change at a time.** While a change is unresolved every write control is disabled and the change's state shows as an `InlineStatus` above the list (info while in flight, ok/warn/error once resolved, with Dismiss). A failed read keeps the last good list under a `stale` note. Never toast these.
- **Unvalidated features say so quietly**: one muted `text-meta text-ink3` sentence, not a warning box.
- **Adding an item is an inline, stepped panel** at the top of the list panel, never a modal: the list stays visible underneath. The panel's single submit is the view's one primary (Add in the panel header steps down to outline while a row's rename form is open, and disappears while the panel is open). Steps replace each other in place: input → progress (a short ordered step list, `aria-current="step"` on the current one) → review (mono-labelled facts from the device or server, an optional field, the commit as primary and Cancel as ghost, any time limit shown as a mono countdown) → result (an `InlineStatus` with the next action as an outline button, never automatic). Steps that cannot be cancelled say so instead of showing Cancel.
- **Secrets in fields** (activation and confirmation codes): `type="password"` with a Show/Hide ghost toggle (`aria-pressed`), held only in component state and cleared once the device accepts them. Errors and hints describe the problem without quoting the value; a hint may name a non-secret part (for example the server host).
- **Local-only file input** for decoding (a QR image): an outline button opens a visually hidden file input; the copy says the file is read in the browser and not uploaded. Decoders load on demand in their own chunk, never in the shared vendor chunk.
- **Destructive row actions** (Delete): a ghost button in `text-danger`, shown only on rows where the action is allowed (never on the selected item or a locked one), always behind `confirm({kind: 'danger'})` with Cancel focused, a body that says it can't be undone, and a recovery line that says honestly what getting it back would take.
- **On-request maintenance actions** (Send receipts): an outline button beside the fact it acts on, with one muted sentence saying exactly what it does; its progress and outcome appear as a `role="status"` line under the facts, never a toast.

## What every screen must share
The mark, the three faces, the token palette, the 10/6/4 radii, underline tabs, mono labels, and the rule that numbers are mono.
