// Mock for the "Claude scheduled task" provider: one server fakes both halves of the mailbox.
//   - the GitHub REST routes routine.rs uses (repo, contents, branches, git refs, /user, /user/repos), in memory;
//   - the scheduled task's fire endpoint (POST /fire), which "runs" the batch: after a delay it reads
//     inbox/<batch>.txt, writes outbox/<batch>/<request id>.json plus DONE on the branch claude/batch-<batch>.
//
// Standalone: `node scripts/mock-routine-server.mjs` (port 8788). Point the app at it with
//   ai.routine.fireUrl = http://127.0.0.1:8788/fire   and   ai.routine.githubApi = http://127.0.0.1:8788
//
// Behaviour is changed while it runs:
//   POST /_mock  { delayMs, fire429, retryAfter, fireStatus, skip, hold, branch, private }
//     delayMs    how long a run takes (default 6000)
//     fire429    answer the next N fires with 429 and Retry-After: retryAfter (seconds, default 120)
//     fireStatus answer the next fire with this status (e.g. 503, 401, or 400 for "paused")
//     skip       leave out one answer from each of the next N runs
//     hold       true: runs never finish until POST /_mock/release
//     branch     push answers to this branch instead of claude/batch-<id> ("" = the default branch)
//     private    false: the repository reports itself public
//   POST /_mock/release   finish every held run now
//   GET  /_mock           fires, files per branch, current settings
import http from 'node:http'
import { createHash } from 'node:crypto'
import { pathToFileURL } from 'node:url'
import { defaultAnswer } from './mock-ai-server.mjs'

const sha = (s) => createHash('sha1').update(s).digest('hex')

