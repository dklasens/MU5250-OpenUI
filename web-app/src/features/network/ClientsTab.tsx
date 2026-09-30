import { api } from '../../data/api'
import { usePoll } from '../../data/poll'
import type { Client, UsbStatus } from '../../types'
import { ICable, ILaptop, IRefresh, IUsb, IWifi } from '../../icons'
import { Button } from '../../ui/controls'
import { Card, Chip, Empty, Skeleton } from '../../ui/primitives'

function formatLinkMbps(value?: number) {
  if (value == null || value <= 0) return '\u2014'
  return `${Math.round(value)} Mbps`
}

function formatBitrate(mbps?: number) {
  if (mbps == null || mbps <= 0) return null
  return mbps >= 1000 ? `${mbps / 1000} Gbit/s` : `${mbps} Mbit/s`
}

function formatWifiLink(client: Client) {
  const parts: string[] = []
  if (client.tx_bitrate_mbps != null) parts.push(`TX ${client.tx_bitrate_mbps.toFixed(0)}`)
  if (client.rx_bitrate_mbps != null) parts.push(`RX ${client.rx_bitrate_mbps.toFixed(0)}`)
  return parts.length > 0 ? `${parts.join(' / ')} Mbps` : '\u2014'
}

function groupClients(clients: Client[]) {
  return {
    wifi: clients.filter((c) => c.medium === 'wifi'),
    usb: clients.filter((c) => c.medium === 'usb-c'),
    ethernet: clients.filter((c) => c.medium === 'ethernet'),
    other: clients.filter((c) => !c.medium || c.medium === 'wired'),
  }
}

const TH_CLS = 'pb-1.5 pr-4 font-semibold'
const TD_CLS = 'py-2 pr-4'

