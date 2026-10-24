import { useEffect, useState } from 'react'
import { useHome } from '../../app/HomeContext'
import { TabPanel, Tabs, type TabDef } from '../../ui/Tabs'
import ApnTab from './ApnTab'
import DataTab from './DataTab'
import TtlTab from './TtlTab'
import SmsTab from './SmsTab'
import SimTab from './SimTab'

type Tab = 'apn' | 'data' | 'ttl' | 'sms' | 'sim'

/**
 * Once an eSIM card has been seen this session the tab stays, even while the heartbeat reports a
 * temporary failure (a SIM reload after a switch makes the card unreadable for a while). It goes
 * as soon as the agent says the SIM in the slot is not an eSIM card (an ordinary SIM swapped in).
 */
let simSeenThisSession = false

export default function ModemGroup() {
  const [tab, setTab] = useState<Tab>('apn')
  const home = useHome()
  const available = home.data?.esim?.available === true
  const unsupported = home.data?.esim?.status === 'unsupported'
  const [showSim, setShowSim] = useState(simSeenThisSession)
  if (available && !showSim) setShowSim(true)
  if (unsupported && showSim) setShowSim(false)
  useEffect(() => {
    simSeenThisSession = showSim
  }, [showSim])

  const tabs: TabDef<Tab>[] = [
    { id: 'apn', label: 'APN' },
    { id: 'data', label: 'Data' },
    { id: 'ttl', label: 'TTL' },
    { id: 'sms', label: 'SMS' },
  ]
  if (showSim) tabs.push({ id: 'sim', label: 'SIM Management' })
  const current: Tab = tab === 'sim' && !showSim ? 'apn' : tab

  return (
    <div className="space-y-4">
      <div>
        <h1 className="hidden font-display text-2xl font-semibold tracking-[-0.015em] text-ink lg:block">Modem</h1>
        <p className="lg:mt-0.5 text-body text-ink2">
          APN profiles, data usage, TTL and SMS{showSim ? ', and the eSIM profiles on the card' : ''}
        </p>
      </div>

      <Tabs tabs={tabs} active={current} onChange={setTab} label="Modem sections" idBase="modem" />

      <TabPanel idBase="modem" id={current} className="space-y-4">
        {current === 'apn' && <ApnTab />}
        {current === 'data' && <DataTab />}
        {current === 'ttl' && <TtlTab />}
        {current === 'sms' && <SmsTab />}
        {current === 'sim' && <SimTab />}
      </TabPanel>
    </div>
  )
}
