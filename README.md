# Interleaved Study (Tauri + Rust)

Interleaved practice with spaced repetition for lesson documents. You import lessons (Markdown, text, PDF, DOCX, and
pictures with Gemini or the scheduled task), an AI
splits them into topics, and the app schedules timed or pomodoro study sessions that mix topics across lessons and
courses. It also tracks deadlines, a daily goal and progress.

This is version 2: the Rust rewrite of the Electron app in `../interleaved-study`. The interface is the same React
code; everything behind it (database, AI queue, timer, keys, menu, tray, notifications) is a Rust core running in a
Tauri 2 shell. macOS is the only platform it is built and tested for.

## Requirements

- macOS 12 or later
- Node 20+ and npm
- Rust (stable) through rustup. If `cargo` is not on your PATH:
  ```bash
  export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
  ```
- Xcode command line tools

## Commands

| Command | What it does |
| --- | --- |
| `npm install` | Install the JavaScript dependencies |
| `npm run dev` | Run the app with hot reload (Vite on `127.0.0.1:5183`, debug Rust build) |
| `npm run build` | Typecheck, build the UI and produce the release `.app` |
| `npm run typecheck` | TypeScript only |
| `npm test` | Rust unit tests (`cd src-tauri && cargo test --lib` runs the same 45 tests) |
| `npm run mock-ai` | Mock AI server on `127.0.0.1:8787` (Gemini, Claude and OpenAI routes; `GET /_mock` lists requests and the files they carried, `POST /_mock {"rejectFiles":"mime"\|"bare"}` refuses files) |
| `npm run mock-routine` | Mock Claude scheduled task and GitHub mailbox on `127.0.0.1:8788` (see the top of `scripts/mock-routine-server.mjs`) |

The release bundle lands in `src-tauri/target/release/bundle/macos/Interleaved Study.app`. It is ad-hoc signed, not
notarized, so it is meant for this Mac; copy it to `/Applications` to install.

## Where things are stored

- **Library:** `~/Library/Application Support/Interleaved Study 2/study.sqlite` (SQLite, WAL, FTS5 search).
- **First run:** if that folder has no library yet and the Electron app's
  `~/Library/Application Support/Interleaved Study/study.sqlite` exists, it is copied over. The old folder is only
  read. The copy happens once, so anything studied in the old app afterwards does not follow; to adopt again, quit the
  app and delete the `Interleaved Study 2` folder.
- **API keys:** one macOS Keychain item (service `Interleaved Study`, account `api-keys`). Keys are never written to
  the database or to a backup. Keys saved by the Electron app are not carried over; enter them again in Settings.
  Because the build is ad-hoc signed, macOS may ask for Keychain access again after a rebuild.
- **Backups:** File > Export Backup writes one JSON file with everything except keys. The format is the same as the
  Electron app's (restoring an Electron backup here, or the reverse, has not been tested). Restore keeps this machine's keys, provider addresses and
  scheduled-task setup, and first writes a `before-restore-<time>.json` safety copy next to the library.

## AI providers

Google Gemini, Anthropic Claude and OpenAI over their HTTPS APIs with your own key, plus a "Claude scheduled task"
provider that uses a Claude subscription through a GitHub mailbox repository (see the comment at the top of
`src-tauri/src/routine.rs`).

Requests go through one persisted queue (`src-tauri/src/queue.rs`):

- one request at a time for API providers;
- HTTP 429 pauses everything and keeps all work; **Continue** on the Queue page picks up where it stopped;
- 5xx and network errors retry with backoff, up to 6 attempts, then the job is marked failed and can be retried;
- any queued or running job can be cancelled, and the request in flight stops at once;
- finished requests are cached, so a retry or re-import does not pay twice.

### Lesson files sent as files

Gemini (at Google's own address) and the Claude scheduled task get PDFs and pictures as the files themselves, so the
model reads the text and an image of every page; Gemini also gets Word files. Claude API and OpenAI get the text the
Mac extracts. Settings > Lesson files, the import dialog and each lesson row say which applies (`src-tauri/src/files.rs`
is the one table both the core and the UI use).

- The file is read from disk when the request goes out, checked against the hash taken at import and against its
  type's signature, and never stored in the database or a job. A moved or changed file goes as text, with a note.
- A scanned PDF or a picture (no text on the Mac) gets a `read` job first: the model writes out its text, which becomes
  the lesson, then it is organized as usual.
- Limits: Gemini 15 MB inline; the scheduled task 20 MB and 100 PDF pages. Larger files go as extracted text.
- A 400 while a file is attached retries once without it and learns nothing else; only an explicit "MIME type not
  supported" is remembered for that model. An answer given without the file is never cached as if it had the file.
- Scheduled task: a batch's files go up as one commit with no parent on the branch `files-<batch>` (numbered names,
  never the file name), deleted when the batch ends, fails or is given up on; launch sweeps leftovers. The mailbox's
  privacy is checked again before every upload. GitHub may keep deleted objects for a while, and the run's claude.ai
  session keeps what it read.

