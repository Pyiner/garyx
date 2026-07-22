# Claude SessionStore eager default

## Goal

Garyx uses a local SessionStore rooted at ~/.claude/projects to keep one
native Claude session resumable across System default, managed accounts, and
the terminal CLI. For this local adapter, mirror delivery should be
near-real-time while preserving the pinned official TypeScript SDK contract.

Garyx will explicitly choose eager for production Claude runs. The generic
Rust SDK keeps Anthropic's public default, batched, so callers that do not
make a Garyx-specific choice still match
@anthropic-ai/claude-agent-sdk@0.3.217.

## Official oracle

The parity oracle remains:

- npm package: @anthropic-ai/claude-agent-sdk@0.3.217;
- TypeScript repository tag: v0.3.217;
- repository commit: 2997b3d35a729ef823d4edf6cf3c690f86d888e3.

The official type contract describes:

- batched (default): buffer transcript_mirror frames and flush at end-of-turn
  or after pending thresholds are exceeded;
- eager: schedule a background flush after every frame, deliver near-real-time
  to SessionStore.append, and do not coalesce frames;
- both modes await outstanding work before forwarding result and during
  cleanup.

The official implementation makes enqueue() synchronous. A threshold crossing
schedules drain() without awaiting it in the transport reader. Each drain
captures the pending frames present at scheduling time, waits for the previous
drain, then sends its own batch. This preserves append order without blocking
unrelated Claude output.

## Current gaps

1. SessionStoreFlush::Eager exists in the Rust SDK, but
   ClaudeProvider::build_sdk_options_with_launch_env inherits
   ClaudeAgentOptions::default(), which is correctly Batched. Production
   Garyx therefore never selects eager.
2. Rust TranscriptMirrorBatcher::enqueue() awaits flush() whenever a threshold
   is exceeded. In eager mode every frame exceeds the zero thresholds, so the
   Claude stdout reader blocks on every store append. That is not the official
   background scheduling behavior.
3. Existing TypeScript differential tests pin final append batch shapes
   ([1, 1, 1] for eager), thresholds, retry, and error counts, but do not
   distinguish a background append from a reader-blocking append.

## Design

### 1. Preserve the SDK default; select eager at the Garyx composition root

ClaudeAgentOptions::default().session_store_flush stays
SessionStoreFlush::Batched.

The Garyx Claude provider explicitly sets session_store_flush to
SessionStoreFlush::Eager next to session_store: Some(...). This makes eager the
Garyx product default without silently changing the reusable SDK's
Anthropic-compatible default. There is no user-facing setting in this change.

### 2. Serialize background drains through one worker

Replace the reader-owned pending mutex/flush lock with one per-query mirror
worker. enqueue() sends a frame to an unbounded command queue and returns
immediately. The worker owns:

- pending frames and entry/UTF-16-byte counters;
- the configured entry and byte thresholds;
- the SessionStore and retry policy;
- an unbounded mirror-failure channel back to the SDK reader.

Commands are processed in order:

- Enqueue(frame): add it to pending state; if either strict greater-than
  threshold is crossed, drain now. Eager uses zero thresholds, so every command
  drains one frame and frames are never coalesced.
- Flush(barrier): drain all pending frames, then acknowledge the barrier.

Because the worker is single-consumer, drains remain ordered exactly like the
official promise chain. Batched grouping by filePath, strict 501 / 1 MiB
thresholds, retry [200ms, 800ms], and 60-second timeout remain unchanged.

### 3. Deliver background failures without blocking Claude output

The reader multiplexes transport messages and the worker's mirror-failure
channel. A terminal append failure still becomes one synthetic mirror_error
system message while the query continues.

Before forwarding result, and on EOF/read error, the reader sends a flush
barrier and then drains all failures already emitted by that barrier. Explicit
disconnect also waits for a barrier before tearing down the reader. This pins
the official property that eager work is background during the turn but
complete before the turn result is observable.

Dropping the last batcher sender lets the worker finish already queued commands
and best-effort drain any remaining batched state before exiting.

### 4. Keep local durability and recovery layers unchanged

Eager changes only the latency of canonical appends. Claude still writes its
selected profile first. The existing run-end reconcile, startup/account-switch
sweep, per-resume reconcile/materialize, parsed-deep-equal check, and mtime LWW
rules remain the durable fallback.

## Differential and regression tests

Extend the pinned official TypeScript differential with an eager-background
scenario:

1. the fake CLI emits one mirror frame and then an ordinary assistant message;
2. the test store holds the corresponding append() behind a gate;
3. receiving the assistant message opens the gate;
4. a watchdog opens it only on failure, recording timeout.

The official oracle must observe the assistant before the append completes.
The Rust probe must produce the identical event order and must not use the
watchdog. Reverting background enqueue to the old awaited implementation makes
this scenario fail deterministically.

Add a bridge contract test asserting production Claude options choose
SessionStoreFlush::Eager; it fails on the parent commit, which chooses Batched.

Keep and rerun:

- all existing mirror trace differentials;
- the official SessionStore parity gate, with the new scenario added;
- cargo test -p claude-agent-sdk --all-targets;
- cargo test -p garyx-bridge --all-targets;
- formatting and AGENTS/CLAUDE byte parity.

## Acceptance

- Garyx managed Claude runs use eager by default.
- Generic SDK callers still default to official batched behavior.
- An eager append cannot delay the next ordinary Claude message.
- Outstanding eager appends complete before result is delivered.
- Every eager frame remains one ordered append() batch.
- Mirror failures remain best-effort, visible, and non-fatal.
- Existing session reconciliation and account isolation behavior is unchanged.
