import './style.css'
import { StrictMode, useEffect, useState } from 'react'
import { createRoot } from 'react-dom/client'
import { AppProvider, useAction, useApp, type View } from './state'
import { api, errMsg } from './api'
import { dropSummary, useFileSupport } from './components/FileSupport'
import { Icon } from './icons'
import { Ribbon } from './components/Ribbon'
import { ImportFlow, importFlow, importTarget } from './components/ImportFlow'
import { Board } from './views/Board'
import { Today } from './views/Today'
import { SessionView } from './views/Session'
import { Plan } from './views/Plan'
import { Progress } from './views/Progress'
import { QueueView } from './views/Queue'
import { SettingsView } from './views/Settings'
import { providerInfo } from '../../shared/defaults'
import { RATINGS } from '../../shared/sched'

const NAV: { id: View; label: string; icon: keyof typeof Icon; key: string }[] = [
  { id: 'today', label: 'Today', icon: 'today', key: '1' }, { id: 'session', label: 'Study', icon: 'timer', key: '2' },
  { id: 'board', label: 'Library', icon: 'topics', key: '3' }, { id: 'plan', label: 'Plan', icon: 'calendar', key: '4' },
  { id: 'progress', label: 'Progress', icon: 'chart', key: '5' }
]

const TITLE: Record<View, string> = { today: 'Today', board: 'Library', session: 'Study', plan: 'Plan', progress: 'Progress', queue: 'AI queue', settings: 'Settings' }

function isTyping(e: KeyboardEvent): boolean {
  const el = e.target as HTMLElement | null
  if (!el) return false
  return el.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(el.tagName)
}

function Shortcuts() {
  const { session, pending, setPending, setView, requestQuickstart, toast } = useApp()
  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (isTyping(e) || e.metaKey || e.ctrlKey || e.altKey) return
      if (document.querySelector('.scrim')) return
      const s = session
      const run = (p: Promise<unknown>): void => { p.catch((x) => toast(errMsg(x), true)) }
      if (e.key === ' ' && s && (s.phase === 'work' || s.phase === 'break')) {
        e.preventDefault()
        run(s.pausedAt !== null ? api.sessionResume() : api.sessionPause())
      } else if (e.key === 'n' && s) {
        e.preventDefault()
        run(api.sessionSkip())
      } else if (/^[1-4]$/.test(e.key) && s && (s.phase === 'rate' || s.phase === 'work')) {
        e.preventDefault()
        setPending({ rating: RATINGS[Number(e.key) - 1] })
        setView('session')
      } else if (e.key === 'Enter' && s && s.phase === 'rate' && pending.rating) {
        e.preventDefault()
        run(api.sessionRate({ rating: pending.rating, quiz: pending.quiz, note: pending.note || undefined }))
      } else if (e.key === 'Enter' && s && s.phase === 'next' && s.proposal) {
        e.preventDefault()
        run(api.sessionAccept(s.proposal.topicId))
      } else if (e.key === 'x' && s && (s.phase === 'next' || s.phase === 'break') && s.proposal) {
        e.preventDefault()
        run(api.sessionSkipSuggestion())
      } else if (e.key === 's') {
        e.preventDefault()
        setView('session')
        if (!s) requestQuickstart()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [session, pending, setPending, setView, requestQuickstart, toast])
  return null
}

function DropTarget() {
  const { view, courseWizard, courses } = useApp()
  const [over, setOver] = useState(false)
  const files = useFileSupport()
  useEffect(() => api.onFileDrop((e) => {
    setOver(e.type === 'over')
    if (e.type === 'drop' && e.paths.length) importFlow.drop(e.paths)
  }), [])
  if (!over) return null
  const target = view === 'board' && !courseWizard ? courses.get(importTarget.course ?? '') : undefined
  return (
    <div className="dropzone">
      <div>
        {courseWizard ? 'Drop files or folders to add them to the new course' : target ? `Drop lesson files or folders to add them to ${target.name}` : 'Drop lesson files or folders to start a new course'}
        <span className="muted small" style={{ display: 'block', marginTop: 4 }}>{dropSummary(files)}</span>
      </div>
    </div>
  )
}

