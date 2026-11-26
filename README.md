# Interleaved Study

A Mac app that turns your lecture files into study topics, then runs timed study blocks that mix (interleave) topics
across lessons and courses, with spaced-repetition reviews, deadlines and a daily goal.

Import a lesson (Markdown, text, PDF, Word or a picture) and an AI splits it into topics. The app then decides what to
study next: what is due for review, what is close to a deadline, and an order that switches between courses instead of
working through one block at a time. You press Start and study; the timer, breaks and reviews are handled for you.

It is a native Mac app: the interface is SwiftUI and AppKit (no web view), and the engine underneath (storage,
scheduling, sessions and the AI queue) is Rust, linked into the app as a static library. Earlier versions (Electron,
then Tauri with a web interface) are in this repository's history. macOS 14 or later.

## What it does

- **Today:** what is due, the suggested study order, your daily goal and the next deadlines, with one Start button.
- **Study:** timed or pomodoro sessions that step between topics from different courses. Shows the topic, the lecture
  file to open and the clock. You rate each block when it ends, and can take a quick quiz on the topic.
- **Library:** courses, lessons and topics, with search, and an inspector to edit or re-organize any of them.
- **Plan:** deadlines on a calendar. Deadlines can be read from a spreadsheet (xlsx, xls, csv).
- **Progress:** mastery per course and topic, streaks and study history.
- **AI Queue:** every AI request in one queue you can pause, retry or cancel.
- Menu bar timer, notifications, and keyboard shortcuts for everything you do daily.

## The design: "Lanes"

Each course is a lane, and the study order is one thread that steps between lanes. Mixing courses is something you
see, not a setting: the line appears on Today (the order coming up), during a session (what you did and what is next)
and in the toolbar. The chrome is ink on warm paper, with a dark mode; course colours are the only colours. Topic names
use a reading serif, and motion is limited to small feedback so it never pulls attention from studying.

The full design contract, with tokens and rules, is in [DESIGN.md](DESIGN.md). Read it before changing the interface.

## Build and install

You need:

- macOS 14 or later, with Xcode (or its command line tools) for Swift 6
- Rust (stable) through rustup. If `cargo` is not on your PATH:
  ```bash
  export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
  ```

Then:

```bash
git clone https://github.com/20clouds/Interleaved-Study-App.git
```

```bash
cd Interleaved-Study-App && scripts/build-app.sh
```

The app lands in `build/Interleaved Study.app`. Copy it to `/Applications` to install. It is ad-hoc signed, not
notarized, so macOS may ask you to confirm the first time you open it.

| Command | What it does |
| --- | --- |
| `scripts/build-app.sh` | Build the engine and the app (release) into `build/Interleaved Study.app` |
| `scripts/build-app.sh debug` | The same as a debug build, with the engine's dev bridge |
| `cargo test --manifest-path engine/Cargo.toml` | Engine unit tests |
| `node scripts/mock-ai-server.mjs` | Mock AI server on `127.0.0.1:8787` (Gemini, Claude and OpenAI routes, scriptable failures) |
| `node scripts/mock-routine-server.mjs` | Mock Claude scheduled task and GitHub mailbox on `127.0.0.1:8788` |

The Swift package in `app/` can also be opened in Xcode; build the engine first (the script copies it to
`app/build/lib/`).

## AI providers

Use Google Gemini, Anthropic Claude or OpenAI with your own API key, or the "Claude scheduled task" provider, which
runs the work on a Claude subscription through a private GitHub mailbox repository (see the comment at the top of
`engine/src/routine.rs`).

All requests go through one saved queue (`engine/src/queue.rs`):

- one request at a time for API providers;
- a rate limit (HTTP 429) pauses everything and keeps all work; **Continue** on the AI Queue page picks up where it
  stopped;
- server and network errors retry with backoff, up to 6 attempts, then the job is marked failed and can be retried;
- any queued or running job can be cancelled, and the request in flight stops at once;
- finished requests are cached, so a retry or re-import does not pay twice.

### Lesson files sent as files

Gemini and the scheduled task receive PDFs and pictures as the files themselves, so the model reads the text and an
image of every page; Gemini also receives Word files. Claude API and OpenAI receive the text the Mac extracts.
Settings > Lesson files, the import dialog and each lesson row say which applies (`engine/src/files.rs`).

- The file is read from disk when the request goes out and checked against the hash taken at import. It is never
  stored in the database. A moved or changed file goes as text, with a note.
- A scanned PDF or a picture with no text first gets a `read` job: the model writes out its text, which becomes the
  lesson.
- Limits: Gemini 15 MB; the scheduled task 20 MB and 100 PDF pages. Larger files go as extracted text.
- Scheduled task: a batch's files go up on a temporary branch of the mailbox (numbered names, never the file name)
  that is deleted when the batch ends. The mailbox must be private; this is checked again before every upload.

Changing a provider's base URL to an unofficial host asks for confirmation first, because requests carry your API key.

## Your data and keys

- **Library:** `~/Library/Application Support/Interleaved Study 2/study.sqlite` (SQLite with full-text search).
- **API keys:** stored in the macOS Keychain as one item (service `Interleaved Study`, account `api-keys`). They are
  never written to the database, a backup or this repository. Because the build is ad-hoc signed, macOS may ask for
  Keychain access again after a rebuild.
