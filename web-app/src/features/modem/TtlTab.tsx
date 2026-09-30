import { useCallback, useEffect, useState } from 'react'
import { api } from '../../data/api'
import type { TtlStatus } from '../../types'
import { Button, Input } from '../../ui/controls'
import { toast, toastError } from '../../ui/feedback'
import { Card, Chip } from '../../ui/primitives'

export default function TtlTab() {
  const [status, setStatus] = useState<TtlStatus | null>(null)
  const [ttlInput, setTtlInput] = useState('65')
  const [busy, setBusy] = useState(false)

  const fetchStatus = useCallback(async () => {
    try {
      const data = await api.ttlStatus()
      setStatus(data)
      if (data.ttl_value && data.ttl_value > 0) setTtlInput(String(data.ttl_value))
    } catch {
      setStatus(null)
    }
  }, [])

  useEffect(() => {
    fetchStatus()
  }, [fetchStatus])

  const active = Boolean(status?.active || status?.ipv6_active)

  async function applyTtl() {
    const val = parseInt(ttlInput)
    if (!val || val < 1 || val > 255) {
      toast('TTL must be 1-255', 'err')
      return
    }
    setBusy(true)
    try {
      await api.ttlSet(val)
      toast(`TTL set to ${val} (IPv4 + IPv6)`)
      await fetchStatus()
    } catch (e) {
      toastError(e, 'Failed to set TTL')
    } finally {
      setBusy(false)
    }
  }

  async function clearTtl() {
    setBusy(true)
    try {
      await api.ttlClear()
      toast('TTL clamping disabled')
      await fetchStatus()
    } catch (e) {
      toastError(e, 'Failed to clear TTL')
    } finally {
      setBusy(false)
    }
  }

  return (
    <Card title="TTL clamping">
      <div className="space-y-3">
        <p className="text-meta text-ink2">
          Overrides the TTL / hop limit on LAN ingress traffic to prevent carrier tethering detection.
          Applied immediately and persists across reboots.
        </p>

        {status == null ? (
          <p className="text-body text-ink3">Checking status…</p>
        ) : active ? (
          <div className="flex flex-wrap items-center gap-x-4 gap-y-3">
            <div className="flex items-center gap-2">
              <span className="h-2 w-2 rounded-full bg-ok" />
              <span className="tnum font-mono text-body font-semibold text-ok">Active (TTL={status.ttl_value})</span>
              {status.ipv6_active && <Chip tone="default">IPv4 + IPv6</Chip>}
            </div>
            <div className="flex items-center gap-2">
              <div className="w-20">
                <Input type="number" min={1} max={255} value={ttlInput} onChange={(e) => setTtlInput(e.target.value)} />
              </div>
              <Button variant="outline" onClick={applyTtl} loading={busy}>
                Update
              </Button>
              <Button variant="ghost" onClick={clearTtl} disabled={busy}>
                Disable
              </Button>
            </div>
          </div>
        ) : (
          <div className="flex flex-wrap items-center gap-2">
            <div className="w-24">
              <Input type="number" min={1} max={255} value={ttlInput} onChange={(e) => setTtlInput(e.target.value)} placeholder="65" />
            </div>
            <Button variant="primary" onClick={applyTtl} loading={busy} disabled={!ttlInput}>
              Enable clamping
            </Button>
          </div>
        )}
      </div>
    </Card>
  )
}
