// Settings: AI provider (an API key in the Keychain, or a Claude scheduled task with its mailbox), model + Test
// model with capability probing, limits, study and Pomodoro, notifications, appearance, backup, capability check.
import { useEffect, useState } from 'react'
import type { Caps, Provider, SecretName, TestResult, ThinkingLevel } from '../../../shared/types'
import { DEFAULT_BASE_URLS, MODEL_PRESETS, PROVIDERS, SOUNDS, geminiLevelFor, geminiThinking, providerInfo } from '../../../shared/defaults'
import { api, errMsg } from '../api'
import { FileSupportList, useFileSupport } from '../components/FileSupport'
import { useAction, useApp } from '../state'
import { Icon } from '../icons'
import { Spinner } from '../ui'

function NumField({ label, value, onSave, min = 0, max, step = 1, suffix }: { label: string; value: number; onSave: (v: number) => void; min?: number; max?: number; step?: number; suffix?: string }) {
  const [v, setV] = useState(String(value))
  useEffect(() => setV(String(value)), [value])
  const commit = (): void => {
    // An empty or non-numeric entry is not a value: revert to what is stored and save nothing.
    const n = v.trim() === '' ? NaN : Number(v)
    if (!Number.isFinite(n)) { setV(String(value)); return }
    const c = Math.max(min, max !== undefined ? Math.min(max, n) : n)
    if (c !== value) onSave(c)
    setV(String(c))
  }
  return (
    <label className="field"><span>{label}</span>
      <span className="row-flex" style={{ flexWrap: 'nowrap' }}>
        <input className="input num" type="number" value={v} min={min} max={max} step={step} onChange={(e) => setV(e.target.value)} onBlur={commit} onKeyDown={(e) => e.key === 'Enter' && commit()} />
        {suffix && <span className="muted small">{suffix}</span>}
      </span>
    </label>
  )
}

function TextField({ label, value, placeholder, onSave }: { label: string; value: string; placeholder?: string; onSave: (v: string) => void }) {
  const [v, setV] = useState(value)
  useEffect(() => setV(value), [value])
  const commit = (): void => { if (v.trim() !== value) onSave(v.trim()) }
  return (
    <label className="field" style={{ maxWidth: 420 }}><span>{label}</span>
      <input className="input" value={v} placeholder={placeholder} onChange={(e) => setV(e.target.value)} onBlur={commit} onKeyDown={(e) => e.key === 'Enter' && commit()} aria-label={label} />
    </label>
  )
}

/** Advanced: point a provider at a proxy or the local mock server. Blank means the official endpoint. */
function BaseUrlField({ value, fallback, onSave }: { value: string; fallback: string; onSave: (v: string) => Promise<boolean> }) {
  const [v, setV] = useState(value)
  useEffect(() => setV(value), [value])
  const commit = (): void => {
    const t = v.trim().replace(/\/$/, '')
    if (t === value) { setV(value); return }
    // A rejected or cancelled save leaves the stored value as it was, so show that again.
    void onSave(t).then((ok) => { if (!ok) setV(value) })
  }
  return (
    <label className="field" style={{ marginTop: 10, maxWidth: 420 }}><span>Base URL (advanced)</span>
      <input className="input" value={v} placeholder={fallback} onChange={(e) => setV(e.target.value)} onBlur={commit} onKeyDown={(e) => e.key === 'Enter' && commit()} aria-label="Base URL" />
    </label>
  )
}

const THINK_LABEL: Record<Provider, string> = { gemini: 'Minimal thinking', anthropic: 'Low effort', openai: 'Low reasoning effort', routine: 'Low effort' }

const LEVEL_NAME: Record<ThinkingLevel, string> = { minimal: 'Minimal', low: 'Low', medium: 'Medium', high: 'High' }

/** A Gemini thinking level as the dropdown names it. Gemma's two levels are its off and on switch. */
function levelLabel(model: string, l: ThinkingLevel, levels: ThinkingLevel[], dflt: ThinkingLevel | null): string {
  if (/^gemma-4/i.test(model)) return l === 'minimal' ? 'Off' : 'On'
  const name = LEVEL_NAME[l] + (l === levels[0] ? ', fastest' : l === levels[levels.length - 1] ? ', slowest' : '')
  return l === dflt ? `${name} (model default)` : name
}

