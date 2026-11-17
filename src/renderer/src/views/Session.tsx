// Session view: choose a pool and mode, then study timed blocks or Pomodoros, rate, quiz, and accept or
// override the next suggestion. All timing lives in the main process, so switching views never loses it.
import { useEffect, useMemo, useState } from 'react'
import type { Pool, Rating, Suggestion } from '../../../shared/types'
import { RATINGS } from '../../../shared/sched'
import { api } from '../api'
import { courseClass, useAction, useApp, useConfirmAction } from '../state'
import { Icon } from '../icons'
import { Deck, Empty, deckName, fmtClock, fmtMin, Spinner } from '../ui'
import { QuizPanel } from '../components/Quiz'
import { useCountdown } from '../components/Ribbon'
import { Lanes, type LaneItem } from '../components/Lanes'

const RATE_HELP: Record<Rating, string> = {
  lost: 'Could not recall the main ideas', shaky: 'Partly, with gaps', okay: 'Mostly, with some effort', solid: 'Clearly and confidently'
}
const cap = (s: string): string => s[0].toUpperCase() + s.slice(1)

let handledQuick = 0

type PoolKind = 'global' | 'course' | 'lesson' | 'unit' | 'topic' | 'sandbox'

/** A pool's suggested order as the interleave line; clicking a topic starts with it. */
function Suggestions({ items, onPick, busy, limit = 12 }: { items: Suggestion[] | null; onPick: (id: string) => void; busy?: boolean; limit?: number }) {
  const { topics } = useApp()
  if (!items) return <Spinner />
  if (!items.length) return <div className="muted">No topics in this pool.</div>
  return (
    <Lanes label="Suggested order" items={items.slice(0, limit).map((s, i) => ({
      key: s.topicId, courseId: topics.get(s.topicId)?.courseId ?? '', state: i === 0 ? 'next' : 'later',
      onClick: () => onPick(s.topicId), disabled: busy, title: busy ? 'Rate or skip the rating first' : 'Study this topic',
      body: <><span className="lane-name">{s.name}</span><span className="lane-meta"><Deck topicId={s.topicId} /><span className="lane-why">{s.reason}</span></span></>
    }))} />
  )
}

const POOLS: { k: PoolKind; label: string; help: string }[] = [
  { k: 'global', label: 'Everything', help: 'All courses, mixed' },
  { k: 'course', label: 'Courses', help: 'One or more, mixed' },
  { k: 'lesson', label: 'One lesson', help: 'Its topics, in a mixed order' },
  { k: 'unit', label: 'One unit', help: 'Part of a lesson' },
  { k: 'topic', label: 'One topic', help: 'Just this one' },
  { k: 'sandbox', label: 'Sandbox', help: 'Hand-picked, off the schedule' }
]

