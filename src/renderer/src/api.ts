// The bridge to the Rust core. Every method is one `api` command; events arrive as `ev:<name>`.
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { API_METHODS, type Api, type ApiEvent } from '../../shared/api'

export interface Bridge extends Api {
  on(event: ApiEvent, cb: (payload: unknown) => void): () => void
  /** Files or folders dragged from Finder onto the window. */
  onFileDrop(cb: (e: { type: 'over' | 'leave' } | { type: 'drop'; paths: string[] }) => void): () => void
}

const inApp = '__TAURI_INTERNALS__' in window
// Development only: the page opened in a browser talks to a debug build's loopback bridge instead.
const DEV_BRIDGE = import.meta.env.DEV && !inApp ? `http://127.0.0.1:${import.meta.env.VITE_DEV_BRIDGE_PORT ?? '5184'}` : null

async function call(method: string, args: unknown[]): Promise<unknown> {
  if (DEV_BRIDGE) {
    const r = (await (await fetch(`${DEV_BRIDGE}/api`, { method: 'POST', body: JSON.stringify({ method, args }) })).json()) as { ok: boolean; value?: unknown; error?: string }
    if (!r.ok) throw new Error(r.error)
    return r.value
  }
  try {
    return await invoke('api', { method, args })
  } catch (e) {
    throw new Error(typeof e === 'string' ? e : e instanceof Error ? e.message : String(e))
  }
}

const devListeners = new Map<string, Set<(p: unknown) => void>>()
if (DEV_BRIDGE) {
  new EventSource(`${DEV_BRIDGE}/events`).onmessage = (m) => {
    const { event, payload } = JSON.parse(m.data as string) as { event: string; payload: unknown }
    devListeners.get(event)?.forEach((cb) => cb(payload))
  }
}

/** Run `subscribe` now; the returned function unsubscribes even if it is called before the subscription settles. */
function later(subscribe: () => Promise<() => void>): () => void {
  let off: (() => void) | null = null
  let gone = false
  void subscribe().then((un) => { if (gone) un(); else off = un })
  return () => { gone = true; off?.() }
}

const bridge: Record<string, unknown> = {}
for (const m of API_METHODS) bridge[m] = (...args: unknown[]) => call(m, args)
bridge.on = (event: ApiEvent, cb: (payload: unknown) => void): (() => void) => {
  if (DEV_BRIDGE) {
    const set = devListeners.get(event) ?? new Set()
    devListeners.set(event, set.add(cb))
    return () => { set.delete(cb) }
  }
  return later(() => listen(`ev:${event}`, (e) => cb(e.payload)))
}
bridge.onFileDrop = (cb: Parameters<Bridge['onFileDrop']>[0]): (() => void) => {
  if (!inApp) return () => undefined
  return later(() => getCurrentWebview().onDragDropEvent((e) => {
    const p = e.payload
    if (p.type === 'drop') cb({ type: 'drop', paths: p.paths })
    else cb({ type: p.type === 'leave' ? 'leave' : 'over' })
  }))
}

export const api = bridge as unknown as Bridge

export function errMsg(e: unknown): string {
  return e instanceof Error ? e.message : String(e)
}

/** The AI request was cancelled (by the person, or because the window that wanted it went away). */
export const isCancelled = (e: unknown): boolean => errMsg(e) === 'Cancelled'
