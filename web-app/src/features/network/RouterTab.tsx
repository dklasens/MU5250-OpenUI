import { useEffect, useState } from 'react'
import { api } from '../../data/api'
import { confirmLan } from '../../data/client'
import type { DnsConfig, LanConfig } from '../../types'
import { Button, Field, Input, Toggle } from '../../ui/controls'
import { toast, toastError } from '../../ui/feedback'
import { Card } from '../../ui/primitives'

const DNS_PRESETS: { label: string; v: DnsConfig }[] = [
  {
    label: 'Cloudflare',
    v: { primary: '1.1.1.1', secondary: '1.0.0.1', ipv6_primary: '2606:4700:4700::1111', ipv6_secondary: '2606:4700:4700::1001' },
  },
  {
    label: 'Google',
    v: { primary: '8.8.8.8', secondary: '8.8.4.4', ipv6_primary: '2001:4860:4860::8888', ipv6_secondary: '2001:4860:4860::8844' },
  },
  {
    label: 'Quad9',
    v: { primary: '9.9.9.9', secondary: '149.112.112.112', ipv6_primary: '2620:fe::fe', ipv6_secondary: '2620:fe::9' },
  },
]

function DnsSection() {
  const [dns, setDns] = useState<DnsConfig>({ primary: '', secondary: '' })
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    api.dnsGet().then(setDns).catch(() => {})
  }, [])

  async function save() {
    setBusy(true)
    try {
      await api.dnsSet({
        dns_mode: 'manual',
        prefer_dns_manual: dns.primary,
        standby_dns_manual: dns.secondary,
        ...(dns.ipv6_primary ? { ipv6_wan_prefer_dns_manual: dns.ipv6_primary } : {}),
        ...(dns.ipv6_secondary ? { ipv6_wan_standby_dns_manual: dns.ipv6_secondary } : {}),
      })
      toast('DNS settings saved')
    } catch (e) {
      toastError(e, 'Failed to save DNS')
    } finally {
      setBusy(false)
    }
  }

  return (
    <Card title="DNS servers">
      <div className="grid grid-cols-1 gap-2.5 lg:grid-cols-2">
        <Field label="Primary DNS (IPv4)">
          <Input value={dns.primary} onChange={(e) => setDns((d) => ({ ...d, primary: e.target.value }))} placeholder="1.1.1.1" inputMode="numeric" />
        </Field>
        <Field label="Secondary DNS (IPv4)">
          <Input value={dns.secondary} onChange={(e) => setDns((d) => ({ ...d, secondary: e.target.value }))} placeholder="1.0.0.1" inputMode="numeric" />
        </Field>
        <Field label="Primary DNS (IPv6)">
          <Input value={dns.ipv6_primary ?? ''} onChange={(e) => setDns((d) => ({ ...d, ipv6_primary: e.target.value }))} placeholder="2606:4700:4700::1111" />
        </Field>
        <Field label="Secondary DNS (IPv6)">
          <Input value={dns.ipv6_secondary ?? ''} onChange={(e) => setDns((d) => ({ ...d, ipv6_secondary: e.target.value }))} placeholder="2001:4860:4860::8888" />
        </Field>
      </div>
      <div className="mt-3.5 flex flex-wrap items-center gap-2">
        <Button variant="primary" onClick={save} loading={busy}>
          Apply
        </Button>
        <div className="flex gap-1.5">
          {DNS_PRESETS.map((p) => (
            <Button key={p.label} variant="ghost" size="sm" onClick={() => setDns(p.v)}>
              {p.label}
            </Button>
          ))}
        </div>
      </div>
    </Card>
  )
}