Changing a provider's base URL to a non-official host asks for confirmation in a native dialog, because requests
carry the API key.

## Layout

```
src/renderer/        React UI (views, components, state.tsx, api.ts = the bridge to the core)
src/shared/          Types and pure helpers shared with the UI (API method list, scheduling, streaks)
src-tauri/src/
  lib.rs             App setup: window, menu, tray, notifications, one-second heartbeat
  commands.rs        Every call the window can make, dispatched by method name
  core.rs            App state and domain operations (courses, lessons, topics, reviews)
  store.rs           SQLite store; same table layout as the Electron version
  queue.rs           AI request queue: pause, rate limits, retries, cancel
  jobs.rs            Job handlers: organize, dedupe, enrich, quiz, grade, deadlines
  provider.rs        Gemini / Claude / OpenAI adapters, per-model capability fallback
  routine.rs         Claude scheduled-task provider
  importer.rs        File and folder import (md, txt, pdf, docx, pictures)
  files.rs           Which file types go to which provider as files; loading an attachment
  preprocess.rs      Local text cleanup, paragraph numbering, AI windows
  organize.rs        Organize prompts, validation, merging
  session.rs         Timed and pomodoro session state machine
  sched.rs           Spaced-repetition scheduler and mastery model
  deadlines.rs       Spreadsheet reading (xlsx, xls, csv) and deadline extraction
  keys.rs            Keychain storage
  settings.rs        Defaults, migrations, base-URL safety check
  devbridge.rs       Debug builds only: the API over loopback HTTP (see below)
scripts/mock-ai-server.mjs   Mock AI API with scripted failures
```

## Testing without touching your library or spending tokens

Three environment variables exist for development:

| Variable | Effect |
| --- | --- |
| `INTERLEAVED_STUDY_DATA=<folder>` | Use this folder for the library instead of Application Support (and skip adoption) |
| `INTERLEAVED_STUDY_KEYS='{"gemini":"test-key"}'` | Supply keys from the environment; the Keychain is not read or written |
| `INTERLEAVED_STUDY_DEV_PORT=5184` | Debug builds only: serve the API and events on this loopback port |

A typical test run against a copy of the real library:

```bash
mkdir -p /tmp/is-test && cp ~/Library/Application\ Support/Interleaved\ Study\ 2/study.sqlite /tmp/is-test/
```

```bash
npm run mock-ai
```

```bash
INTERLEAVED_STUDY_DATA=/tmp/is-test INTERLEAVED_STUDY_KEYS='{"gemini":"test-key"}' INTERLEAVED_STUDY_DEV_PORT=5184 npm run dev
```

With the dev port set, the UI at `http://127.0.0.1:5183` also works in an ordinary browser, and the core can be
driven directly:

```bash
curl -s -X POST 127.0.0.1:5184/api -d '{"method":"queueStatus","args":[]}'
```

`GET /events` on the same port is a server-sent event stream of what the core emits (`queue`, `session`, `data`).
The dev bridge is compiled out of release builds.

To send AI requests to the mock, set the Gemini base URL to `http://127.0.0.1:8787` in Settings (the app asks for
confirmation). The mock answers every prompt sensibly by default and can be scripted to fail (429 with Retry-After,
5xx, invalid JSON, stalled responses); see the header of `scripts/mock-ai-server.mjs`.

## Native behaviour worth knowing

- Closing the window hides it; the app and the menu bar timer keep running. Click the menu bar icon or the Dock icon
  to bring it back. Cmd+Q quits.
- The window's size and position are saved when it closes and when the app quits.
- Shortcuts: Cmd+1 to Cmd+5 switch views, Cmd+, opens Settings, Cmd+O / Cmd+Shift+O import files or a folder,
  Cmd+Shift+S starts the next suggested session, Cmd+Shift+P pauses or resumes the timer.
- Destructive confirmations (restore, delete, custom base URL) never have the destructive button as the Return-key
  default.

## Known limits

- Without Gemini or the scheduled task, PDF and DOCX text is extracted on the Mac (best effort); Markdown gives the
  cleanest topics.
- Not notarized or signed with a developer identity.
- macOS only.
