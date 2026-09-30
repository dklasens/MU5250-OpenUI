export interface TabDef<T extends string> {
  id: T
  label: string
}

/** Horizontally scrollable underline tab strip (design.md § Controls). */
export function Tabs<T extends string>({
  tabs,
  active,
  onChange,
}: {
  tabs: TabDef<T>[]
  active: T
  onChange: (id: T) => void
}) {
  return (
    <div className="no-scrollbar flex gap-5 overflow-x-auto border-b border-line/8" role="tablist">
      {tabs.map((t) => (
        <button
          key={t.id}
          role="tab"
          aria-selected={active === t.id}
          onClick={() => onChange(t.id)}
          className={`-mb-px whitespace-nowrap border-b-2 pb-2 pt-1 text-body font-semibold transition-colors ${
            active === t.id
              ? 'border-accent text-ink'
              : 'border-transparent text-ink3 hover:text-ink'
          }`}
        >
          {t.label}
        </button>
      ))}
    </div>
  )
}