/** A secret typed once and saved to the Keychain; afterwards only "saved" is shown. */
function SecretField({ name, label, placeholder, saved, onChange }: { name: SecretName; label: string; placeholder: string; saved: boolean; onChange: () => Promise<void> }) {
  const act = useAction()
  const [v, setV] = useState('')
  return (
    <div className="field"><span>{label}{saved && <span className="tag ok">Saved in Keychain</span>}</span>
      <div className="row-flex" style={{ flexWrap: 'nowrap' }}>
        <input className="input" type="password" value={v} placeholder={saved ? 'Paste a new one to replace it' : placeholder} onChange={(e) => setV(e.target.value)} autoComplete="off" aria-label={label} />
        <button className="btn primary" disabled={!v.trim()} onClick={() => void act(async () => {
          const r = await api.setKey(v, name)
          if (!r.ok) throw new Error(r.error)
          setV('')
          await onChange()
        }, 'Saved')}>Save</button>
        {saved && <button className="btn ghost danger" onClick={() => void act(async () => { await api.clearKey(name); await onChange() }, 'Removed')}>Remove</button>}
      </div>
    </div>
  )
}

/**
 * Setting up the Claude scheduled task. The task is created on claude.ai (only there can its API trigger and token
 * be made); the app needs its address, its token, and a private GitHub repository the answers come back through.
 */