export default function ClientsTab() {
  // Clients is an expensive endpoint (iw station dump + bridge fdb + arp) —
  // poll slowly and offer a manual refresh instead.
  const { data, refreshing, refresh } = usePoll(
    'clients',
    async () => {
      const [clients, usb] = await Promise.all([api.clients(), api.usbStatus().catch(() => null)])
      return { clients: clients ?? [], usb }
    },
    15000,
  )

  if (!data) {
    return (
      <div className="space-y-3">
        <Skeleton className="h-20" />
        <Skeleton className="h-56" />
      </div>
    )
  }

  const { clients, usb } = data as { clients: Client[]; usb: UsbStatus | null }
  const grouped = groupClients(clients)
  const usbLink = usb?.link
  const usbNegotiatedRate = formatBitrate(usbLink?.negotiated_mbps)
  const usbMaxRate = formatBitrate(usbLink?.max_mbps)

  return (
    <div className="space-y-3">
      <Card
        title={`Connected clients (${clients.length})`}
        action={
          <Button size="sm" variant="ghost" onClick={refresh} loading={refreshing}>
            <IRefresh size={13} /> Refresh
          </Button>
        }
      >
        {clients.length === 0 ? (
          <Empty icon={<ILaptop size={28} />} title="No clients connected" />
        ) : (
          <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
            {[
              { label: 'Wi-Fi', count: grouped.wifi.length, icon: <IWifi size={15} /> },
              { label: 'USB-C', count: grouped.usb.length, icon: <IUsb size={15} /> },
              { label: 'Ethernet', count: grouped.ethernet.length, icon: <ICable size={15} /> },
              { label: 'Other', count: grouped.other.length, icon: <ILaptop size={15} /> },
            ].map((g) => (
              <div key={g.label} className="rounded-ctl bg-surface2/70 px-3 py-2.5">
                <div className="flex items-center gap-1.5 text-ink3">
                  {g.icon}
                  <p className="label">{g.label}</p>
                </div>
                <p className="tnum font-mono mt-1 text-2xl font-medium text-ink">{g.count}</p>
              </div>
            ))}
          </div>
        )}
      </Card>

      {grouped.wifi.length > 0 && (
        <Card title={`Wi-Fi (${grouped.wifi.length})`} pad={false}>
          {/* Mobile: one row per client instead of a six-column table */}
          <ul className="divide-y divide-line/6 px-4 sm:hidden">
            {grouped.wifi.map((c) => (
              <li key={c.mac} className="py-2.5">
                <div className="flex items-center justify-between gap-2">
                  <span className="min-w-0 truncate text-body font-medium text-ink">{c.hostname || '—'}</span>
                  <Chip tone={c.wifi_band === '5 GHz' ? 'accent' : 'ok'}>{c.wifi_band ?? 'Wi-Fi'}</Chip>
                </div>
                <div className="tnum font-mono mt-1 flex flex-wrap gap-x-3 gap-y-0.5 text-meta text-ink2">
                  <span className="font-mono">{c.ip ?? '—'}</span>
                  <span>{c.signal_dbm != null ? `${c.signal_dbm} dBm` : '—'}</span>
                  <span>{formatWifiLink(c)}</span>
                </div>
                <p className="tnum mt-0.5 font-mono text-caption text-ink3">{c.mac}</p>
              </li>
            ))}
          </ul>
          <div className="hidden overflow-x-auto px-4 pb-3 sm:block">
            <table className="w-full text-body">
              <thead>
                <tr className="label border-b border-line/8 text-left">
                  <th className={TH_CLS}>Hostname</th>
                  <th className={TH_CLS}>IP</th>
                  <th className={TH_CLS}>Radio</th>
                  <th className={TH_CLS}>Signal</th>
                  <th className={TH_CLS}>Link</th>
                  <th className="pb-1.5 font-semibold">MAC</th>
                </tr>
              </thead>
              <tbody>
                {grouped.wifi.map((c) => (
                  <tr key={c.mac} className="border-b border-line/6 last:border-0">
                    <td className={`${TD_CLS} font-medium text-ink`}>{c.hostname || '\u2014'}</td>
                    <td className={`${TD_CLS} tnum font-mono text-meta text-ink2`}>{c.ip ?? '\u2014'}</td>
                    <td className={TD_CLS}>
                      <Chip tone={c.wifi_band === '5 GHz' ? 'accent' : 'ok'}>{c.wifi_band ?? 'Wi-Fi'}</Chip>
                    </td>
                    <td className={`${TD_CLS} tnum font-mono text-ink2`}>
                      {c.signal_dbm != null ? `${c.signal_dbm} dBm` : '\u2014'}
                    </td>
                    <td className={`${TD_CLS} tnum font-mono text-ink2`}>{formatWifiLink(c)}</td>
                    <td className="tnum py-2 font-mono text-caption text-ink3">{c.mac}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Card>
      )}

      {(grouped.usb.length > 0 || usbLink) && (
        <Card title={`USB-C (${grouped.usb.length})`} pad={false}>
          <div className="px-4 pb-3">
            {usbLink && (
              <div className="mb-3 flex flex-wrap items-center gap-2 rounded-ctl bg-surface2/70 px-3 py-2">
                <span className="label">Tether link</span>
                <span className="text-body font-bold text-ink">
                  {usbLink.negotiated_label ?? usbLink.negotiated ?? 'Unknown'}
                  {usbNegotiatedRate && <span className="font-medium text-ink2"> · {usbNegotiatedRate}</span>}
                </span>
                {usbLink.at_full_speed === false && usbMaxRate && (
                  <Chip tone="warn">
                    {usbLink.max_label ?? 'Higher'} capable · {usbMaxRate} — cable/port limiting
                  </Chip>
                )}
                {usbLink.at_full_speed === true && <Chip tone="ok">Full speed</Chip>}
              </div>
            )}
            {grouped.usb.length > 0 ? (
              <div className="overflow-x-auto">
                <table className="w-full text-body">
                  <thead>
                    <tr className="label border-b border-line/8 text-left">
                      <th className={TH_CLS}>Hostname</th>
                      <th className={TH_CLS}>IP</th>
                      <th className={TH_CLS}>Interface</th>
                      <th className="pb-1.5 font-semibold">MAC</th>
                    </tr>
                  </thead>
                  <tbody>
                    {grouped.usb.map((c) => (
                      <tr key={c.mac} className="border-b border-line/6 last:border-0">
                        <td className={`${TD_CLS} font-medium text-ink`}>{c.hostname || '\u2014'}</td>
                        <td className={`${TD_CLS} tnum font-mono text-meta text-ink2`}>{c.ip ?? '\u2014'}</td>
                        <td className={`${TD_CLS} tnum font-mono text-meta text-ink2`}>{c.interface ?? '\u2014'}</td>
                        <td className="tnum py-2 font-mono text-caption text-ink3">{c.mac}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            ) : (
              <p className="text-body text-ink3">No USB-C clients connected</p>
            )}
          </div>
        </Card>
      )}

      {grouped.ethernet.length > 0 && (
        <Card title={`Ethernet (${grouped.ethernet.length})`} pad={false}>
          <div className="overflow-x-auto px-4 pb-3">
            <table className="w-full text-body">
              <thead>
                <tr className="label border-b border-line/8 text-left">
                  <th className={TH_CLS}>Hostname</th>
                  <th className={TH_CLS}>IP</th>
                  <th className={TH_CLS}>Speed</th>
                  <th className="pb-1.5 font-semibold">MAC</th>
                </tr>
              </thead>
              <tbody>
                {grouped.ethernet.map((c) => (
                  <tr key={c.mac} className="border-b border-line/6 last:border-0">
                    <td className={`${TD_CLS} font-medium text-ink`}>{c.hostname || '\u2014'}</td>
                    <td className={`${TD_CLS} tnum font-mono text-meta text-ink2`}>{c.ip ?? '\u2014'}</td>
                    <td className={`${TD_CLS} tnum font-mono text-ink2`}>{formatLinkMbps(c.wired_link_mbps)}</td>
                    <td className="tnum py-2 font-mono text-caption text-ink3">{c.mac}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Card>
      )}

      {grouped.other.length > 0 && (
        <Card title={`Other (${grouped.other.length})`} pad={false}>
          <div className="overflow-x-auto px-4 pb-3">
            <table className="w-full text-body">
              <thead>
                <tr className="label border-b border-line/8 text-left">
                  <th className={TH_CLS}>Hostname</th>
                  <th className={TH_CLS}>IP</th>
                  <th className="pb-1.5 font-semibold">MAC</th>
                </tr>
              </thead>
              <tbody>
                {grouped.other.map((c) => (
                  <tr key={c.mac} className="border-b border-line/6 last:border-0">
                    <td className={`${TD_CLS} font-medium text-ink`}>{c.hostname || '\u2014'}</td>
                    <td className={`${TD_CLS} tnum font-mono text-meta text-ink2`}>{c.ip ?? '\u2014'}</td>
                    <td className="tnum py-2 font-mono text-caption text-ink3">{c.mac}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Card>
      )}
    </div>
  )
}
