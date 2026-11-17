# DESIGN.md: Interleaved Study
> The design contract for the app's interface. Re-read before every UI task. Views use these tokens only
> (`app/Sources/InterleavedStudy/Theme.swift`). Written 2026-11-17 with the site-design-director skill, adapted from
> websites to a Mac app (no marketing page order, no LCP or scroll reveals, product-UI motion only). Since the native
> rewrite (2026-11-17) the interface is SwiftUI: standard Mac structure (source-list sidebar, toolbar, outline table,
> inspector, Settings window, menu bar extra, sheets) carries these tokens; CSS names below map to `Ink.*`,
> `Hue.color(_:)` and `Typeset.*`.

## Brief
- **Subject:** a Mac app that turns a student's lecture files into topics, then runs timed study blocks that mix
  (interleave) topics across lessons and courses, with spaced-repetition reviews, deadlines and a daily goal.
- **Audience:** one university student, at their own Mac, every day, for sessions of 30 minutes to 3 hours.
  Knows the app; not sceptical; wants to start the right block in one keystroke and not think about scheduling.
- **The app's one job:** start (and stay in) the right study block now.
- **Tone words:** hushed, exact, unhurried.
- **Memorable element:** the *interleave line*. Each course is a lane; the study order is one thread that
  steps between lanes, so mixing courses is something you see, not a setting. It appears on Today (the order
  coming up), in a session (what you did and what is next) and in the toolbar strip.
- **Real materials:** the student's own courses, lessons, topics, deadlines and study history (shown as they are).
  No logo beyond the existing app icon. No invented numbers anywhere.

## Niche and Spectacle
- **Niche playbook:** education × fintech-like data density, adapted to a desktop tool: calm, plain, specific;
  tabular numbers; colour used functionally.
- **Spectacle level:** 1 (Alive) because this is a tool used for hours; motion must never pull attention from
  reading. Only the block-advance moment and hover/press feedback move.
- **Signature motif:** the interleave line, because interleaving across courses is the method the app exists for.
- **Technology:** drawn from data with SwiftUI `Canvas` (`Lanes.swift`), no libraries, no animation loop.
- **Reduced motion:** every transition is removed; nothing depends on motion to be understood.

## Direction
- **Chosen: "Lanes".** Today is the home: what is due, the order the app suggests drawn as the interleave line,
  the goal and the next deadlines, with one Start button. A running session becomes a quiet focus stage: the topic
  in a large reading serif, the lecture file to open, one large clock, the lane history beside it. The chrome is
  ink on warm paper-white; the course colours are the only hues, and they are the threads.
- **Rejected:** *Index cards* (topics as a card file, sessions flip cards): suits flash-card recall, not timed
  reading blocks, and lands on the cream-serif default. *Instrument panel* (decay curves and gauges everywhere,
  mono type): puts analysis ahead of studying, and mono there would only be costume.
- **Replaces** the 2026-11-10 "refined native Mac" direction at the user's request (redesign from scratch with
  this skill). Kept from it, because they are Mac conventions rather than style: system fonts, labelled sidebar,
  window traffic-light inset, native confirmation sheets, light and dark.

## Tokens
### Colour
Chrome holds no hue. The primary button and the active state are ink. Course hues (`.c0`–`.c7` or a picked hue)
carry identity: lanes, threads, dots, calendar chips, the course bar on a shelf card.

| Token | Light | Dark | Role |
|---|---|---|---|
| --paper | #FBFAF8 | #161615 | window ground |
| --panel | #F3F2EF | #1E1E1C | sidebar, side panes, quiet bands |
| --raised | #FFFFFF | #252523 | fields, cards, popovers |
| --ink | #1C1B19 | #EEEDEA | primary text, primary button fill |
| --ink-2 | #5F5D58 | #A5A39D | secondary text (≥4.5:1 on paper) |
| --ink-3 | #8C8A84 | #77756F | tertiary marks only, never reading text |
| --line | rgba(28,27,25,.10) | rgba(255,255,250,.09) | hairlines |
| --line-2 | rgba(28,27,25,.18) | rgba(255,255,250,.16) | control borders |
| --hover | rgba(28,27,25,.045) | rgba(255,255,250,.05) | hover fill |
| --sel | rgba(28,27,25,.075) | rgba(255,255,250,.09) | selected row / nav |
| --due | #B4331E | #F08A70 | overdue, close deadline, failure (status, not accent) |
| --ok | #2F7A4B | #6CC48E | saved, correct |
| --focus | #2F6FD6 | #5E9BFF | focus ring and text selection only |

