/** Brand mark: four ascending signal bars, the tallest in accent. */
export function Mark({ size = 20, className = '' }: { size?: number; className?: string }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" className={className} aria-hidden="true">
      <rect x={2} y={15} width={4} height={7} rx={1} fill="currentColor" />
      <rect x={7.5} y={11} width={4} height={11} rx={1} fill="currentColor" />
      <rect x={13} y={7} width={4} height={15} rx={1} fill="currentColor" />
      <rect x={18.5} y={2} width={4} height={20} rx={1} fill="rgb(var(--accent))" />
    </svg>
  )
}

