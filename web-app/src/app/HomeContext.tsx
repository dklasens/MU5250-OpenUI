/* eslint-disable react-refresh/only-export-components */
import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react'
import { api } from '../data/api'
import { usePoll, type PollResult } from '../data/poll'
import type { HomeData } from '../types'
import { deriveConditions, type AlertConditions } from './alerts'

/**
 * The home poll is the app's heartbeat: one batched request that feeds the
 * Home screen, the Signal group, the Modem data tab, System → Metrics (CPU and
 * memory) and the global alert banner. Those screens read it instead of
 * re-fetching the same data.
 *
 * `fast` is set for the groups that render live radio data. A screen inside
 * another group that shows live heartbeat readings asks for the fast cadence
 * only while it is mounted (`useFastHeartbeat`). Elsewhere the poll only feeds
 * the alert banner, so it idles. Changing the interval does not restart the
 * loop (see `usePoll`), so switching screens costs no extra request.
 */
const HomeContext = createContext<PollResult<HomeData> | null>(null)
const FastHeartbeatContext = createContext<(() => () => void) | null>(null)

const FAST_MS = 3000
const IDLE_MS = 15000

export function HomeProvider({ fast, children }: { fast: boolean; children: ReactNode }) {
  // Number of mounted screens that asked for the fast cadence.
  const [fastScreens, setFastScreens] = useState(0)
  const poll = usePoll('home', api.home, fast || fastScreens > 0 ? FAST_MS : IDLE_MS)
  const retainFast = useCallback(() => {
    setFastScreens((n) => n + 1)
    return () => setFastScreens((n) => n - 1)
  }, [])
  return (
    <FastHeartbeatContext.Provider value={retainFast}>
      <HomeContext.Provider value={poll}>{children}</HomeContext.Provider>
    </FastHeartbeatContext.Provider>
  )
}

export function useHome(): PollResult<HomeData> {
  const ctx = useContext(HomeContext)
  if (!ctx) throw new Error('useHome outside HomeProvider')
  return ctx
}

/** Run the heartbeat at the fast cadence while the calling screen is mounted. */
export function useFastHeartbeat(): void {
  const retainFast = useContext(FastHeartbeatContext)
  if (!retainFast) throw new Error('useFastHeartbeat outside HomeProvider')
  useEffect(() => retainFast(), [retainFast])
}

// ── Alerts derived from the home poll (no extra requests) ─────────────────────

export function useAlertConditions(): AlertConditions {
  const { data, error } = useHome()
  return useMemo(() => deriveConditions(data, error), [data, error])
}