function RoutineSetup({ testing, test, runTest }: { testing: boolean; test: TestResult | null; runTest: () => void }) {
  const { settings, saveSettings, boot, refreshBoot, toast } = useApp()
  const act = useAction()
  const [info, setInfo] = useState<Awaited<ReturnType<typeof api.routineInfo>> | null>(null)
  const [creating, setCreating] = useState(false)
  const load = (): Promise<void> => api.routineInfo().then(setInfo, () => setInfo(null))
  useEffect(() => { void load() }, [])
  if (!settings || !boot) return null
  const r = settings.ai.routine
  const ready = !!r.repo && !!r.fireUrl && boot.keys.routine
  const createRepo = async (): Promise<void> => {
    setCreating(true)
    try {
      const res = await api.routineCreateRepo()
      if (!res.ok) throw new Error(res.error)
      await refreshBoot()
      toast(`Mailbox ready: ${res.repo}`)
    } catch (e) { toast(errMsg(e), true) } finally { setCreating(false) }
  }
  return (
    <div className="routine">
      <p className="help">
        Runs the AI work as a Claude scheduled task on your Claude plan, so no API key is billed. Requests are sent in batches of up to {r.batchMax}; each batch is one run and takes a minute or two. Claude allows 30 starts an hour for one task, and the app holds the next run when that is close. There is no daily count of runs: what runs out is your plan’s usage, which these runs share with everything else you do in Claude.
      </p>
      <ol className="steps">
        <li>
          <h3>Make the mailbox</h3>
          <p className="help">A private GitHub repository carries the requests to Claude and the answers back. Your lesson text passes through it, so it has to be private; the app checks that before sending anything and clears each batch once it is read.</p>
          {info && (info.github === 'gh'
            ? <p className="help">GitHub access: using your GitHub CLI sign-in.</p>
            : info.github === 'token' ? null
              : <p className="help">GitHub access: none found. Sign in with the GitHub CLI (<code>gh auth login</code>) and reopen Settings, or paste a token with the “repo” permission below.</p>)}
          {info && info.github !== 'gh' && <SecretField name="github" label="GitHub token" placeholder="ghp_… or github_pat_…" saved={boot.keys.github} onChange={async () => { await refreshBoot(); await load() }} />}
          <div className="row-flex" style={{ alignItems: 'end' }}>
            <TextField label="Mailbox repository" value={r.repo} placeholder="owner/name" onSave={(v) => void act(() => saveSettings({ ai: { routine: { repo: v } } }))} />
            <button className="btn" disabled={creating || !info?.github} onClick={() => void createRepo()}>{creating ? <Spinner /> : <Icon.plus />} {r.repo ? 'Create another' : 'Create a private mailbox for me'}</button>
          </div>
        </li>
        <li>
          <h3>Create the scheduled task on claude.ai</h3>
          <p className="help">Open <a href="https://claude.ai/code/routines">claude.ai/code/routines</a> and click <b>New task</b> (choose <b>Set up manually</b> if asked). Paste the prompt below, pick <b>API</b> as the trigger, and under <b>Advanced settings</b> choose the mailbox repository{r.repo ? <> (<span className="sel">{r.repo}</span>)</> : null}. When you click <b>Create</b> the token is shown once: copy it. The task URL is in the task’s <b>Edit</b> dialog, in the API trigger’s panel (<b>Fire URL</b>). A smaller model such as Sonnet uses less of your plan.</p>
          <div className="copybox">
            <p className="sel">{info?.prompt ?? '…'}</p>
            <button className="btn small" disabled={!info} onClick={() => void navigator.clipboard.writeText(info?.prompt ?? '').then(() => toast('Prompt copied'), () => toast('Could not copy; select the text instead', true))}><Icon.copy /> Copy prompt</button>
          </div>
        </li>
        <li>
          <h3>Connect it</h3>
          <TextField label="Task URL (shown with the API trigger)" value={r.fireUrl} placeholder="https://api.anthropic.com/v1/claude_code/routines/trig_…/fire" onSave={(v) => void act(() => saveSettings({ ai: { routine: { fireUrl: v } } }))} />
          <SecretField name="routine" label="Task token" placeholder="sk-ant-oat01-…" saved={boot.keys.routine} onChange={refreshBoot} />
        </li>
        <li>
          <h3>Check the setup</h3>
          <div className="row-flex">
            <button className="btn" disabled={testing} onClick={runTest}>{testing ? <Spinner /> : <Icon.check />} Check setup</button>
            <span className="muted small">{ready ? 'Checks the mailbox and the fields without starting a run.' : 'Finish the steps above first.'}</span>
          </div>
          {test && (
            <div style={{ marginTop: 10 }}>
              {test.ok ? <p>Ready. The first lesson you organize is the real test.</p> : <p className="err-text sel">{test.error}</p>}
              {test.notes.map((n) => <p key={n} className="help">{n}</p>)}
            </div>
          )}
        </li>
      </ol>
      <div className="grid3">
        <NumField label="Requests per run" value={r.batchMax} min={1} max={20} onSave={(v) => void saveSettings({ ai: { routine: { batchMax: v } } })} />
        <NumField label="Shortest time between runs" value={r.gapMinutes} min={0} max={240} suffix="min" onSave={(v) => void saveSettings({ ai: { routine: { gapMinutes: v } } })} />
        <NumField label="Give up waiting after" value={r.timeoutMinutes} min={5} max={180} suffix="min" onSave={(v) => void saveSettings({ ai: { routine: { timeoutMinutes: v } } })} />
      </div>
      <p className="help">A run that takes longer than the wait above pauses the queue; its answers are still collected when they arrive, and Continue never pays for the same request twice.</p>
    </div>
  )
}

