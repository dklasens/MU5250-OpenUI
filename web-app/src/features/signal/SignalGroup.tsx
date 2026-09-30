import { useState } from 'react'
import { Tabs } from '../../ui/Tabs'
import Overview from './Overview'
import Locking from './Locking'

type Tab = 'overview' | 'locking'

export default function SignalGroup() {
  const [tab, setTab] = useState<Tab>('overview')

  return (
    <div className="space-y-4">
      <div>
        <h1 className="hidden font-display text-2xl font-semibold tracking-[-0.015em] text-ink lg:block">Signal</h1>
        <p className="lg:mt-0.5 text-body text-ink2">Live radio metrics, band and cell locking</p>
      </div>

      <Tabs
        tabs={[
          { id: 'overview', label: 'Overview' },
          { id: 'locking', label: 'Mode & Locking' },
        ]}
        active={tab}
        onChange={setTab}
      />

      {tab === 'overview' ? <Overview /> : <Locking />}
    </div>
  )
}