Course hue: `oklch(var(--c-l) var(--c-ch) H)`, light L .58 C .13, dark L .72 C .11. Tints use `color-mix` with paper.

### Type
System faces only (offline app, no font files; CSP allows none from the network). Chosen, not defaulted:
SF Pro for the interface, tuned; **New York** (`ui-serif`) for topic names and reading text, because the material
is lecture text and a book face separates *what you study* from *the app around it*; SF Pro at weight 200 with
tabular figures for the clock.

| Role | Family | Size | Weight | Line-height | Tracking |
|---|---|---|---|---|---|
| Clock | SF Pro (`--sans`) | 88px | 200 | 1 | −0.03em |
| Display (Today headline, session topic) | New York (`--serif`) | 30px | 500 | 1.12 | −0.01em |
| Title (view title, course title) | SF Pro | 20px | 600 | 1.2 | −0.015em |
| Heading (section) | SF Pro | 13px | 600 | 1.3 | 0 |
| Topic (lists) | New York | 14px | 400 | 1.35 | 0 |
| Body | SF Pro | 13px | 400 | 1.45 | 0 |
| Small / meta | SF Pro | 12px | 400 | 1.35 | 0 |
| Caption | SF Pro | 11px | 500 | 1.3 | 0 |

No uppercase tracked labels. Hierarchy comes from ink-2 before size or weight. All figures tabular.

### Space, shape, depth
- **Spacing scale:** 4, 8, 12, 16, 24, 32, 48, 64. Inside a group 8, between groups 24, between page bands 32–48.
- **Containers:** pages max 1120px wide with 32px gutters; reading columns ≤ 68ch.
- **Controls:** 28px tall (Mac), primary actions on Today and in a session 36px.
- **Radius set:** 6 controls · 8 cards and fields · 12 dialogs · 999 for the clock's pause button only.
- **Elevation:** hairlines everywhere; a shadow only on what floats (popover, menu, dialog, toast):
  `0 1px 2px rgba(20,18,14,.06), 0 12px 32px -8px rgba(20,18,14,.22)`. Never border + shadow + tint together.

### Motion
- Hover/press 120ms ease-out; popover 160ms; dialog 200ms; toast 200ms; exits at 70%.
- Easing `cubic-bezier(0.22, 1, 0.36, 1)` in, `ease-in` 120ms out. No bounce.
- One orchestrated moment: when a new block starts, the toolbar strip slides one block left (300ms).
- Reduce Motion (System Settings): all transitions and the slide are off.
- Never animate on keyboard-started actions beyond the state change itself.

## Watchlist items used
| Pattern | Reason |
|---|---|
| System fonts | Offline Mac app; SF Pro and New York are tuned (weights, tracking, roles) and paired. |
| Serif for topic names | Separates study material from interface; it is the lecture's voice. |

## Do / Don't
- Do: let course colour appear only as identity (lane, thread, dot, chip, course bar). Keep chrome ink and paper.
- Do: say what is due and why in plain words; numbers are real data or absent.
- Do: keep every shortcut visible next to its action (`S`, `space`, `N`, `1`–`4`, `Enter`, `X`).
- Don't: gradients, glass, glow, coloured side stripes on banners, uppercase eyebrows, emoji, count-up numbers.
- Don't: put two filled primary buttons in one view region.

## Learned rules (from user corrections)
- 2026-11-17: No system blue and no glowing buttons. The app's accent colour is a warm grey (`app/Resources/Assets.xcassets`),
  default buttons in sheets use the ink primary style, and text actions are quiet ink-2 (`QuietLink`), never blue links.
- 2026-11-17: Nothing that cannot be clicked may look clickable, and every button must look like one (bordered, or an
  icon with its label). Check this visually, in the running app, not only in code.
- 2026-11-17: Keep panes simple. The topic details pane shows the essentials; longer material (source text, schedule
  and overrides, history) sits in sections that open on request.
