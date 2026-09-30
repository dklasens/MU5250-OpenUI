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
- **Tabs:** underline tabs — ink label + 2 px accent underline when active.
- **Nav:** desktop sidebar with a hairline edge; phones get a bottom bar whose active item shows a 2 px accent rule on top.
- Clickable labels never wrap (`whitespace-nowrap`).

## Motion and microinteractions
- No entrance or scroll animation. Colour transitions only (`transition-colors`), meters animate width.
- Focus ring: 2 px accent, instant.
- Toasts only for async device actions whose effect isn't visible; confirms only for irreversible or connection-dropping actions.
- Respect `prefers-reduced-motion`: spinners and meters stop animating.

## What every screen must share
The mark, the three faces, the token palette, the 10/6/4 radii, underline tabs, mono labels, and the rule that numbers are mono.
