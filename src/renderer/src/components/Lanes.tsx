// The interleave line: each course is a lane, and the study order is one thread that steps between lanes.
// Used on Today (the order coming up), in a session (what was done, what is next) and on the Study start panel.
// Pure SVG drawn from data; no animation loop. See DESIGN.md, "Signature motif".
import type { CSSProperties, ReactNode } from 'react'
import { courseClass, useApp } from '../state'

export type LaneState = 'done' | 'skipped' | 'current' | 'next' | 'later'

export interface LaneItem {
  key: string
  courseId: string
  state?: LaneState
  body: ReactNode
  aside?: ReactNode
  onClick?: () => void
  disabled?: boolean
  title?: string
}

const STEP = 14 // px between lanes
const PAD = 7 // first lane's centre

/** The courses the line passes through, in the library's course order. */
function laneOrder(items: LaneItem[], courseIds: string[]): string[] {
  const used = new Set(items.map((i) => i.courseId))
  const known = courseIds.filter((id) => used.has(id))
  return [...known, ...[...used].filter((id) => !known.includes(id))]
}

export function Lanes({ items, label, legend = true }: { items: LaneItem[]; label: string; legend?: boolean }) {
  const { data, courses } = useApp()
  const lanes = laneOrder(items, (data?.courses ?? []).map((c) => c.id))
  const x = (cid: string): number => PAD + Math.max(0, lanes.indexOf(cid)) * STEP
  const w = PAD * 2 + (lanes.length - 1) * STEP
  if (!items.length) return null
  return (
    <div className="lanes" style={{ '--lw': `${w}px` } as CSSProperties}>
      {legend && lanes.length > 1 && (
        <div className="lanes-legend" aria-hidden="true">
          {lanes.map((id) => <span key={id} className={courseClass(courses.get(id))}><i />{courses.get(id)?.name}</span>)}
        </div>
      )}
      <ol className="lanes-list" aria-label={label}>
        {items.map((it, i) => {
          const cur = x(it.courseId)
          const prev = i > 0 ? x(items[i - 1].courseId) : null
          const last = i === items.length - 1
          const st = it.state ?? 'later'
          const Tag = it.onClick ? 'button' : 'div'
          return (
            <li key={it.key} className={`lane-row ${st} ${courseClass(courses.get(it.courseId))}`}>
              <svg className="lane-svg" width={w} viewBox={`0 0 ${w} 100`} preserveAspectRatio="none" aria-hidden="true">
                {lanes.map((id) => <line key={id} className="lane-guide" x1={x(id)} x2={x(id)} y1={0} y2={100} />)}
                {prev !== null && (
                  <path className="lane-thread" d={`M${prev} 0 C${prev} 30, ${cur} 20, ${cur} 50`} />
                )}
                {!last && <line className="lane-thread" x1={cur} x2={cur} y1={50} y2={100} />}
              </svg>
              <span className="lane-dot" style={{ left: cur - 5 }} aria-hidden="true" />
              <Tag className="lane-body" {...(it.onClick ? { onClick: it.onClick, disabled: it.disabled, type: 'button' as const } : {})} title={it.title}>
                <span className="lane-main">{it.body}</span>
                {it.aside && <span className="lane-aside">{it.aside}</span>}
              </Tag>
            </li>
          )
        })}
      </ol>
    </div>
  )
}
