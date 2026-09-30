import { useCallback, useEffect, useState } from 'react'
import { api } from '../../data/api'
import type { SmsCapabilities, SmsMessage } from '../../types'
import { IMessage, IPlus } from '../../icons'
import { Button, Field, Input, Segmented } from '../../ui/controls'
import { toast, toastError, confirm } from '../../ui/feedback'
import { Card, Empty, Skeleton } from '../../ui/primitives'

const BOX_INBOX = 1
const BOX_SENT = 2

function formatDate(d?: string) {
  if (!d) return ''
  try {
    const stock = d.match(/^(\d{2});(\d{2});(\d{2});(\d{2});(\d{2});(\d{2})/)
    const value = stock
      ? new Date(2000 + Number(stock[1]), Number(stock[2]) - 1, Number(stock[3]), Number(stock[4]), Number(stock[5]), Number(stock[6]))
      : new Date(d)
    return Number.isNaN(value.getTime()) ? d : value.toLocaleString()
  } catch {
    return d
  }
}

export default function SmsTab() {
  const [box, setBox] = useState(BOX_INBOX)
  const [messages, setMessages] = useState<SmsMessage[]>([])
  const [loading, setLoading] = useState(true)
  const [selected, setSelected] = useState<SmsMessage | null>(null)
  const [composing, setComposing] = useState(false)
  const [to, setTo] = useState('')
  const [text, setText] = useState('')
  const [sending, setSending] = useState(false)
  const [capabilities, setCapabilities] = useState<SmsCapabilities | null | undefined>(undefined)

  useEffect(() => {
    api.smsCapabilities().then(setCapabilities).catch(() => setCapabilities(null))
  }, [])

  const load = useCallback(async () => {
    setLoading(true)
    try {
      const all = await api.smsList()
      setMessages(all.filter((message) => box === BOX_INBOX ? message.tag === 0 || message.tag === 1 : message.tag === 2 || message.tag === 3))
    } catch {
      setMessages([])
    } finally {
      setLoading(false)
    }
  }, [box])

  useEffect(() => {
    if (capabilities?.available && capabilities.ready) load()
    else if (capabilities !== undefined) setLoading(false)
  }, [capabilities, load])

  async function markRead(id: number) {
    try {
      await api.smsRead([id])
    } catch {
      /* ignore */
    }
  }

  async function deleteMsg(id: number) {
    const ok = await confirm({ title: 'Delete this message?', confirmLabel: 'Delete', danger: true })
    if (!ok) return
    try {
      await api.smsDelete([id])
      setMessages((m) => m.filter((x) => x.id !== id))
      if (selected?.id === id) setSelected(null)
      toast('Message deleted')
    } catch (e) {
      toastError(e, 'Delete failed')
    }
  }

  function openMsg(m: SmsMessage) {
    setSelected(m)
    if (m.tag === 1) {
      markRead(m.id)
      setMessages((ms) => ms.map((x) => (x.id === m.id ? { ...x, tag: 0 } : x)))
    }
  }

  async function send(e: React.FormEvent) {
    e.preventDefault()
    setSending(true)
    try {
      await api.smsSend(to, text)
      toast('Message sent')
      setTo('')
      setText('')
      setComposing(false)
      if (box === BOX_SENT) load()
    } catch (err) {
      toastError(err, 'Failed to send')
    } finally {
      setSending(false)
    }
  }

  const unread = messages.filter((m) => m.tag === 1).length

  if (capabilities === undefined) return <Skeleton className="h-64" />
  if (!capabilities?.available || !capabilities.ready) {
    return (
      <Card title="SMS unavailable">
        <Empty
          icon={<IMessage size={26} />}
          title="Firmware WMS is not ready"
          body={capabilities?.reason ?? 'The agent could not verify the SMS service, so listing, sending, and deletion are disabled.'}
        />
      </Card>
    )
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <Segmented
          options={[
            { value: String(BOX_INBOX), label: unread > 0 ? `Inbox (${unread})` : 'Inbox' },
            { value: String(BOX_SENT), label: 'Sent' },
          ]}
          value={String(box)}
          onChange={(v) => {
            setBox(Number(v))
            setSelected(null)
          }}
        />
        <Button
          variant="primary"
          onClick={() => {
            setComposing(true)
            setSelected(null)
          }}
        >
          <IPlus size={14} /> New
        </Button>
      </div>

      {composing && (
        <Card title="New message">
          <form onSubmit={send} className="space-y-2.5">
            <Field label="To">
              <Input value={to} onChange={(e) => setTo(e.target.value)} required placeholder="+61400000000" inputMode="tel" />
            </Field>
            <Field label="Message">
              <textarea
                value={text}
                onChange={(e) => setText(e.target.value)}
                required
                rows={4}
                className="w-full resize-none rounded-ctl border border-line/12 bg-surface2/50 px-3 py-2 text-body text-ink outline-none transition-colors placeholder:text-ink3 focus:border-accent/60"
                placeholder="Type a message…"
              />
            </Field>
            <div className="flex items-center gap-2">
              <Button type="submit" variant="primary" loading={sending} disabled={!to || !text}>
                Send
              </Button>
              <Button type="button" variant="ghost" onClick={() => setComposing(false)}>
                Cancel
              </Button>
              <span className="tnum font-mono ml-auto text-caption text-ink3">{text.length}/160</span>
            </div>
          </form>
        </Card>
      )}

      <div className="grid grid-cols-1 gap-3 lg:grid-cols-3">
        <Card className="lg:col-span-1" title={box === BOX_INBOX ? 'Inbox' : 'Sent'} pad={false}>
          {loading ? (
            <div className="space-y-2 p-4">
              <Skeleton className="h-14" />
              <Skeleton className="h-14" />
              <Skeleton className="h-14" />
            </div>
          ) : messages.length === 0 ? (
            <Empty icon={<IMessage size={26} />} title="No messages" />
          ) : (
            <ul className="max-h-[32rem] divide-y divide-line/6 overflow-y-auto">
              {messages.map((m) => (
                <li key={m.id}>
                  <button
                    onClick={() => openMsg(m)}
                    className={`block w-full px-4 py-2.5 text-left transition-colors hover:bg-surface2/60 ${
                      selected?.id === m.id ? 'bg-accent/8' : ''
                    }`}
                  >
                    <div className="flex items-start justify-between gap-2">
                      <p className={`truncate text-body ${m.tag === 1 ? 'font-bold text-ink' : 'font-medium text-ink2'}`}>
                        {m.number || '—'}
                      </p>
                      {m.tag === 1 && <span className="mt-1 h-2 w-2 shrink-0 rounded-full bg-accent" />}
                    </div>
                    <p className="mt-0.5 truncate text-meta text-ink3">{m.content}</p>
                    <p className="tnum font-mono mt-0.5 text-caption text-ink3">{formatDate(m.date)}</p>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </Card>

        <Card className="lg:col-span-2" title={selected ? 'Message' : 'Select a message'}>
          {selected ? (
            <div className="space-y-3">
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0">
                  <p className="truncate text-body font-semibold text-ink">
                    {box === BOX_INBOX ? 'From' : 'To'}: {selected.number || '\u2014'}
                  </p>
                  <p className="tnum font-mono mt-0.5 text-caption text-ink3">{formatDate(selected.date)}</p>
                </div>
                <Button size="sm" variant="ghost" onClick={() => deleteMsg(selected.id)}>
                  Delete
                </Button>
              </div>
              <div className="rounded-ctl bg-surface2/70 p-3.5">
                <p className="whitespace-pre-wrap break-words text-body leading-relaxed text-ink">{selected.content}</p>
              </div>
              {box === BOX_INBOX && (
                <Button
                  variant="outline"
                  onClick={() => {
                    setComposing(true)
                    setTo(selected.number)
                    setSelected(null)
                  }}
                >
                  Reply
                </Button>
              )}
            </div>
          ) : (
            <Empty icon={<IMessage size={26} />} title="No message selected" />
          )}
        </Card>
      </div>
    </div>
  )
}