function StartPanel() {
  const { data, settings, saveSettings, quickstart, scope } = useApp()
  const act = useAction()
  const [mode, setMode] = useState<'timed' | 'pomodoro'>(() => (localStorage.getItem('mode') as 'timed' | 'pomodoro') || 'timed')
  const initialKind: PoolKind = scope.kind === 'course' ? 'course' : scope.kind === 'lesson' ? 'lesson' : scope.kind === 'unit' ? 'unit' : 'global'
  const [kind, setKind] = useState<PoolKind>(initialKind)
  const [courseIds, setCourseIds] = useState<string[]>(scope.kind === 'course' ? [scope.id] : [])
  const [docId, setDocId] = useState(scope.kind === 'lesson' ? scope.id : scope.kind === 'unit' ? scope.docId : data?.docs[0]?.id ?? '')
  const [unit, setUnit] = useState(scope.kind === 'unit' ? scope.unit : '')
  const [ids, setIds] = useState<string[]>([])
  const [filter, setFilter] = useState('')
  const [preview, setPreview] = useState<Suggestion[] | null>(null)

  const units = useMemo(() => [...new Set((data?.topics ?? []).filter((t) => t.docId === docId).sort((a, b) => a.ord - b.ord).map((t) => t.unit))], [data, docId])
  useEffect(() => { if (kind === 'unit' && !units.includes(unit)) setUnit(units[0] ?? '') }, [kind, units, unit])

  const pool: Pool | null = useMemo(() => {
    switch (kind) {
      case 'global': return { kind: 'global' }
      case 'course': return courseIds.length ? { kind: 'course', courseIds } : null
      case 'lesson': return docId ? { kind: 'lesson', docId } : null
      case 'unit': return docId && unit ? { kind: 'unit', docId, unit } : null
      case 'topic': return ids.length ? { kind: 'topic', ids: ids.slice(0, 1) } : null
      case 'sandbox': return ids.length ? { kind: 'sandbox', ids } : null
    }
  }, [kind, courseIds, docId, unit, ids])

  const label = (): string => {
    if (!data) return ''
    if (kind === 'global') return 'Everything'
    if (kind === 'course') return courseIds.map((id) => data.courses.find((c) => c.id === id)?.name).join(', ')
    if (kind === 'lesson') return data.docs.find((d) => d.id === docId)?.title ?? 'Lesson'
    if (kind === 'unit') return unit
    if (kind === 'topic') return data.topics.find((t) => t.id === ids[0])?.name ?? 'Topic'
    return `Sandbox of ${ids.length} topics`
  }

  useEffect(() => {
    if (!pool) { setPreview([]); return }
    let live = true
    setPreview(null)
    api.sessionPreview(pool, pool.kind === 'sandbox').then((s) => { if (live) setPreview(s) }).catch(() => live && setPreview([]))
    return () => { live = false }
  }, [pool, data])

  const start = async (topicId?: string): Promise<void> => {
    if (!pool) return
    localStorage.setItem('mode', mode)
    await act(() => api.sessionStart({ mode, pool, poolLabel: label(), topicId: topicId ?? null }))
  }

  useEffect(() => {
    if (quickstart > handledQuick) { handledQuick = quickstart; void start() }
  }) // eslint-disable-line react-hooks/exhaustive-deps

  if (!data || !settings) return null
  if (!data.topics.length) return <Empty title="No topics to study yet"><p>Import lessons on the Topics page first.</p></Empty>

  const toggleId = (id: string, multi: boolean): void => setIds((p) => (multi ? (p.includes(id) ? p.filter((x) => x !== id) : [...p, id]) : [id]))
  const filtered = data.topics.filter((t) => !filter || t.name.toLowerCase().includes(filter.toLowerCase())).slice(0, 300)

  const num = (v: number, on: (n: number) => void, label: string, w = 56, min = 1, max?: number) => (
    <input className="input num" type="number" min={min} max={max} style={{ width: w }} value={v} onChange={(e) => on(Math.max(min, Number(e.target.value) || min))} aria-label={label} />
  )
  return (
    <div className="page">
      <div className="page-inner">
        <header className="page-head">
          <h2 className="display">What do you want to study?</h2>
          <p className="lede">Pick a pool and a timer. The app mixes topics from different lessons and courses, puts overdue reviews first and gives close deadlines priority.</p>
        </header>
        <div className="start-grid">
          <div className="start-form">
            <section aria-labelledby="pool-h">
              <h3 id="pool-h">Pool</h3>
              <div className="choices" role="radiogroup" aria-labelledby="pool-h">
                {POOLS.map((o) => (
                  <label key={o.k} className="choice">
                    <input type="radio" name="pool" checked={kind === o.k} onChange={() => setKind(o.k)} />
                    <span><b>{o.label}</b><span>{o.help}</span></span>
                  </label>
                ))}
              </div>
              <div className="pool-pick">
                {kind === 'course' && <div className="pick-list">{data.courses.map((c) => (
                  <label key={c.id} className={`check ${courseClass(c)}`}><input type="checkbox" checked={courseIds.includes(c.id)} onChange={() => setCourseIds((p) => (p.includes(c.id) ? p.filter((x) => x !== c.id) : [...p, c.id]))} /><span className="dot" /><span>{c.name}</span></label>
                ))}</div>}
                {(kind === 'lesson' || kind === 'unit') && (
                  <select className="input" value={docId} onChange={(e) => setDocId(e.target.value)} aria-label="Lesson">
                    {data.courses.map((c) => <optgroup key={c.id} label={c.name}>{data.docs.filter((d) => d.courseId === c.id).map((d) => <option key={d.id} value={d.id}>{d.title}</option>)}</optgroup>)}
                  </select>
                )}
                {kind === 'unit' && <select className="input" value={unit} onChange={(e) => setUnit(e.target.value)} aria-label="Unit">{units.map((u) => <option key={u} value={u}>{u}</option>)}</select>}
                {(kind === 'topic' || kind === 'sandbox') && (
                  <>
                    <input className="input" placeholder="Filter topics" value={filter} onChange={(e) => setFilter(e.target.value)} aria-label="Filter topics" />
                    <div className="pick-list">{filtered.map((t) => (
                      <label key={t.id} className="check"><input type={kind === 'topic' ? 'radio' : 'checkbox'} name="pick" checked={ids.includes(t.id)} onChange={() => toggleId(t.id, kind === 'sandbox')} /><span><span className="serif">{t.name}</span> <Deck topicId={t.id} /></span></label>
                    ))}</div>
                    {kind === 'sandbox' && <p className="help">Ratings in a sandbox stay separate. At the end you choose to save them to the schedule or discard them.</p>}
                  </>
                )}
              </div>
            </section>
            <section aria-labelledby="timer-h">
              <h3 id="timer-h">Timer</h3>
              <div className="seg" role="group" aria-label="Mode">
                <button aria-pressed={mode === 'timed'} onClick={() => setMode('timed')}>Timed topics</button>
                <button aria-pressed={mode === 'pomodoro'} onClick={() => setMode('pomodoro')}>Interleaved Pomodoro</button>
              </div>
              {mode === 'timed' ? (
                <div className="sentence">
                  <span>Each topic gets</span>{num(settings.study.minutes, (n) => void saveSettings({ study: { minutes: n } }), 'Minutes per topic', 64, 1, 240)}<span>minutes.</span>
                  <label className="check"><input type="checkbox" checked={settings.study.useSuggested} onChange={(e) => void saveSettings({ study: { useSuggested: e.target.checked } })} /><span>Use each topic’s suggested time instead</span></label>
                </div>
              ) : (
                <div className="sentence">
                  <span>Work</span>{num(settings.pomo.work, (n) => void saveSettings({ pomo: { work: n } }), 'Work minutes')}
                  <span>min, break</span>{num(settings.pomo.brk, (n) => void saveSettings({ pomo: { brk: n } }), 'Break minutes')}
                  <span>min, a long break of</span>{num(settings.pomo.long, (n) => void saveSettings({ pomo: { long: n } }), 'Long break minutes')}
                  <span>min every</span>{num(settings.pomo.cycles, (n) => void saveSettings({ pomo: { cycles: n } }), 'Cycles before long break', 52)}<span>blocks.</span>
                  <select className="input" style={{ width: 'auto' }} value={settings.pomo.perTopic} onChange={(e) => void saveSettings({ pomo: { perTopic: e.target.value as 'one' | 'n' | 'minutes' } })} aria-label="Topic length">
                    <option value="one">1 Pomodoro per topic</option><option value="n">N Pomodoros per topic</option><option value="minutes">Custom minutes per topic</option>
                  </select>
                  {settings.pomo.perTopic === 'n' && num(settings.pomo.n, (n) => void saveSettings({ pomo: { n } }), 'Pomodoros per topic', 52)}
                  {settings.pomo.perTopic === 'minutes' && <>{num(settings.pomo.minutes, (n) => void saveSettings({ pomo: { minutes: n } }), 'Minutes per topic', 64)}<span>min per topic</span></>}
                </div>
              )}
            </section>
            <div className="start-go">
              <button className="btn primary big" disabled={!pool || !preview?.length} onClick={() => void start()}><Icon.play /> Start with the top suggestion <kbd>S</kbd></button>
              <span className="muted small">{pool ? label() : 'Choose what goes in the pool'}</span>
            </div>
          </div>
          <section className="start-order" aria-labelledby="order-h">
            <h3 id="order-h">Suggested order</h3>
            <p className="help">Click any topic to start with it instead.</p>
            <Suggestions items={preview} onPick={(id) => void start(id)} />
          </section>
        </div>
      </div>
    </div>
  )
}