/** The window's left edge: the product, where you can go, and what the AI is doing right now. */
function Sidebar() {
  const { view, setView, queue, data, now } = useApp()
  const act = useAction()
  const busy = !!queue && (!!queue.running || queue.queued > 0)
  const due = data ? data.topics.filter((t) => t.due !== null && t.due !== undefined && t.due <= now).length : 0
  const waiting = queue ? queue.queued + queue.failed.length : 0
  const state = !queue ? null
    : queue.rateLimit ? 'Limit reached, paused'
      : queue.failed.length ? `${queue.failed.length} failed`
        : queue.userPaused && queue.queued ? 'Paused'
          : queue.run?.state === 'waiting' ? 'Waiting for Claude'
            : busy ? `Organizing ${queue.chunksDone}/${queue.chunksTotal || queue.queued}` : null
  const warn = !!(queue?.rateLimit || queue?.failed.length)
  return (
    <aside className="sidebar">
      <div className="sidebar-drag" data-tauri-drag-region />
      <div className="brand" data-tauri-drag-region>
        <svg className="mark" width="20" height="20" viewBox="0 0 20 20" aria-hidden="true">
          <path className="lane" d="M5 2v16M15 2v16" />
          <path className="thread" d="M5 3c0 3 10 2 10 5s-10 2-10 5 10 2 10 4" />
        </svg>
        <span>Interleaved Study</span>
      </div>
      <nav className="nav" aria-label="Views">
        {NAV.map((t) => {
          const I = Icon[t.icon]
          const n = t.id === 'today' ? due : 0
          return (
            <button key={t.id} className="nav-item" aria-current={view === t.id ? 'page' : undefined} onClick={() => setView(t.id)} title={`${t.label} (⌘${t.key})`}>
              <I /><span>{t.label}</span>{n > 0 && <span className="nav-count num" title={`${n} topics due for review`}>{n}</span>}
            </button>
          )
        })}
      </nav>
      <div className="sidebar-foot">
        <div className={`ai-row${warn ? ' warn' : ''}`}>
          <button className="nav-item" aria-current={view === 'queue' ? 'page' : undefined} onClick={() => setView('queue')} title="AI queue (⌘6)">
            {queue?.running && !queue.userPaused ? <span className="spinner" /> : <Icon.layers />}
            <span className="ai-text"><span>AI queue</span>{state && <span className="ai-state">{state}</span>}</span>
            {!state && waiting > 0 && <span className="nav-count num">{waiting}</span>}
          </button>
          {busy && <button className="icon-btn" title="Cancel all AI requests" aria-label="Cancel all AI requests" onClick={() => void act(() => api.cancelAi(), 'AI requests cancelled')}><Icon.close /></button>}
        </div>
        <button className="nav-item" aria-current={view === 'settings' ? 'page' : undefined} onClick={() => setView('settings')} title="Settings (⌘,)">
          <Icon.sliders /><span>Settings</span>
        </button>
      </div>
    </aside>
  )
}

/**
 * Shown on every view while the provider's rate limit has paused the AI. Continue resumes the persisted queue:
 * the request that hit the limit is retried and everything already finished stays done.
 */
function RateLimitBanner() {
  const { queue, toast } = useApp()
  const rl = queue?.rateLimit ?? null
  const [now, setNow] = useState(Date.now())
  useEffect(() => {
    if (!rl?.resetAt) return
    const id = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(id)
  }, [rl?.resetAt])
  if (!rl) return null
  const name = providerInfo(rl.provider).label.replace(/ \(.*\)$/, '')
  const routine = rl.provider === 'routine'
  const left = rl.resetAt ? Math.max(0, Math.ceil((rl.resetAt - now) / 1000)) : null
  const wait = left === null ? '' : left > 0 ? ` ${name} asked to wait ${left >= 120 ? `about ${Math.ceil(left / 60)} min` : `${left} s`}.` : ` ${name}’s wait time has passed.`
  return (
    <div className="banner alert" role="alert">
      <span>
        <b>{routine ? 'The Claude scheduled task stopped answering or hit its limit.' : `${name} rate limit reached.`}</b> Everything is paused and nothing is lost; Continue picks up exactly where it stopped.{wait}
        <span className="muted small sel" style={{ display: 'block' }} title={rl.message}>{rl.model}: {rl.message.slice(0, 160)}</span>
      </span>
      <button className="btn primary" onClick={() => void api.resumeQueue().then(() => toast('Continuing'), (e) => toast(errMsg(e), true))}><Icon.play /> Continue</button>
    </div>
  )
}

function Toast() {
  const { toastMsg } = useApp()
  if (!toastMsg) return null
  return <div className={`toast${toastMsg.error ? ' error' : ''}`} role="status">{toastMsg.msg}</div>
}

function Shell() {
  const { view, data, dataError, reload } = useApp()
  let body
  if (dataError && !data) {
    body = <div className="empty"><h2>The data could not be loaded</h2><p className="err-text">{dataError}</p><button className="btn" onClick={() => void reload()}>Try again</button></div>
  } else if (!data) {
    body = <div className="empty"><span className="spinner" /></div>
  } else {
    body = (
      <>
        {view === 'today' && <Today />}
        {view === 'board' && <Board />}
        {view === 'session' && <SessionView />}
        {view === 'plan' && <Plan />}
        {view === 'progress' && <Progress />}
        {view === 'queue' && <QueueView />}
        {view === 'settings' && <SettingsView />}
      </>
    )
  }
  return (
    <div className="app" data-view={view}>
      <Sidebar />
      <div className="content">
        <div className="content-top">
          <header className="topbar" data-tauri-drag-region>
            <h1 className="view-title" data-tauri-drag-region>{TITLE[view]}</h1>
            <Ribbon />
          </header>
          <RateLimitBanner />
        </div>
        <main>{body}</main>
      </div>
      <ImportFlow />
      <DropTarget />
      <Shortcuts />
      <Toast />
    </div>
  )
}

// Links to the provider consoles and GitHub open in the browser, never inside the app's window.
document.addEventListener('click', (e) => {
  const a = (e.target as HTMLElement | null)?.closest?.('a[href]') as HTMLAnchorElement | null
  if (!a || !/^https?:/.test(a.href)) return
  e.preventDefault()
  void api.openExternal(a.href)
})

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <AppProvider>
      <Shell />
    </AppProvider>
  </StrictMode>
)