function LanSection() {
  const [lan, setLan] = useState<LanConfig>({
    ipaddr: '',
    netmask: '',
    dhcp_enabled: true,
    dhcp_start: '',
    dhcp_end: '',
    lease_seconds: 86400,
  })
  const [busy, setBusy] = useState(false)
  const [transition, setTransition] = useState('')

  useEffect(() => {
    api.lanGet().then(setLan).catch(() => {})
  }, [])

  async function save() {
    setBusy(true)
    try {
      const result = await api.lanSet({
        ipaddr: lan.ipaddr,
        netmask: lan.netmask,
        dhcp_enabled: lan.dhcp_enabled,
        dhcp_start: lan.dhcp_start,
        dhcp_end: lan.dhcp_end,
        lease_seconds: lan.lease_seconds,
      })
      if (result.changed) {
        if (result.reconnect_ip !== lan.ipaddr || typeof result.confirmation_token !== 'string') {
          throw new Error('Invalid LAN transition response; previous settings will be restored automatically')
        }
        setTransition(`Reconnecting to ${lan.ipaddr}. Rejoin Wi-Fi if needed. Previous settings return automatically if confirmation fails.`)
        const deadline = Date.now() + 90_000
        let confirmed = false
        while (Date.now() < deadline) {
          await new Promise(resolve => setTimeout(resolve, 2000))
          try {
            await confirmLan(lan.ipaddr, result.confirmation_token)
            confirmed = true
            break
          } catch { /* The address may still be changing; retry within the recovery window. */ }
        }
        if (!confirmed) {
          throw new Error('Could not confirm the new address. Wait up to two minutes from Apply for the previous LAN settings to return, then reconnect.')
        }
        setTransition('LAN settings confirmed. Opening the dashboard at its new address…')
        if (window.location.hostname !== lan.ipaddr) {
          const next = new URL(window.location.href)
          next.hostname = lan.ipaddr
          // Session storage belongs to the old origin; sign in again at the new address.
          window.location.assign(next.toString())
        }
      }
      toast('LAN settings saved and confirmed')
    } catch (e) {
      setTransition(e instanceof Error ? e.message : 'LAN change failed')
      toastError(e, 'Failed to save LAN settings')
    } finally {
      setBusy(false)
    }
  }

  return (
    <Card title="LAN / DHCP">
      <p className="mb-3 text-xs text-ink3" role="status">{transition || 'Changes must reconnect and confirm within two minutes; otherwise the previous settings are restored.'}</p>
      <div className="grid grid-cols-1 gap-2.5 lg:grid-cols-2">
        <Field label="LAN IP">
          <Input value={lan.ipaddr} onChange={(e) => setLan((l) => ({ ...l, ipaddr: e.target.value }))} inputMode="numeric" />
        </Field>
        <Field label="Netmask">
          <Input value={lan.netmask} onChange={(e) => setLan((l) => ({ ...l, netmask: e.target.value }))} inputMode="numeric" />
        </Field>
        <Field label="DHCP start">
          <Input disabled={!lan.dhcp_enabled} value={lan.dhcp_start} onChange={(e) => setLan((l) => ({ ...l, dhcp_start: e.target.value }))} inputMode="numeric" />
        </Field>
        <Field label="DHCP end">
          <Input disabled={!lan.dhcp_enabled} value={lan.dhcp_end} onChange={(e) => setLan((l) => ({ ...l, dhcp_end: e.target.value }))} inputMode="numeric" />
        </Field>
        <Field label="Lease time (hours)" hint="The firmware stores this value in seconds.">
          <Input
            type="number"
            min={1}
            max={168}
            disabled={!lan.dhcp_enabled}
            value={lan.lease_seconds / 3600}
            onChange={(e) => setLan((l) => ({ ...l, lease_seconds: Math.round(Number(e.target.value) * 3600) }))}
            inputMode="numeric"
          />
        </Field>
      </div>
      <div className="mt-3 flex items-center justify-between rounded-ctl bg-surface2/60 px-3 py-2.5">
        <div>
          <p className="text-body font-semibold text-ink">DHCP server</p>
          <p className="text-caption text-ink3">Assign addresses to LAN and Wi-Fi clients</p>
        </div>
        <Toggle checked={lan.dhcp_enabled} onChange={(dhcp_enabled) => setLan((l) => ({ ...l, dhcp_enabled }))} label="DHCP server" />
      </div>
      <div className="mt-3.5">
        <Button variant="primary" onClick={save} loading={busy}>
          Apply
        </Button>
      </div>
    </Card>
  )
}

export default function RouterTab() {
  return (
    <div className="space-y-3">
      <LanSection />
      <DnsSection />
    </div>
  )
}