export function SessionView() {
  const { session, topics, docs, courses, pending, setPending, data } = useApp()
  const act = useAction()
  const ask = useConfirmAction()
  const left = useCountdown()
  const [quiz, setQuiz] = useState(false)
  const [others, setOthers] = useState<Suggestion[] | null>(null)

  useEffect(() => { setQuiz(false); setOthers(null) }, [session?.topicId, session?.phase])
  useEffect(() => {
    if (!session || session.phase === 'work') return
    let live = true
    api.sessionPreview(session.pool, session.sandbox).then((s) => { if (live) setOthers(s) }).catch(() => live && setOthers([]))
    return () => { live = false }
  }, [session?.phase, session?.topicId, session?.pool, session?.sandbox, data]) // eslint-disable-line react-hooks/exhaustive-deps

  if (!session) return <StartPanel />
  const t = session.topicId ? topics.get(session.topicId) : undefined
  const d = t ? docs.get(t.docId) : undefined
  const c = t ? courses.get(t.courseId) : undefined
  const paused = session.pausedAt !== null
  const pick = (id: string): void => void act(() => api.sessionAccept(id))

  const done = session.blocks.filter((b) => b.state !== 'current')
  const cur = session.blocks.find((b) => b.state === 'current')
  const timing = session.phase === 'work' || session.phase === 'break'
  const propCourse = session.proposal ? topics.get(session.proposal.topicId)?.courseId : undefined
  const laneItems: LaneItem[] = [
    ...done.map((b, i) => ({
      key: `d${i}`, courseId: b.courseId, state: (b.state === 'skipped' ? 'skipped' : 'done') as LaneItem['state'],
      body: <><span className="lane-name">{b.name}</span><span className="lane-meta"><Deck topicId={b.topicId} /></span></>,
      aside: <span className="num">{b.state === 'skipped' ? 'skipped' : b.rating}, {Math.round(b.minutes)}m</span>
    })),
    ...(cur && session.phase !== 'next' ? [{
      key: 'cur', courseId: cur.courseId, state: 'current' as const,
      body: <><span className="lane-name">{cur.name}</span><span className="lane-meta">{session.phase === 'rate' ? 'Rate it now' : session.phase === 'break' ? 'On a break' : 'Now'}</span></>
    }] : []),
    ...(session.proposal && session.phase !== 'work' && propCourse ? [{
      key: 'next', courseId: propCourse, state: 'next' as const,
      body: <><span className="lane-name">{session.proposal.name}</span><span className="lane-meta">Up next</span></>
    }] : [])
  ]

  return (
    <div className="session">
      <section className={`stage ph-${session.phase}`} aria-label="Study block">
        {session.message && <div className="banner alert">{session.message}</div>}
        {session.phase === 'done' ? (
          <div className="stage-block">
            <h2 className="stage-topic">Sandbox finished</h2>
            <p className="lede">{session.sandboxReviews.length} block(s) recorded in this sandbox, {fmtMin(session.sandboxReviews.reduce((a, r) => a + r.minutes, 0))} in total. They have not touched your schedule.</p>
            <div className="row-flex">
              <button className="btn primary big" onClick={() => void act(() => api.sessionSandbox(true), 'Saved to schedule')}>Save to schedule</button>
              <button className="btn big" onClick={() => void act(() => api.sessionSandbox(false), 'Sandbox discarded')}>Discard</button>
            </div>
          </div>
        ) : (
          <div className="stage-block">
            <div className="stage-head">
              {session.phase === 'next'
                ? <div className="stage-path">Finished: {done.slice(-1)[0]?.name ?? session.topicName}</div>
                : session.phase === 'break'
                  ? <div className="stage-path">{session.breakKind === 'long' ? 'Long break' : 'Break'}{session.blockIndex < session.blockPlan.length - 1 ? `, then back to ${session.topicName} (block ${session.blockIndex + 2} of ${session.blocksForTopic})` : ''}</div>
                  : <div className={`stage-path ${courseClass(c)}`}><span className="dot" /><span>{c?.name}{d ? ` / ${d.title}` : ''}</span></div>}
              <h2 className="stage-topic sel">{session.phase === 'break' ? 'Stand up, look away, drink water.' : session.phase === 'next' ? 'Choose the next topic' : session.topicName}</h2>
              {(session.phase === 'work' || session.phase === 'rate') && d && <div className="stage-file"><Icon.file /> Study from <b>{deckName(d)}</b></div>}
            </div>

            {timing && (
              <div className="stage-clock">
                <div className={`clock-big num${paused ? ' paused' : ''}`} role="timer" aria-label={`${fmtClock(left)} left`}>{fmtClock(left)}</div>
                <div className="stage-phase">{paused ? 'Paused' : session.phase === 'work' ? (session.mode === 'pomodoro' ? `Pomodoro ${session.blockIndex + 1} of ${session.blocksForTopic}` : 'Timed block') : session.breakKind === 'long' ? 'Long break' : 'Short break'}</div>
                <div className="stage-controls">
                  <button className="btn primary big" onClick={() => void act(() => (paused ? api.sessionResume() : api.sessionPause()))}>{paused ? <><Icon.play /> Resume</> : <><Icon.pause /> Pause</>} <kbd>space</kbd></button>
                  {session.phase === 'work' && <button className="btn big" onClick={() => void act(() => api.sessionEndEarly())}><Icon.check /> Done, rate now</button>}
                  <button className="btn ghost" onClick={() => void act(() => api.sessionExtend(5))}><Icon.plus /> 5 min</button>
                  {session.phase === 'work' && <button className="btn ghost" onClick={() => void act(() => api.sessionSkip())} title="Records time spent only"><Icon.skip /> Skip topic <kbd>N</kbd></button>}
                  {session.phase === 'break' && <button className="btn ghost" onClick={() => void act(() => api.sessionSkipBreak())}><Icon.skip /> Skip break <kbd>N</kbd></button>}
                </div>
              </div>
            )}

            {session.phase === 'rate' && (
              <div className="rate-block">
                <h3>How well do you know it now?</h3>
                <div className="rate-row" role="group" aria-label="Rating">
                  {RATINGS.map((r, i) => (
                    <button key={r} className="rate" aria-pressed={pending.rating === r} onClick={() => setPending({ rating: r })}>
                      <b>{cap(r)} <span className="kbd">{i + 1}</span></b><span>{RATE_HELP[r]}</span>
                    </button>
                  ))}
                </div>
                <div className="row-flex">
                  {pending.quiz !== null ? <span className="small">Quiz score <b className="num">{Math.round(pending.quiz * 100)}%</b>, counts double</span> : <button className="btn" aria-expanded={quiz} onClick={() => setQuiz((x) => !x)}><Icon.quiz /> Quick quiz</button>}
                </div>
                {quiz && pending.quiz === null && <div className="quiz-box"><QuizPanel topicId={session.topicId!} onScore={(q) => { setPending({ quiz: q }); setQuiz(false) }} /></div>}
                <textarea className="input serif" rows={2} placeholder="Optional note for this review" value={pending.note} onChange={(e) => setPending({ note: e.target.value })} aria-label="Note" />
                <div className="row-flex">
                  <button className="btn primary big" disabled={!pending.rating} onClick={() => void act(() => api.sessionRate({ rating: pending.rating, quiz: pending.quiz, note: pending.note || undefined }))}>Save rating <kbd>Enter</kbd></button>
                  <button className="btn big" onClick={() => void act(() => api.sessionRate({ rating: null }))} title="Records the time without a rating">Skip rating</button>
                  <button className="btn ghost" onClick={() => void act(() => api.sessionExtend(5))}>Keep going 5 more min</button>
                </div>
              </div>
            )}

            {(session.phase === 'next' || session.phase === 'break') && (
              <div className={`upnext ${courseClass(courses.get(propCourse ?? ''))}`}>
                {session.proposal ? (
                  <>
                    <div className="upnext-label"><span className="dot" />Up next{propCourse ? `, ${courses.get(propCourse)?.name ?? ''}` : ''}</div>
                    <div className="upnext-name serif">{session.proposal.name}</div>
                    <div className="upnext-meta"><Deck topicId={session.proposal.topicId} /><span>{session.proposal.reason}</span></div>
                    <div className="row-flex">
                      <button className={`btn ${session.phase === 'break' ? '' : 'primary '}big`} onClick={() => pick(session.proposal!.topicId)}><Icon.play /> {session.phase === 'break' ? 'Start it now' : 'Start'} <kbd>Enter</kbd></button>
                      <button className="btn ghost" onClick={() => void act(() => api.sessionSkipSuggestion())} title="Suggest a different topic instead"><Icon.skip /> Something else <kbd>X</kbd></button>
                    </div>
                  </>
                ) : <div className="muted">Nothing left to suggest in this pool. Pick a topic on the right or end the session.</div>}
              </div>
            )}
          </div>
        )}
      </section>
      <aside className="side" aria-label="This session">
        <div>
          <div className="side-kicker">{session.mode === 'pomodoro' ? 'Interleaved Pomodoro' : 'Timed topics'}{session.sandbox ? ', sandbox' : ''}</div>
          <div className="pool-name">{session.poolLabel}</div>
        </div>
        {session.phase === 'work' && t && (t.summary || t.terms.length > 0) && (
          <div>
            <h3>About this topic</h3>
            {t.summary && <p className="summary serif">{t.summary}</p>}
            {t.terms.length > 0 && <div className="terms">{t.terms.map((x) => <span className="chip" key={x}>{x}</span>)}</div>}
          </div>
        )}
        <div>
          <h3>This session</h3>
          {laneItems.length === 0 ? <div className="muted small">No finished blocks yet.</div> : <Lanes label="Blocks in this session" items={laneItems} legend={false} />}
        </div>
        {session.phase !== 'work' && session.phase !== 'done' && (
          <div>
            <h3>Or choose any topic in the pool</h3>
            <Suggestions items={others} onPick={pick} busy={session.phase === 'rate'} limit={8} />
            {session.phase === 'rate' && <p className="help">Rate or skip the rating first.</p>}
          </div>
        )}
        {session.phase !== 'done' && <button className="btn ghost danger end-btn" onClick={() => void ask({
          message: 'End this session?', confirm: 'End Session',
          detail: `Time already spent on the current topic is recorded without a rating.${session.sandbox ? ' You can then save the sandbox to your schedule or discard it.' : ''}`
        }, () => api.sessionEnd())}><Icon.stop /> End session</button>}
      </aside>
    </div>
  )
}
