# Claude Code local SessionStore parity

Status: implemented
Owner: Garyx  
Baseline: `@anthropic-ai/claude-agent-sdk@0.3.217`, tag/commit
`v0.3.217` / `2997b3d35a729ef823d4edf6cf3c690f86d888e3`

## Problem

Garyx gives every managed Claude Code account its own `CLAUDE_CONFIG_DIR`.
Claude Code stores native sessions below that directory, so changing accounts
changes the session lookup root. A subsequent `--resume <session-id>` cannot
find the old transcript and Garyx currently falls back to a new native
session. The Garyx transcript remains intact, but Claude Code loses its native
conversation, compaction, tool, and subagent state.

The official TypeScript Agent SDK solves this with `SessionStore`: load the
same native transcript before spawning a resumed process and mirror every
successful local transcript write back to a store. Garyx's Rust SDK must
implement the same protocol rather than reconstructing context in a prompt.

## Product contract

1. Switching Claude Code accounts never changes the native Claude session ID
   attached to a Garyx thread.
2. A resumed run uses Claude Code's native `--resume`; Garyx does not replay or
   summarize the old conversation into a user message.
3. The canonical store defaults to `~/.claude/projects`. Sessions created by
   Garyx are consequently visible to the ordinary terminal Claude Code, and
   terminal updates are available to the next Garyx resume.
4. Managed account directories continue to own credentials and settings.
   Credentials are never copied between accounts and the canonical store
   contains transcripts only.
5. The account/config directory is runtime provider state. It is never written
   to thread metadata, agent metadata, an admission fingerprint, or the quota
   recovery row.
6. An already-running Claude process keeps its launch snapshot. A later run,
   including quota recovery after an account switch, materializes the session
   into the newly selected account and resumes it.

## Scope

Implement the TypeScript SessionStore behavior needed by Garyx and one
production adapter:

- `SessionKey`, opaque JSON `SessionStoreEntry`, `SessionStore`, and
  `SessionStoreFlush` equivalents in `claude-agent-sdk`;
- `LocalDirectorySessionStore`, backed by a Claude-compatible `projects`
  directory;
- main transcript and nested subagent transcript load/append/list/delete;
- pre-spawn resume materialization;
- CLI `--session-mirror` consumption, batching, bounded retry, timeout, and
  `mirror_error` reporting;
- lazy import of pre-feature sessions from managed account project roots;
- parsed-content/mtime LWW reconciliation across canonical and managed copies;
- startup, account-switch, pre-resume, and post-run reconciliation;
- Garyx bridge wiring with `~/.claude/projects` as the default canonical root.

S3, Redis, Postgres, remote storage, a settings control for the directory, and
the TypeScript SDK's unrelated session browsing/mutation helpers are out of
scope. The Rust API accepts an explicit local directory so another embedding
can select a different root without adding another backend.

Store-backed `--continue` latest-session discovery is also out of scope:
Garyx always persists and resumes an explicit native session ID. The ordinary
SDK `continue_conversation` option retains its existing CLI-local behavior.

## Official parity boundary

The source oracle is pinned, never `latest`. Upgrading it requires an explicit
baseline change and a fresh differential run.

| Behavior | TypeScript `0.3.217` | Rust requirement |
| --- | --- | --- |
| Project key | Canonical cwd; non-ASCII-alphanumeric mapped to `-`; at 200 chars append the portable signed-32-bit hash in base 36 | Byte-for-byte key equality, including UTF-16 hash input |
| Unknown key | `load()` returns `null` | `load()` returns `None` |
| Append | Empty append is a no-op; calls preserve entry and call order | Same |
| Isolation | Project, session, and optional subpath are independent | Same |
| Listing | Main transcripts only; integer epoch-ms mtime; result order unspecified | Same |
| Delete | Main delete cascades to subkeys; subpath delete is isolated | Same |
| Subkeys | Main transcript excluded; nested relative subpaths retained | Same |
| Resume load | Main transcript loaded before subprocess spawn; empty/missing does not materialize | Same |
| Subagent restore | `listSubkeys`, unsafe-path rejection, JSONL restore, last `agent_metadata` entry to `.meta.json` | Same |
| Mirror mapping | Main paths require JSONL; nested paths preserve the pinned SDK's permissive suffix handling; all paths must remain below the launched config's `projects` root | Same |
| Batched flush | 500 entries or 1 MiB thresholds, flush when strictly exceeded; group by file path | Same |
| Eager flush | Zero thresholds, one scheduled frame per append batch | Same observable append batches |
| Flush points | Before forwarding `result`, at EOF, and during cleanup | Same |
| Retry | Three attempts total, 200 ms then 800 ms; 60 s attempt timeout is not retried | Same |
| Failure | Drop failed batch, keep CLI running, emit `system/mirror_error` with key | Same |
| Local write | CLI local write remains primary; mirror is secondary | Same |