export function SettingsView() {
  const { settings, saveSettings, boot, refreshBoot, toast } = useApp()
  const act = useAction()
  const [key, setKey] = useState('')
  const [model, setModel] = useState(settings?.ai.model ?? '')
  const [testing, setTesting] = useState(false)
  const [test, setTest] = useState<TestResult | null>(null)
  const [caps, setCaps] = useState<Caps | null>(null)
  useEffect(() => setModel(settings?.ai.model ?? ''), [settings?.ai.model])
  const files = useFileSupport()
  if (!settings || !boot) return null
  const ai = settings.ai
  const routine = ai.provider === 'routine'
  const info = providerInfo(ai.provider)
  const presets = MODEL_PRESETS[ai.provider]
  const thinkLabel = THINK_LABEL[ai.provider]
  const gt = geminiThinking(ai.model)
  const level = geminiLevelFor(ai)
  const mc = boot.modelCaps[ai.model]
  const yn = (v: boolean | null | undefined): string => (v === true ? 'yes' : v === false ? 'no' : 'not tested')

  const runTest = async (): Promise<void> => {
    setTesting(true)
    setTest(null)
    try {
      if (!routine && model.trim() !== ai.model) await saveSettings({ ai: { model: model.trim() } })
      setTest(await api.testModel(model.trim()))
      await refreshBoot()
    } catch (e) { toast(errMsg(e), true) } finally { setTesting(false) }
  }

  /** Save a model right away: picking a preset applies it, a typed ID applies on Enter or when leaving the field. */
  const chooseModel = async (m: string): Promise<void> => {
    const t = m.trim()
    setModel(t)
    if (!t || t === ai.model) return
    setTest(null)
    await saveSettings({ ai: { model: t } })
    await refreshBoot()
  }

  /** Switch provider and swap in that provider's model: the one last used with it, else its first preset. */
  const switchProvider = async (p: Provider): Promise<void> => {
    if (p === ai.provider) return
    setTest(null)
    setKey('')
    await saveSettings({ ai: { provider: p, model: ai.lastModels[p] ?? MODEL_PRESETS[p][0], lastModels: { [ai.provider]: ai.model } } })
    await refreshBoot()
  }

  return (
    <div className="page">
      <div className="page-inner settings">
        <section className="block">
          <h2>AI provider</h2>
          <label className="field" style={{ maxWidth: 300 }}><span>Provider</span>
            <select className="input" value={ai.provider} onChange={(e) => void switchProvider(e.target.value as Provider)} aria-label="AI provider">
              {PROVIDERS.map((p) => <option key={p.id} value={p.id}>{p.label}{boot.keys[p.id] ? ' (saved)' : ''}</option>)}
            </select>
          </label>
          <p className="help">Each provider keeps its own key. Switching keeps the other keys saved and changes the model list below.</p>
          {routine ? <RoutineSetup testing={testing} test={test} runTest={() => void runTest()} /> : <>
          <h3 style={{ marginTop: 14 }}>{info.keyName}</h3>
          <p className="help">{boot.hasKey ? 'A key is saved in the macOS Keychain. The page you are looking at never sees it.' : 'No key saved yet.'} Get a key at <a href={info.keyUrl} target="_blank" rel="noreferrer">{info.keyUrl.replace(/^https:\/\//, '')}</a>.</p>
          <div className="row-flex">
            <input className="input" type="password" style={{ maxWidth: 380 }} placeholder={boot.hasKey ? 'Paste a new key to replace it' : `Paste your API key (${info.keyPlaceholder})`} value={key} onChange={(e) => setKey(e.target.value)} autoComplete="off" aria-label={info.keyName} />
            <button className="btn primary" disabled={!key.trim()} onClick={() => void act(async () => {
              const r = await api.setKey(key, ai.provider)
              if (!r.ok) throw new Error(r.error)
              setKey('')
              await refreshBoot()
            }, 'Key saved')}>Save key</button>
            {boot.hasKey && <button className="btn ghost danger" onClick={() => void act(async () => { await api.clearKey(ai.provider); await refreshBoot() }, 'Key removed')}>Remove key</button>}
          </div>
          {ai.provider === 'gemini' && (
            <div className="notice">
              <b>Gemini has a free API tier.</b> Create a key in Google AI Studio with just a Google account, no credit card needed. The free tier has lower rate limits, and Gemma models on this API are free tier only. Privacy: Google says paid-tier content is not used to improve its products, which implies free-tier content may be.
            </div>
          )}
          {ai.provider === 'anthropic' && <p className="help">Claude API usage is billed per token through the Claude Console, separately from a Claude.ai subscription. Anthropic does not train on API content by default.</p>}
          {ai.provider === 'openai' && <p className="help">OpenAI API usage is billed per token through the OpenAI Platform, separately from a ChatGPT subscription. Requests are sent with storage turned off, and OpenAI does not train on API content by default.</p>}
          </>}
        </section>

        {!routine && <section className="block">
          <h2>Model</h2>
          <div className="row-flex">
            <select className="input" style={{ width: 210 }} value={presets.includes(model) ? model : ''} onChange={(e) => e.target.value && void chooseModel(e.target.value)} aria-label="Preset models">
              <option value="">Other model ID…</option>
              {presets.map((m) => <option key={m} value={m}>{m}</option>)}
            </select>
            <input className="input" style={{ maxWidth: 260 }} value={model} onChange={(e) => setModel(e.target.value)} onBlur={() => void chooseModel(model)} onKeyDown={(e) => e.key === 'Enter' && void chooseModel(model)} aria-label="Model ID" />
            <button className="btn" disabled={testing || !model.trim()} onClick={() => void runTest()}>{testing ? <Spinner /> : <Icon.check />} Test model</button>
          </div>
          {test && (
            <div style={{ marginTop: 10 }}>
              {test.ok ? <p>Working with <b>{test.model}</b> via {test.api} in {test.latencyMs} ms.{test.reply ? <span className="muted"> Reply: {test.reply}</span> : null}</p> : <p className="err-text sel">{test.error}</p>}
              {test.notes.map((n) => <p key={n} className="help">{n}</p>)}
            </div>
          )}
          {mc && <p className="help">Recorded for {ai.model}: endpoint {mc.api}, system instruction {yn(mc.system)}, JSON mode {yn(mc.json)}, {thinkLabel.toLowerCase()} {yn(mc.thinkMinimal)}{ai.provider === 'gemini' && mc.thinkLevel === 'low' ? ' (level low used)' : ''}{ai.provider === 'gemini' ? `, structured output ${yn(mc.schema)}` : ''}{ai.provider === 'anthropic' ? `, refusal fallback ${yn(mc.fallback)}` : ''}{mc.testedAt ? `, tested ${new Date(mc.testedAt).toLocaleString()}` : ''}.</p>}
          <div className="grid2" style={{ marginTop: 8 }}>
            {ai.provider === 'gemini' && (
              <label className="field"><span>Endpoint</span>
                <select className="input" value={ai.endpoint} onChange={(e) => void saveSettings({ ai: { endpoint: e.target.value as typeof ai.endpoint } })}>
                  <option value="auto">Automatic</option>
                  <option value="generateContent">generateContent</option>
                  <option value="interactions">Interactions API</option>
                </select>
              </label>
            )}
            {ai.provider === 'gemini' ? (
              <label className="field"><span>Thinking</span>
                <select className="input" value={level} onChange={(e) => void saveSettings({ ai: { thinking: { [ai.model]: e.target.value as ThinkingLevel } } })}>
                  {gt.levels.map((l) => <option key={l} value={l}>{levelLabel(ai.model, l, gt.levels, gt.dflt)}</option>)}
                </select>
              </label>
            ) : (
              <label className="check" style={{ alignSelf: 'end' }}><input type="checkbox" checked={ai.minimalThinking} onChange={(e) => void saveSettings({ ai: { minimalThinking: e.target.checked } })} /><span>Ask for {thinkLabel.toLowerCase()} (fewer output tokens; dropped automatically if the model refuses it)</span></label>
            )}
          </div>
          {ai.provider === 'gemini' && (
            <p className="help">
              {/^gemma-4/i.test(ai.model)
                ? 'Gemma can only turn thinking off or on. Off answers sooner. '
                : 'More thinking gives the model longer to reason before it answers, so each request is slower and uses more output tokens. '}
              {gt.known ? `Only the levels ${ai.model} accepts are listed, saved for this model.` : 'This model isn’t in Google’s list, so every level is offered; one it rejects is skipped for that request.'}
            </p>
          )}
          <BaseUrlField key={ai.provider} value={ai.baseUrls[ai.provider] ?? ''} fallback={DEFAULT_BASE_URLS[ai.provider]} onSave={async (v) => (await act(async () => { await saveSettings({ ai: { baseUrls: { [ai.provider]: v } } }); return true })) === true} />
        </section>}

        <section className="block">
          <h2>Lesson files</h2>
          <p className="help">{files && files.types.some((t) => t.kind === 'file')
            ? `${files.provider} gets PDFs${routine ? '' : ', Word files'} and pictures as the files themselves: it reads the text and sees every page, so tables, equations, figures and scans count. Other types go as text.`
            : 'Only Gemini and the Claude scheduled task are sent the files themselves. This provider gets the text the Mac extracts.'}</p>
          {files && <FileSupportList table={files} />}
          {routine && <p className="help" style={{ marginTop: 8 }}>Files go to the mailbox on a branch of their own, deleted once the run is over. GitHub may keep deleted data for a while, and the run's session on claude.ai keeps what it read, so use a mailbox only you can see.</p>}
        </section>

        <section className="block">
          <h2>Requests</h2>
          <p className="help">{routine
            ? 'Lessons are cut into windows; each window is one request, and requests travel together in runs. If the task stops answering or hits its limit, everything pauses and you get a notification; Continue picks up exactly where it stopped, without redoing finished work.'
            : `The app sends one request at a time, as fast as ${info.label.replace(/ \(.*\)$/, '')} allows. If the provider’s rate limit is reached, everything pauses and you get a notification; Continue picks up exactly where it stopped, without redoing finished work.`}</p>
          <div className="grid3">
            {routine
              ? <NumField label="Window size" value={ai.routine.windowTokens} min={2000} max={20000} step={500} suffix="input tokens" onSave={(v) => void saveSettings({ ai: { routine: { windowTokens: v } } })} />
              : <NumField label="Window size" value={ai.windowTokens} min={1000} max={8000} step={250} suffix="input tokens" onSave={(v) => void saveSettings({ ai: { windowTokens: v } })} />}
            <NumField label="Max output per request" value={ai.maxOutputTokens} min={300} max={8000} step={100} suffix="tokens" onSave={(v) => void saveSettings({ ai: { maxOutputTokens: v } })} />
          </div>
          <label className="check" style={{ marginTop: 10 }}><input type="checkbox" checked={ai.includeModelInCacheKey} onChange={(e) => void saveSettings({ ai: { includeModelInCacheKey: e.target.checked } })} /><span>Keep separate cached results per model. Off: switching models reuses earlier results and saves tokens.</span></label>
        </section>

        <section className="block">
          <h2>Study</h2>
          <div className="grid3">
            <NumField label="Minutes per topic (timed mode)" value={settings.study.minutes} min={1} max={240} onSave={(v) => void saveSettings({ study: { minutes: v } })} />
            <NumField label="Reading pace for suggested times" value={settings.study.pace} min={30} max={400} suffix="words/min" onSave={(v) => void saveSettings({ study: { pace: v } })} />
          </div>
          <label className="check" style={{ marginTop: 8 }}><input type="checkbox" checked={settings.study.useSuggested} onChange={(e) => void saveSettings({ study: { useSuggested: e.target.checked } })} /><span>Use each topic’s suggested time in timed mode</span></label>
        </section>

        <section className="block">
          <h2>Pomodoro</h2>
          <div className="grid3">
            <NumField label="Work" value={settings.pomo.work} min={1} suffix="min" onSave={(v) => void saveSettings({ pomo: { work: v } })} />
            <NumField label="Break" value={settings.pomo.brk} min={1} suffix="min" onSave={(v) => void saveSettings({ pomo: { brk: v } })} />
            <NumField label="Long break" value={settings.pomo.long} min={1} suffix="min" onSave={(v) => void saveSettings({ pomo: { long: v } })} />
            <NumField label="Work blocks before a long break" value={settings.pomo.cycles} min={1} onSave={(v) => void saveSettings({ pomo: { cycles: v } })} />
            <label className="field"><span>Topic length</span>
              <select className="input" value={settings.pomo.perTopic} onChange={(e) => void saveSettings({ pomo: { perTopic: e.target.value as 'one' | 'n' | 'minutes' } })}>
                <option value="one">1 Pomodoro per topic</option><option value="n">N Pomodoros per topic</option><option value="minutes">Custom minutes per topic</option>
              </select>
            </label>
            {settings.pomo.perTopic === 'n' && <NumField label="Pomodoros per topic" value={settings.pomo.n} min={1} onSave={(v) => void saveSettings({ pomo: { n: v } })} />}
            {settings.pomo.perTopic === 'minutes' && <NumField label="Minutes per topic" value={settings.pomo.minutes} min={1} suffix="split into work blocks" onSave={(v) => void saveSettings({ pomo: { minutes: v } })} />}
          </div>
          <label className="check" style={{ marginTop: 8 }}><input type="checkbox" checked={settings.pomo.autoStartNext} onChange={(e) => void saveSettings({ pomo: { autoStartNext: e.target.checked } })} /><span>Start the suggested topic automatically when a break ends</span></label>
        </section>

        <section className="block">
          <h2>Notifications</h2>
          <div className="row-flex">
            <label className="field" style={{ width: 200 }}><span>Sound</span>
              <select className="input" value={settings.notify.sound} onChange={(e) => void saveSettings({ notify: { sound: e.target.value } })}>{SOUNDS.map((s) => <option key={s}>{s}</option>)}</select>
            </label>
            <label className="check"><input type="checkbox" checked={settings.notify.bounce} onChange={(e) => void saveSettings({ notify: { bounce: e.target.checked } })} /><span>Bounce the Dock icon</span></label>
            <label className="check"><input type="checkbox" checked={settings.notify.badge} onChange={(e) => void saveSettings({ notify: { badge: e.target.checked } })} /><span>Badge the Dock icon</span></label>
            <button className="btn" onClick={() => void act(() => api.testNotification())}>Send a test notification</button>
          </div>
          <p className="help">If nothing appears, allow notifications for Interleaved Study in System Settings, Notifications.</p>
        </section>

        <section className="block">
          <h2>Appearance</h2>
          <div className="seg" role="group" aria-label="Theme">
            {(['system', 'light', 'dark'] as const).map((t) => <button key={t} aria-pressed={settings.theme === t} onClick={() => void saveSettings({ theme: t })}>{t === 'system' ? 'System' : t === 'light' ? 'Light' : 'Dark'}</button>)}
          </div>
        </section>

        <section className="block">
          <h2>Data</h2>
          <p className="help">
            Storage: SQLite database on this Mac (WAL, full-text search).
            {boot.caps.storeError && <span className="err-text"> {boot.caps.storeError}</span>}
          </p>
          <div className="row-flex">
            <button className="btn" onClick={() => void act(async () => { const p = await api.exportBackup(); if (p) toast(`Backup saved to ${p}`) })}><Icon.download /> Export backup</button>
            <button className="btn" onClick={() => void act(async () => {
              const r = await api.importBackup()
              if (r && !r.ok) throw new Error(r.error)
              if (r?.ok) { await refreshBoot(); toast('Backup restored') }
            })}><Icon.upload /> Import backup</button>
            <button className="btn ghost" onClick={() => void api.openDataFolder()}><Icon.folder /> Open data folder</button>
          </div>
          <p className="help">Backups contain everything except keys and tokens. Importing replaces all data and keeps your current keys and scheduled task setup; a safety copy of the current data is written to the data folder first.</p>
        </section>

        <section className="block">
          <h2>Capability check</h2>
          <button className="btn" onClick={() => void api.capsCheck().then(setCaps)}>Run check</button>
          {caps && (
            <dl className="facts" style={{ marginTop: 10 }}>
              <dt>Database</dt><dd>SQLite loaded</dd>
              <dt>Keychain</dt><dd>{caps.safeStorage ? 'available' : 'not available'}</dd>
              <dt>Notifications</dt><dd>{caps.notifications ? 'supported (permission is granted in System Settings)' : 'not supported'}</dd>
              <dt>Menu bar timer</dt><dd>{caps.tray ? 'created' : 'not created'}</dd>
              <dt>Dock badge and bounce</dt><dd>{caps.dock ? 'available' : 'not available on this platform'}</dd>
              <dt>Data folder</dt><dd className="sel">{caps.dataPath}</dd>
            </dl>
          )}
        </section>
      </div>
    </div>
  )
}