- **Backups:** File > Export Backup writes one JSON file with everything except keys. Restoring first saves a
  `before-restore-<time>.json` copy next to the library and keeps this Mac's keys and provider setup.

## Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| Cmd+1 … Cmd+6 | Today, Study, Library, Plan, Progress, AI Queue |
| Cmd+, | Settings |
| Cmd+O / Cmd+Shift+O | Import lessons / import a folder |
| Cmd+Shift+S | Start the next suggested session |
| Cmd+Shift+P | Pause or resume the timer |
| Space, N, 1–4, Return, X, S | In a session: pause, skip, rate, save or start, something else; S starts studying |

Closing the window keeps the app and the menu bar timer running. Click the Dock icon, or choose Open Interleaved Study
from the menu bar timer's menu, to bring it back; Cmd+Q quits. Confirmations for destructive actions never make the destructive button the Return default.

## Project layout

```
DESIGN.md            The interface's design contract ("Lanes")
app/                 The Mac app (Swift package)
  Sources/InterleavedStudy/
    App.swift        Window, Settings window, menu bar timer and menus
    Engine.swift     The bridge to the engine: API calls, events, dialogs and panels it asks for
    AppModel.swift   App state, actions, notifications
    Models.swift     The engine's records as Swift types
    Schedule.swift   Mastery, status, streaks, date keys, formatting, providers and models
    Theme.swift      Design tokens (colours, type, course hues) and small shared views
    Lanes.swift      The interleave line
    RootView.swift   Sidebar, toolbar strip and clock, banners, file drops, one-key shortcuts
    TodayView, StudyView, QuizPanel, LibraryView, InspectorView, PlanView, DeadlineSheets,
    ProgressScreen, QueueView, SettingsView, ImportFlow
  Sources/CEngine/   The engine's C header
  Resources/         Info.plist, app icon, menu bar image
engine/src/
  ffi.rs             The C interface the app calls: start, call a method, stop
  host.rs            What the engine asks the app for: confirmation sheets, open and save panels
  commands.rs        Every API method, dispatched by name
  core.rs            App state and domain operations (courses, lessons, topics, reviews)
  store.rs           SQLite store
  types.rs           Stored records
  queue.rs           AI request queue: pause, rate limits, retries, cancel
  jobs.rs            Job handlers: organize, dedupe, enrich, quiz, grade, deadlines
  provider.rs        Gemini / Claude / OpenAI adapters, per-model capability fallback
  routine.rs         Claude scheduled-task provider
  aijson.rs          Reading JSON out of model answers safely
  importer.rs        File and folder import (md, txt, pdf, docx, pictures)
  files.rs           Which file types go to which provider as files
  preprocess.rs      Local text cleanup, paragraph numbering, AI windows
  organize.rs        Organize prompts, validation, merging
  history.rs         Keeps study history when topics are re-imported or re-organized
  session.rs         Timed and pomodoro session state machine
  sched.rs           Spaced-repetition scheduler and mastery model
  deadlines.rs       Spreadsheet reading and deadline extraction
  keys.rs            Keychain storage
  settings.rs        Defaults, migrations, base-URL safety check
  util.rs            Shared helpers (dates, names, ordering)
  devbridge.rs       Debug builds only: the API over loopback HTTP
scripts/             build-app.sh, and the mock AI and mock scheduled-task servers
```

## Testing without touching your library or spending tokens

| Variable | Effect |
| --- | --- |
| `INTERLEAVED_STUDY_DATA=<folder>` | Debug builds only: use this folder for the library instead of Application Support |
| `INTERLEAVED_STUDY_KEYS='{"gemini":"test-key"}'` | Debug builds only: supply keys from the environment; the Keychain is not touched |
| `INTERLEAVED_STUDY_DEV_PORT=5184` | Debug builds only: serve the API and events on this loopback port |

A typical run against a copy of your library and the mock AI server:

```bash
mkdir -p /tmp/is-test && cp ~/Library/Application\ Support/Interleaved\ Study\ 2/study.sqlite /tmp/is-test/
```

```bash
node scripts/mock-ai-server.mjs
```

```bash
scripts/build-app.sh debug && INTERLEAVED_STUDY_DATA=/tmp/is-test INTERLEAVED_STUDY_KEYS='{"gemini":"test-key"}' INTERLEAVED_STUDY_DEV_PORT=5184 "build/Interleaved Study.app/Contents/MacOS/Interleaved Study"
```

Then set the Gemini base URL to `http://127.0.0.1:8787` in Settings. With the dev port set, the engine can also be
driven directly:

```bash
curl -s -X POST 127.0.0.1:5184/api -d '{"method":"queueStatus","args":[]}'
```

`GET /events` on the same port streams what the engine emits (`queue`, `session`, `data`). The dev bridge is compiled
out of release builds, and it refuses browsers (any request with an `Origin` header, or a Host other than
`127.0.0.1` or `localhost` on that port), so a web page cannot reach it.

## Known limits

- macOS only, and not notarized or signed with a developer identity.
- Without Gemini or the scheduled task, PDF and Word text is extracted on the Mac (best effort); Markdown gives the
  cleanest topics.
