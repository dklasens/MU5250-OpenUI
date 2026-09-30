import { useState } from 'react'
import { Tabs } from '../../ui/Tabs'
import MetricsTab from './MetricsTab'
import ToolsTab from './ToolsTab'
import SettingsTab from './SettingsTab'

type Tab = 'metrics' | 'tools' | 'settings'

export default function SystemGroup({ onLogout }: { onLogout: () => void }) {
  const [tab, setTab] = useState<Tab>('metrics')

  return (
    <div className="space-y-4">
      <div>
        <h1 className="hidden font-display text-2xl font-semibold tracking-[-0.015em] text-ink lg:block">System</h1>
        <p className="lg:mt-0.5 text-body text-ink2">Health metrics, diagnostic tools and device controls</p>
      </div>

      <Tabs
        tabs={[
          { id: 'metrics', label: 'Metrics' },
          { id: 'tools', label: 'Tools' },
          { id: 'settings', label: 'Settings' },
        ]}
        active={tab}
        onChange={setTab}
      />

      {tab === 'metrics' && <MetricsTab />}
      {tab === 'tools' && <ToolsTab />}
      {tab === 'settings' && <SettingsTab onLogout={onLogout} />}
    </div>
  )
}