The TypeScript implementation creates a temporary config directory because a
remote store has no native Claude layout. Garyx's only adapter is already a
native projects directory and must preserve the selected account directory so
Claude's path-scoped credential lookup continues to work. Therefore Rust
materializes into the selected launch `CLAUDE_CONFIG_DIR/projects` instead of
moving credentials into a temporary directory. This is the sole lifecycle
difference; store calls, canonical JSON entries, CLI arguments, spawn ordering,
and resume identity remain the parity surface.

## Local directory layout

`LocalDirectorySessionStore::new(root)` treats `root` as a Claude `projects`
directory:

```text
<root>/
  <project-key>/
    <session-id>.jsonl
    <session-id>/
      subagents/
        agent-<id>.jsonl
        agent-<id>.meta.json
```

Main and subagent JSONL files use one compact JSON object plus `\n` per entry.
For subagents, `agent_metadata` is represented as the native `.meta.json`
sidecar on disk and reconstructed as the final opaque store entry on load.
Files are created with user-only permissions on Unix.

Keys are validated before joining: project and session are single safe path
segments; subpaths must be non-empty, relative, and contain no `..` segment.
Canonicalization/containment checks prevent a symlink or traversal from
escaping the configured root.

## Read, reconcile, mirror, and account-switch flow

```text
Garyx thread sdk_session_id
        |
        v
reconcile canonical + every safe managed copy
        | parsed equal => no write
        | different => newest mtime wins (tie => canonical)
        v
LocalDirectorySessionStore (~/.claude/projects, canonical winner)
        | load before every resume
        v
selected account CLAUDE_CONFIG_DIR/projects/<project>/<session>.jsonl
        | claude --resume=<same id> --session-mirror
        v
transcript_mirror frames ----append----> canonical store
        |
        `---- post-run reconcile fallback ----^
