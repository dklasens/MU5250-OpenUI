import { useCallback, useSyncExternalStore } from 'react'

export type Theme = 'light' | 'dark'
export type ThemePref = 'auto' | Theme

const KEY = 'u60.theme'
const mq = window.matchMedia('(prefers-color-scheme: dark)')
const listeners = new Set<() => void>()

function readPref(): ThemePref {
  try {
    const v = localStorage.getItem(KEY)
    return v === 'light' || v === 'dark' ? v : 'auto'
  } catch {
    return 'auto'
  }
}

let pref: ThemePref = readPref()

function resolved(): Theme {
  return pref === 'auto' ? (mq.matches ? 'dark' : 'light') : pref
}

function apply() {
  document.documentElement.setAttribute('data-theme', resolved())
  listeners.forEach((l) => l())
}

// Follow OS changes while the preference is Auto.
mq.addEventListener('change', () => {
  if (pref === 'auto') apply()
})

function setPref(next: ThemePref) {
  pref = next
  try {
    if (next === 'auto') localStorage.removeItem(KEY)
    else localStorage.setItem(KEY, next)
  } catch {
    /* private mode */
  }
  apply()
}

function subscribe(l: () => void) {
  listeners.add(l)
  return () => listeners.delete(l)
}

/** Shared theme state: the header quick-toggle and the Settings picker stay in sync. */
export function useTheme() {
  const theme = useSyncExternalStore(subscribe, resolved)
  const current = useSyncExternalStore(subscribe, () => pref)
  const toggle = useCallback(() => setPref(resolved() === 'dark' ? 'light' : 'dark'), [])
  return { theme, pref: current, setPref, toggle }
}
