import { useState } from 'react'
import { Tabs } from '../../ui/Tabs'
import ApnTab from './ApnTab'
import DataTab from './DataTab'
import TtlTab from './TtlTab'
import SmsTab from './SmsTab'

type Tab = 'apn' | 'data' | 'ttl' | 'sms'

export default function ModemGroup() {
  const [tab, setTab] = useState<Tab>('apn')

  return (
    <div className="space-y-4">
      <div>
        <h1 className="hidden font-display text-2xl font-semibold tracking-[-0.015em] text-ink lg:block">Modem</h1>
        <p className="lg:mt-0.5 text-body text-ink2">APN profiles, data usage, TTL and SMS</p>
      </div>

      <Tabs
        tabs={[
          { id: 'apn', label: 'APN' },
          { id: 'data', label: 'Data' },
          { id: 'ttl', label: 'TTL' },
          { id: 'sms', label: 'SMS' },
        ]}
        active={tab}
        onChange={setTab}
      />

      {tab === 'apn' && <ApnTab />}
      {tab === 'data' && <DataTab />}
      {tab === 'ttl' && <TtlTab />}
      {tab === 'sms' && <SmsTab />}
    </div>
  )
}
