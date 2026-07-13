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
   including eight historical rows with completed command tool groups, then
   commit 12 active frames which change only the last row. Every frame clones
   the complete wire snapshot so unchanged tool projections have equal values
   but fresh object identities. Count every transcript row render by its stable
   `row.key`.

The renderer runs under React dev StrictMode, which intentionally invokes each
render twice. Counts therefore have a stable 2x multiplier; comparisons use
the raw values and do not hide that multiplier.

## Pre-change result

Two consecutive runs returned exactly the same values:

- Background phase: 12 frames, 24 `AppShell` renders, 24 `ThreadPage`
  renders.
- Active phase: 12 frames over 40 rows (eight tool-bearing), 24 `AppShell`
  renders, 24 `ThreadPage` renders, and 960 transcript-row renders.
- Of the 960 active row renders, 936 were renders of the 39 unchanged history
  rows. The changing tail row rendered 24 times.
- All 40 stable row keys were observed on every active render.

This reproduces both target regressions deterministically: every background
thread transcript commit reaches the whole shell, and every active stream
frame rebuilds every historical row element. It also prevents the optimized
gate from missing a comparator that accidentally treats an equal decoded tool
projection as changed merely because its object identity is new.

## Post-change gate

The same driver runs with `--expect optimized`. That mode requires:

- zero `AppShell` renders for background transcript frames; and
- zero renders of unchanged transcript row keys during active streaming.

## Post-change result

Two consecutive optimized runs returned exactly the same values:

- Background phase: 12 frames, zero `AppShell` renders, zero `ThreadPage`
  renders, and zero transcript-row renders.
- Active phase: 12 frames over the same 40 rows (including the eight
  tool-bearing rows with fresh equal-value projections), 24 `AppShell`
  renders, 24 `ThreadPage` renders, and 24 transcript-row renders.
- All 24 row renders were the changing tail row. The 39 unchanged history
  rows rendered zero times, and only the tail row key was observed after the
  counter reset.

Against the pre-change result, background shell renders fell from 24 to zero,
total active row-body renders fell from 960 to 24 (97.5%), and unchanged
history-row renders fell from 936 to zero. The active thread still renders its
shell and page once per StrictMode pass, as expected for its per-thread
subscription; the memo boundary confines expensive transcript subtree work to
the row whose presentation actually changed.
