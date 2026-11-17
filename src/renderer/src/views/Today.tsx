// Today: the home. What is due, the order the app would study it in (drawn as the interleave line), one Start
// button, and beside it the day's goal, the next deadlines and each course's standing. Everything here is real
// data; nothing is estimated beyond "minutes per topic × topics".
import { useEffect, useMemo, useState } from 'react'
import type { Suggestion } from '../../../shared/types'
import { computeStreak } from '../../../shared/streak'
import { dayKey, daysBetween, parseDayKey } from '../../../shared/util'
import { api } from '../api'
import { courseClass, useAction, useApp } from '../state'
import { Icon } from '../icons'
import { Deck, Empty, MasteryBar, Spinner, fmtClock, fmtMin, fmtRel } from '../ui'
import { Lanes } from '../components/Lanes'
import { useCountdown } from '../components/Ribbon'
import { useDerived } from './Board'

const plural = (n: number, w: string, ws = `${w}s`): string => `${n} ${n === 1 ? w : ws}`

function listNames(names: string[]): string {
  if (names.length < 2) return names.join('')
  return `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`
}

export function Today() {
  const { data, settings, session, now, courses, setView, setScope, setCourseWizard } = useApp()
  const act = useAction()
  const derived = useDerived()
  const left = useCountdown()
  const [order, setOrder] = useState<Suggestion[] | null>(null)

  useEffect(() => {
    if (!data?.topics.length) { setOrder([]); return }
    let live = true
    api.sessionPreview({ kind: 'global' }, false).then((s) => { if (live) setOrder(s) }).catch(() => live && setOrder([]))
    return () => { live = false }
  }, [data])

  const stats = useMemo(() => {
    const due = derived.filter((d) => d.t.due && d.t.due <= now)
    const fresh = derived.filter((d) => d.s === 'new')
    const nextDue = derived.filter((d) => d.t.due && d.t.due > now).sort((a, b) => (a.t.due ?? 0) - (b.t.due ?? 0))[0]
    return { due, fresh, nextDue }
  }, [derived, now])

  if (!data || !settings) return null
  if (!data.courses.length) {
    return (
      <Empty title="Start with a course">
        <p>A course holds the lecture files for one subject. Each file becomes a lesson, the AI splits it into topics, and this page then shows what to study and in which order.</p>
        <button className="btn primary big" onClick={() => { setView('board'); setCourseWizard([]) }}><Icon.folderPlus /> New course</button>
        <p className="help">You can also drop lecture files anywhere in this window.</p>
      </Empty>
    )
  }

  const mode = (localStorage.getItem('mode') as 'timed' | 'pomodoro') || 'timed'
  const perTopic = (m: number): number => (mode === 'pomodoro' ? settings.pomo.work : settings.study.useSuggested ? m : settings.study.minutes)
  const start = (topicId: string | null): void => {
    void act(async () => {
      await api.sessionStart({ mode, pool: { kind: 'global' }, poolLabel: 'Everything', topicId })
      setView('session')
    })
  }

  const top = (order ?? []).slice(0, 8)
  const topCourses = [...new Set(top.map((s) => data.topics.find((t) => t.id === s.topicId)?.courseId).filter((x): x is string => !!x))]
  const dueMin = stats.due.reduce((a, d) => a + perTopic(d.t.minutes), 0)

  // Goal and streak
  const today = dayKey()
  const todayRec = data.daily.find((d) => d.id === today)
  const goal = todayRec?.goal ?? { kind: settings.goal.kind, value: settings.goal.value }
  const done = goal.kind === 'minutes' ? todayRec?.minutes ?? 0 : todayRec?.topics ?? 0
  const pct = Math.min(100, (100 * done) / Math.max(1, goal.value))
  const streak = computeStreak(new Set(data.daily.filter((d) => d.minutes > 0).map((d) => d.id)), today, settings.goal.restDaysPerWeek)
  const deadlines = data.deadlines.filter((d) => d.date >= today).sort((a, b) => a.date.localeCompare(b.date)).slice(0, 5)

  let headline: string
  let sub: string
  if (stats.due.length) {
    headline = `${plural(stats.due.length, 'topic is', 'topics are')} due for review.`
    sub = `About ${fmtMin(dueMin)} at your current pace${stats.fresh.length ? `, with ${plural(stats.fresh.length, 'new topic')} ready after them` : ''}.`
  } else if (stats.fresh.length) {
    headline = 'Nothing is due.'
    sub = `${plural(stats.fresh.length, 'new topic is', 'new topics are')} ready to learn.`
  } else {
    headline = 'All caught up.'
    sub = stats.nextDue ? `The next review is ${fmtRel(stats.nextDue.t.due, now)}: ${stats.nextDue.t.name}.` : 'Nothing is scheduled yet.'
  }

  const t = session?.topicId ? data.topics.find((x) => x.id === session.topicId) : undefined
  return (
    <div className="page today">
      <div className="page-inner">
        <header className="today-head">
          <div className="today-date">{new Date(now).toLocaleDateString(undefined, { weekday: 'long', month: 'long', day: 'numeric' })}</div>
          {session ? (
            <>
              <h2 className="display">You are in a session.</h2>
              <p className="lede">
                {session.phase === 'work' ? <>Studying <span className="serif-ink">{session.topicName}</span>, {fmtClock(left)} left.</>
                  : session.phase === 'break' ? <>On a break, {fmtClock(left)} left.</>
                    : session.phase === 'rate' ? <>Rate <span className="serif-ink">{session.topicName}</span> to go on.</>
                      : 'Choose the next topic to go on.'}
              </p>
              <div className="today-cta">
                <button className="btn primary big" onClick={() => setView('session')}><Icon.timer /> Back to the session</button>
                {t && (session.phase === 'work' || session.phase === 'rate') && <Deck topicId={t.id} />}
              </div>
            </>
          ) : (
            <>
              <h2 className="display">{headline}</h2>
              <p className="lede">{sub}</p>
              <div className="today-cta">
                <button className="btn primary big" disabled={!top.length} onClick={() => start(null)}><Icon.play /> Start studying <kbd>S</kbd></button>
                <button className="btn big" onClick={() => setView('session')}>Choose what to study</button>
              </div>
            </>
          )}
        </header>

        <div className="today-grid">
          <section className="today-order" aria-labelledby="order-h">
            <div className="sec-h">
              <h2 id="order-h">Up next</h2>
              {topCourses.length > 1 && <span className="sec-note">Mixed across {listNames(topCourses.map((id) => courses.get(id)?.name ?? ''))}</span>}
            </div>
            {order === null ? <Spinner /> : top.length === 0 ? <p className="muted">No topics yet. Lessons appear here once they are imported.</p> : (
              <Lanes label="Suggested study order" items={top.map((s, i) => {
                const tp = data.topics.find((x) => x.id === s.topicId)
                return {
                  key: s.topicId, courseId: tp?.courseId ?? '', state: i === 0 ? 'next' : 'later',
                  title: 'Start a session with this topic',
                  onClick: session ? undefined : () => start(s.topicId),
                  body: <><span className="lane-name">{s.name}</span><span className="lane-meta"><Deck topicId={s.topicId} /><span className="lane-why">{s.reason}</span></span></>,
                  aside: tp ? <span className="num">{fmtMin(perTopic(tp.minutes))}</span> : null
                }
              })} />
            )}
            {(order?.length ?? 0) > top.length && <p className="help">{plural((order?.length ?? 0) - top.length, 'more topic')} after these. The order is worked out again after every block.</p>}
          </section>

          <aside className="today-side">
            <section aria-labelledby="goal-h">
              <div className="sec-h"><h2 id="goal-h">Today’s goal</h2><button className="link" onClick={() => setView('plan')}>Change</button></div>
              <div className="goal-fig"><b className="num">{goal.kind === 'minutes' ? fmtMin(done) : done}</b><span>of {goal.kind === 'minutes' ? fmtMin(goal.value) : plural(goal.value, 'topic')}</span></div>
              <div className="meter" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(pct)} aria-label="Goal progress"><i style={{ width: `${pct}%` }} /></div>
              <p className="small muted">{streak.current ? `${plural(streak.current, 'day')} in a row` : 'No streak yet'}{streak.best > streak.current ? `, best ${streak.best}` : ''}. The goal is a guide, not a rule.</p>
            </section>

            <section aria-labelledby="dl-h">
              <div className="sec-h"><h2 id="dl-h">Deadlines</h2><button className="link" onClick={() => setView('plan')}>Plan</button></div>
              {deadlines.length === 0 ? <p className="small muted">None coming up. Linked deadlines give their topics priority from three weeks out.</p> : (
                <ul className="dl-list">
                  {deadlines.map((d) => {
                    const n = daysBetween(today, d.date)
                    const cid = d.courseId ?? (d.docId ? data.docs.find((x) => x.id === d.docId)?.courseId : undefined)
                    return (
                      <li key={d.id} className={cid ? courseClass(courses.get(cid)) : 'nolink'}>
                        <span className="dl-date num">{parseDayKey(d.date).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}</span>
                        <span className="dl-title">{d.title}</span>
                        <span className={`dl-left num${n <= 7 ? ' due' : ''}`}>{n === 0 ? 'today' : n === 1 ? 'tomorrow' : `${n} days`}</span>
                      </li>
                    )
                  })}
                </ul>
              )}
            </section>

            <section aria-labelledby="c-h">
              <div className="sec-h"><h2 id="c-h">Courses</h2><button className="link" onClick={() => { setScope({ kind: 'all' }); setView('board') }}>Library</button></div>
              <ul className="course-list">
                {data.courses.map((c) => {
                  const rows = derived.filter((d) => d.t.courseId === c.id)
                  const due = rows.filter((r) => r.t.due && r.t.due <= now).length
                  const m = rows.length ? Math.round(rows.reduce((a, r) => a + r.m, 0) / rows.length) : 0
                  return (
                    <li key={c.id}>
                      <button className={`course-row ${courseClass(c)}`} onClick={() => { setScope({ kind: 'course', id: c.id }); setView('board') }}>
                        <span className="dot" /><span className="cr-name">{c.name}</span>
                        <span className={`cr-due num${due ? ' due' : ''}`}>{due ? `${due} due` : plural(rows.length, 'topic')}</span>
                        <MasteryBar v={m} />
                      </button>
                    </li>
                  )
                })}
              </ul>
            </section>
          </aside>
        </div>
      </div>
    </div>
  )
}
