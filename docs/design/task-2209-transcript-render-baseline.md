# TASK-2209 Desktop Transcript Render Baseline

## Purpose

This document records the deterministic pre-change render oracle for the
desktop transcript subscription and row-render path. The measured source
baseline is commit `e734a243c` plus the dev-only probe and oracle driver; no
subscription, memoization, or mirror-retention behavior was changed before the
baseline was captured.

## Oracle

Run from `desktop/garyx-desktop`:

```sh
npm run transcript-render:oracle -- --expect baseline --frames 12 --rows 40
```

The driver launches an isolated Electron dev instance with separate user data
and a separate CDP port. It installs an opt-in render counter, then drives the
real in-renderer `GatewayMirror` and the real `AppShell` / `ThreadPage` render
path with synthetic transcript data. Mirror mutations are in-memory only; the
driver does not create or update gateway threads.

The two phases are:

1. Commit 12 frames to an unselected background thread and count `AppShell`
   and `ThreadPage` function renders.
2. Seed 40 server-owned `render_state` user-turn rows on the active draft,
   then commit 12 active frames which change only the last row. Count every
   transcript row render by its stable `row.key`.

The renderer runs under React dev StrictMode, which intentionally invokes each
render twice. Counts therefore have a stable 2x multiplier; comparisons use
the raw values and do not hide that multiplier.

## Pre-change result

Two consecutive runs returned exactly the same values:

- Background phase: 12 frames, 24 `AppShell` renders, 24 `ThreadPage`
  renders.
- Active phase: 12 frames over 40 rows, 24 `AppShell` renders, 24
  `ThreadPage` renders, and 960 transcript-row renders.
- Of the 960 active row renders, 936 were renders of the 39 unchanged history
  rows. The changing tail row rendered 24 times.
- All 40 stable row keys were observed on every active render.

This reproduces both target regressions deterministically: every background
thread transcript commit reaches the whole shell, and every active stream
frame rebuilds every historical row element.

## Post-change gate

The same driver will run with `--expect optimized`. That mode requires:

- zero `AppShell` renders for background transcript frames; and
- zero renders of unchanged transcript row keys during active streaming.

Post-change raw results will be appended here after implementation and before
code review.