```

When the selected config's projects root is itself the canonical root (System
default), the CLI already writes the canonical files. Rust skips physical
materialization and redundant mirror appends so entries are not duplicated,
while retaining native resume. It still reconciles managed recovery copies
before the default resume.

Every managed-account resume physically materializes the full winning main and
subagent view into that selected profile, including a same-account resume. The
operation happens before the Claude subprocess is spawned. An already-running
process keeps its original directory and is never rewritten in place by an
account switch; only a future top-level run observes the new provider choice.

### Local last-write-wins reconciliation

The canonical file and every safe managed-account file are recovery replicas,
not independent branches. Reconciliation is per key: the main transcript and
each subagent transcript (with its metadata sidecar) are compared separately.

1. Discover existing candidates and their filesystem modification times.
2. Pick the greatest mtime. An exact tie favors the canonical root; equal
   profile mtimes use stable path ordering.
3. Parse the winning candidate. If it is canonical, leave it in place.
4. If a managed candidate wins, compare parsed JSON entries with canonical.
   Equal content is a no-op regardless of mtime. Different content atomically
   replaces canonical with the winner; the managed source is never rewritten
   by reconciliation.

The parsed-content check is deliberately first in effect: resume
materialization refreshes the selected profile's mtime even when bytes are
unchanged, and must not create a feedback loop that continuously rewrites
canonical. The system never concatenates or guesses a semantic merge. When
copies truly diverge, the later durable native write is the complete winner,
as selected by mtime. Claude compaction is append-only in the pinned/native
protocol: prior JSONL lines remain and a compact summary is appended, so normal
compaction does not create a competing rewritten prefix.

Reconciliation runs at four lifecycle points:

- gateway startup: sweep all valid UUID sessions so legacy managed-only files
  become visible to terminal Claude;
- account switch: sweep before quota-recovery threads are expedited;
- every explicit resume: reconcile that session before materialization/spawn;
- every run end: reconcile that session after CLI shutdown, covering a missed
  mirror frame and the optional cctty execution mode, which does not currently
  emit `transcript_mirror` frames.

The post-run pass is best-effort because the account-local file remains primary
evidence; startup/switch/next-resume retries it. A native managed run normally
updates canonical through mirror frames first, so the post-run pass is a
parsed-equal no-op. There remains one unavoidable local-only window: if Garyx
is killed after Claude writes a profile but before mirror/post-run reconcile,
and the user immediately resumes from terminal before Garyx restarts or an
account switch occurs, terminal Claude can still see the older canonical copy.
The next Garyx startup closes that window.

### Legacy bootstrap

On a canonical miss for a resumed session, the local adapter probes configured
legacy projects roots. Garyx supplies:

- the selected account's `projects` directory;
- safe sibling managed account directories beneath the same
  `provider-accounts/claude-code` root;
- the default Garyx managed-account root when it differs.

Candidates are exact `(projectKey, sessionId)` matches. Legacy-only files use
the same per-key mtime reconciliation as post-feature files and are promoted
into canonical without removing or rewriting their source. Because startup
sweeps all safe profiles, a pre-feature managed-only session becomes available
to ordinary terminal Claude without first resuming it inside Garyx.

This repairs sessions created before SessionStore support without persisting
the source account on the thread.

## Failure policy

- Store load/list timeout or invalid stored JSON fails the resumed launch
  before the subprocess starts. Garyx must not silently clear the native
  session and send the turn to a fresh conversation for a storage failure.
- A genuinely absent session after canonical and legacy lookup retains the
  existing explicit "session not found" handling, but the bridge records the
  reason. The SessionStore rollout removes account-switch misses from that
  category.
- Mirror append failure does not kill a successful Claude turn, matching the
  official SDK. It emits `mirror_error`, remains visible in logs, and leaves
  the account-local transcript as recovery evidence for post-run/startup LWW
  reconciliation.
- A newer corrupt candidate fails that session's pre-resume reconciliation and
  never destroys the valid canonical copy. Startup/account-switch sweeps isolate
  a corrupt managed winner to that session and continue scanning. A canonical
  winner is only statted during a sweep and is parsed on actual resume; a stale
  corrupt profile is never parsed when canonical wins.
- Malformed/unsafe subkeys are skipped with a warning; they are never joined
  outside the target session directory.
- File checkpointing remains incompatible with SessionStore, matching the
  official SDK, because checkpoint blobs are not mirrored.

## Differential test gate

`scripts/test/claude_session_store_parity.sh` is a release gate for this
feature. It installs the pinned official npm package into an isolated temp
directory, verifies the package version, and drives both implementations from
the same JSON operation scripts.

The pinned TypeScript/Rust differential covers:

1. all 13 official adapter conformance behaviors;
2. exact project-key output, including a path longer than 200 characters and
   non-ASCII UTF-16 input;
3. main/subagent materialization and `agent_metadata` sidecars;
4. batch grouping, strict 500-entry/1-MiB thresholds, eager mode, flush before
   result, and retry backoff count;
5. unsafe mirror paths;
6. fake-CLI end-to-end: the canonical store contains a native session, the
   selected profile starts empty, and both SDKs resume the same session ID
   after store materialization.

The script compares canonical JSON traces, not log strings or Rust-specific
error wording. Network-independent Rust tests separately pin timeout
no-retry, unsafe subkeys, and legacy sibling-profile import followed by
materialization into a new profile. Parent failure (no SessionStore/mirror
surface) and feature-head success must be demonstrated before review.

## Acceptance criteria

- The reported cross-account reproduction resumes its original Claude Code
  session instead of creating a replacement session.
- Default terminal Claude Code can list/resume a session written by a managed
  Garyx account.
- Switching back and forth across at least two managed accounts preserves one
  native session ID and one monotonically growing canonical transcript.
- Default, two managed accounts, and direct terminal Claude can alternate
  resumes without truncating a newer copy; same-account resumes also reconcile
  before materialization.
- Parsed-equal/newer, divergent/newer, exact-mtime tie, legacy-only, mirror
  failure, corrupt candidate, main/subagent, and startup/account-switch sweeps
  have deterministic tests.
- Quota-recovery dispatch after an account switch uses the same path without a
  special-case prompt or metadata field.
- Official TypeScript/Rust differential traces are identical at the pinned
  baseline.
- Focused SDK/bridge tests, Rust tier-1 changed tests, and a real local
  two-profile smoke test pass before adversarial review.
