/* eslint-disable react-refresh/only-export-components */
import { useId, useRef, useState, type KeyboardEvent, type ReactNode } from 'react'
import { rovingTarget } from './roving'

export interface TabDef<T extends string> {
  id: T
  label: string
}

/** DOM ids shared by a tab and its panel. Pass the same `idBase` to `Tabs` and `TabPanel`. */
export function tabIds(idBase: string, id: string) {
  return { tab: `${idBase}-tab-${id}`, panel: `${idBase}-panel-${id}` }
}

/**
 * Underline tab strip (design.md § Controls, § Selection). When the tabs do not fit, whole tabs wrap
 * onto another row; a label never breaks and the strip never scrolls sideways.
 *
 * Pattern: WAI-ARIA tabs with **manual activation** and a roving tab stop. Left/Right (wrapping),
 * Home and End move focus only; Enter/Space activates. Pair every tab with a `TabPanel` using the
 * same `idBase` and render only the active panel's content.
 *
 * `label` names the tablist (e.g. the group: "Signal sections"). `idBase` defaults to a generated id;
 * pass an explicit one when a test or another component needs stable ids.
 */
export function Tabs<T extends string>({
  tabs,
  active,
  onChange,
  label,
  idBase,
}: {
  tabs: TabDef<T>[]
  active: T
  onChange: (id: T) => void
  /** Accessible name of the tablist. */
  label?: string
  idBase?: string
}) {
  const generated = useId()
  const base = idBase ?? generated
  const refs = useRef(new Map<string, HTMLButtonElement>())

  // Which tab owns the single tab stop. Follows the selection, and follows focus while the user
  // arrows around (manual activation). Reset when the selection changes or the tab set changes.
  const [focusId, setFocusId] = useState<string>(active)
  const [lastActive, setLastActive] = useState<string>(active)
  if (lastActive !== active) {
    setLastActive(active)
    setFocusId(active)
  }
  const stopId = tabs.some((t) => t.id === focusId) ? focusId : tabs.some((t) => t.id === active) ? active : tabs[0]?.id

  function onKeyDown(e: KeyboardEvent<HTMLDivElement>) {
    if (e.altKey || e.ctrlKey || e.metaKey) return
    if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(e.key)) return
    const i = tabs.findIndex((t) => t.id === stopId)
    const next = rovingTarget(e.key, Math.max(i, 0), tabs.length)
    if (next == null) return
    e.preventDefault()
    const target = tabs[next]
    setFocusId(target.id)
    refs.current.get(target.id)?.focus()
  }

  return (
    <div
      className="flex flex-wrap gap-x-5 border-b border-line/8"
      role="tablist"
      aria-label={label}
      onKeyDown={onKeyDown}
      onBlur={(e) => {
        // Leaving the strip: next Tab stop re-enters on the selected tab.
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setFocusId(active)
      }}
    >
      {tabs.map((t) => {
        const ids = tabIds(base, t.id)
        const selected = active === t.id
        return (
          <button
            key={t.id}
            ref={(el) => {
              if (el) refs.current.set(t.id, el)
              else refs.current.delete(t.id)
            }}
            type="button"
            role="tab"
            id={ids.tab}
            aria-selected={selected}
            aria-controls={ids.panel}
            tabIndex={t.id === stopId ? 0 : -1}
            onFocus={() => setFocusId(t.id)}
            onClick={() => onChange(t.id)}
            className={`focus-inset -mb-px inline-flex shrink-0 items-center justify-center whitespace-nowrap border-b-2 pb-2 pt-1 text-body font-semibold transition-colors coarse:min-h-11 coarse:min-w-11 coarse:py-0 ${
              selected ? 'border-accent text-ink' : 'border-transparent text-ink3 hover:text-ink'
            }`}
          >
            {t.label}
          </button>
        )
      })}
    </div>
  )
}

/**
 * Panel for one tab. Render it only for the active tab (inactive panels must not stay in the DOM
 * with focusable content). `tabIndex={0}` lets keyboard users reach panels with no focusable child.
 */
export function TabPanel({
  idBase,
  id,
  children,
  className,
}: {
  /** Same `idBase` given to `Tabs`. */
  idBase: string
  /** The tab id this panel belongs to. */
  id: string
  children: ReactNode
  className?: string
}) {
  const ids = tabIds(idBase, id)
  return (
    <div role="tabpanel" id={ids.panel} aria-labelledby={ids.tab} tabIndex={0} className={className}>
      {children}
    </div>
  )
}