export function startMockRoutine(opts = {}) {
  const state = {
    cfg: { delayMs: opts.delayMs ?? 6000, fire429: 0, retryAfter: 120, fireStatus: 0, skip: 0, hold: false, branch: null, private: true },
    branches: new Map([['', new Map()]]), // '' is the default branch; other branches hold only what was pushed to them
    fires: [],
    held: [],
    log: []
  }
  const files = (b) => state.branches.get(b)
  // A branch sees its own files and, underneath, the default branch's.
  const read = (b, path) => (b && files(b)?.has(path) ? files(b).get(path) : b && !files(b) ? undefined : files('').get(path))
  const list = (b, dir) => {
    if (b && !files(b)) return []
    const all = new Map([...files(''), ...(b ? files(b) : [])])
    return [...all.keys()].filter((p) => p.startsWith(dir + '/') && !p.slice(dir.length + 1).includes('/'))
  }

  const finishRun = (run) => {
    const inbox = files('').get(`inbox/${run.batch}.txt`)
    if (!inbox) return state.log.push(`run ${run.batch}: inbox file missing, nothing written`)
    // Parse the batch the way CLAUDE.md describes it: only lines starting with the marker are structure.
    const marker = inbox.match(/marker (@@[0-9a-f]+)/)?.[1]
    const reqs = []
    let cur = null, part = null
    for (const line of inbox.split('\n').slice(1)) {
      const m = marker && line.startsWith(marker + ' ') ? line.slice(marker.length + 1).match(/^(?:REQUEST (\S+) (SYSTEM|PROMPT|FILE)(?: (\S+))?|END)$/) : null
      if (!m) { if (cur) cur[part] += line + '\n'; continue }
      if (!m[1]) break
      if (m[2] === 'FILE') {
        // As CLAUDE.md says: the file must be on files-<batch>, under a plain numbered name.
        const ok = /^[0-9]{1,3}\.(pdf|png|jpg|webp)$/.test(m[3] ?? '') && files(`files-${run.batch}`)?.has(m[3])
        state.log.push(`run ${run.batch}: ${m[1]} file ${m[3]} ${ok ? 'read' : 'MISSING'}`)
        if (cur) cur.file = ok ? m[3] : null
        continue
      }
      if (m[2] === 'SYSTEM') { cur = { id: m[1], system: '', prompt: '' }; reqs.push(cur) }
      part = m[2].toLowerCase()
    }
    const branch = state.cfg.branch ?? `claude/batch-${run.batch}`
    if (!state.branches.has(branch)) state.branches.set(branch, new Map())
    const out = files(branch)
    let skip = state.cfg.skip > 0 ? (state.cfg.skip--, 1) : 0
    let n = 0
    for (const r of reqs) {
      if (skip) { skip = 0; state.log.push(`run ${run.batch}: left out ${r.id}`); continue }
      out.set(`outbox/${run.batch}/${r.id}.json`, defaultAnswer(r.system, r.prompt))
      n++
    }
    out.set(`outbox/${run.batch}/DONE`, String(n))
    run.doneAt = Date.now()
    state.log.push(`run ${run.batch}: ${n}/${reqs.length} answers on ${branch || '(default branch)'}`)
  }

  const server = http.createServer((req, res) => {
    let raw = ''
    req.on('data', (c) => (raw += c))
    req.on('end', () => {
      let body = {}
      try { body = JSON.parse(raw || '{}') } catch { /* ignore */ }
      const url = new URL(req.url, 'http://x')
      const path = decodeURIComponent(url.pathname)
      const send = (status, obj, headers = {}) => {
        res.writeHead(status, { 'Content-Type': 'application/json', ...headers })
        res.end(typeof obj === 'string' ? obj : JSON.stringify(obj))
      }
      const token = (req.headers.authorization || '').replace(/^Bearer\s+/i, '')

      if (path === '/_mock' && req.method === 'POST') { Object.assign(state.cfg, body); return send(200, state.cfg) }
      if (path === '/_mock/release') { state.held.splice(0).forEach(finishRun); return send(200, { ok: true }) }
      if (path === '/_mock') {
        return send(200, {
          cfg: state.cfg, fires: state.fires, held: state.held.length, log: state.log,
          branches: Object.fromEntries([...state.branches].map(([b, m]) => [b || '(default)', [...m.keys()]]))
        })
      }

      if (path.endsWith('/fire') && req.method === 'POST') {
        const err = (status, type, message, headers) => send(status, { type: 'error', error: { type, message } }, headers)
        if (!token) return err(401, 'authentication_error', 'No bearer token')
        if (!req.headers['anthropic-version']) return err(400, 'invalid_request_error', 'anthropic-version header is required')
        if ((body.text ?? '').length > 65536) return err(400, 'invalid_request_error', 'text exceeds 65,536 characters')
        if (state.cfg.fire429 > 0) {
          state.cfg.fire429--
          state.fires.push({ at: Date.now(), status: 429 })
          return err(429, 'rate_limit_error', 'Hourly fire limit reached for this routine', { 'Retry-After': String(state.cfg.retryAfter) })
        }
        if (state.cfg.fireStatus) {
          const st = state.cfg.fireStatus
          state.cfg.fireStatus = 0
          state.fires.push({ at: Date.now(), status: st })
          return err(st, st === 400 ? 'invalid_request_error' : st === 503 ? 'overloaded_error' : 'api_error', st === 400 ? 'This routine is paused' : `Scripted ${st}`)
        }
        const batch = (body.text ?? '').match(/batch ([a-z0-9]{12})/)?.[1]
        const id = `session_${sha(String(Math.random())).slice(0, 24)}`
        const run = { at: Date.now(), status: 200, batch, text: body.text, session: id }
        state.fires.push(run)
        if (batch) { if (state.cfg.hold) state.held.push(run); else setTimeout(() => finishRun(run), state.cfg.delayMs) }
        return send(200, { type: 'routine_fire', claude_code_session_id: id, claude_code_session_url: `https://claude.ai/code/${id}` })
      }

      // ---- GitHub ----
      if (!token) return send(401, { message: 'Requires authentication' })
      if (path === '/user') return send(200, { login: 'mock-user' })
      if (path === '/user/repos' && req.method === 'POST') return send(201, { full_name: `mock-user/${body.name}`, private: true })
      const m = path.match(/^\/repos\/([^/]+\/[^/]+)(\/.*)?$/)
      if (!m) return send(404, { message: 'Not Found' })
      const rest = m[2] ?? ''
      if (rest === '') return send(200, { full_name: m[1], private: state.cfg.private, default_branch: 'main' })
      if (rest === '/branches') return send(200, [{ name: 'main' }, ...[...state.branches.keys()].filter(Boolean).map((name) => ({ name }))])
      const ref = rest.match(/^\/git\/refs\/heads\/(.+)$/)
      // Git Data API, enough for one parentless commit of blobs on its own branch.
      if (rest === '/git/blobs' && req.method === 'POST') {
        const id = sha(body.content ?? '')
        state.blobs = state.blobs ?? new Map()
        state.blobs.set(id, Buffer.from(body.content ?? '', 'base64'))
        return send(201, { sha: id })
      }
      if (rest === '/git/trees' && req.method === 'POST') {
        const id = sha(JSON.stringify(body.tree))
        state.trees = state.trees ?? new Map()
        state.trees.set(id, body.tree)
        return send(201, { sha: id })
      }
      if (rest === '/git/commits' && req.method === 'POST') {
        if ((body.parents ?? []).length) return send(422, { message: 'mock: file commits must have no parent' })
        const id = sha('c' + body.tree)
        state.commits = state.commits ?? new Map()
        state.commits.set(id, body.tree)
        return send(201, { sha: id })
      }
      if (rest === '/git/refs' && req.method === 'POST') {
        const name = (body.ref ?? '').replace(/^refs\/heads\//, '')
        if (state.branches.has(name)) return send(422, { message: 'Reference already exists' })
        const tree = state.trees?.get(state.commits?.get(body.sha)) ?? []
        state.branches.set(name, new Map(tree.map((t) => [t.path, `<${state.blobs?.get(t.sha)?.length ?? 0} bytes>`])))
        return send(201, { ref: body.ref })
      }
      if (ref && req.method === 'DELETE') return state.branches.delete(ref[1]) ? send(204, '') : send(422, { message: 'Reference does not exist' })
      const c = rest.match(/^\/contents\/(.+)$/)
      if (!c) return send(404, { message: 'Not Found' })
      const p = c[1]
      const branch = url.searchParams.get('ref') ?? ''
      if (branch === 'main') return send(404, { message: 'mock: use no ref for the default branch' })
      if (req.method === 'GET') {
        const text = read(branch, p)
        if (text !== undefined) {
          if ((req.headers.accept || '').includes('raw')) { res.writeHead(200, { 'Content-Type': 'text/plain' }); return res.end(text) }
          return send(200, { type: 'file', name: p.split('/').pop(), path: p, sha: sha(text), content: Buffer.from(text).toString('base64'), encoding: 'base64' })
        }
        const kids = list(branch, p)
        if (kids.length) return send(200, kids.map((k) => ({ type: 'file', name: k.split('/').pop(), path: k, sha: sha(read(branch, k)) })))
        return send(404, { message: 'Not Found' })
      }
      if (req.method === 'PUT') {
        const had = files('').get(p)
        if (had !== undefined && body.sha !== sha(had)) return send(409, { message: 'sha does not match' })
        files('').set(p, Buffer.from(body.content ?? '', 'base64').toString())
        return send(had === undefined ? 201 : 200, { content: { path: p } })
      }
      if (req.method === 'DELETE') {
        const had = files('').get(p)
        if (had === undefined) return send(404, { message: 'Not Found' })
        if (body.sha !== sha(had)) return send(409, { message: 'sha does not match' })
        files('').delete(p)
        return send(200, { commit: {} })
      }
      send(405, { message: 'Method not allowed' })
    })
  })
  return new Promise((resolve) => {
    server.listen(opts.port ?? 0, '127.0.0.1', () => resolve({ url: `http://127.0.0.1:${server.address().port}`, state, close: () => new Promise((r) => server.close(r)) }))
  })
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  startMockRoutine({ port: Number(process.env.PORT) || 8788 }).then((s) => {
    console.log(`Mock scheduled task + GitHub mailbox listening on ${s.url}`)
    console.log(`  fireUrl:   ${s.url}/fire\n  githubApi: ${s.url}\n  control:   POST ${s.url}/_mock   state: GET ${s.url}/_mock`)
  })
}
