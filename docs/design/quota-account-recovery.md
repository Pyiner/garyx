# Quota account recovery

## Outcome

When a provider run stops because its usage quota is exhausted, Garyx records a
durable recovery job in SQLite. The thread-tail quota card becomes a larger,
actionable surface. For Claude Code it can open the existing account chooser,
and selecting an account immediately makes every waiting Claude Code recovery
job eligible for dispatch.

Quota reset, account switch, and manual retry all compete for the same SQL
claim. A recovery generation can therefore produce at most one synthetic
`continue` input, even when an account switch races the original reset time.

This feature does not bind an account to a thread. Claude account selection
remains provider-owned global runtime state. Existing runs keep their launch
snapshot; a recovered run resolves the provider's currently selected account
when it starts.

## Current state and problem

The committed `run_complete(status = rate_limited)` record contains the
provider, quota window, reset time, and `will_auto_resend`. The gateway's
`quota_resend` broadcast subscriber converts that record into a file-backed,
one-shot `InternalDispatch` cron job named `quota-resend:<thread-id>`.

That has three gaps:

1. account selection has no authoritative query for all quota-blocked threads;
2. dispatching immediately after a switch does not fence the old future cron;
3. generic internal dispatch has no quota-generation idempotency across a
   crash or competing trigger.

The transcript remains the historical truth for why the run ended. SQLite
becomes the operational truth for whether and when that terminated run may be
recovered.

## Product contract

### Thread card

The card stays at the transcript tail and continues to be driven by server
`render_state.rateLimit`.

Desktop layout:

- one white, monochrome outer card with a provider identity header;
- a readable reset/countdown line;
- for Claude Code, a full-width current-account row and `Switch account`
  affordance separated by one divider;
- a quiet sentence explaining that switching resumes all quota-paused Claude
  threads;
- no nested card and no quota meters inside the transcript card.

The existing app-centered Claude account dialog is extracted into a shared
component and reused by Provider Settings and the thread card. Candidate rows
retain Session, Weekly, and Fable linear meters.

iOS uses the same information hierarchy in native SwiftUI: a larger integrated
card and a 44-point account action row opening the existing large account
sheet. It does not port the desktop dialog or inline management actions.

For providers without account switching the enlarged card keeps its reset and
manual/automatic retry status but omits the account row.

### Account switch

Both Provider Settings and the thread card call the existing endpoint:

```http
PUT /api/providers/claude_code/accounts/active
```

The gateway performs these operations in order:

1. validate the requested profile;
2. apply and persist provider selection, including the bridge launch env;
3. when and only when the selection changed, hand the remaining effects to a
   detached gateway-owned task whose lifetime is independent of the HTTP
   request;
4. reconcile local Claude session replicas, then immediately expedite every
   already-durable waiting Claude Code recovery job to `due_at = now` and
   notify the recovery worker;
5. return the account selection plus that fast SQL recovery summary;
6. in the same detached task, reconcile committed rate-limit terminals that
   have not reached the async SQL projection yet, then perform a second
   idempotent expedite and notification pass.

The response is backward-compatible:

```json
{
  "active_account_id": "managed-account-id",
  "selection_changed": true,
  "recovery": {
    "matched_threads": 4,
    "expedited_threads": 4,
    "already_claimed_threads": 0
  }
}
```

Old clients ignore the added fields. New clients use the counts for a short
`Resuming 4 threads…` confirmation. The HTTP response does not wait for every
provider run or the whole-library transcript repair to finish. Once the
selection commit succeeds, a client timeout or disconnect cannot cancel the
backend-owned recovery wake. The first summary can report zero in the narrow
terminal-commit/SQL-projection window; the second pass still projects and
wakes that generation.

Selecting the already-active account is a no-op and does not force a premature
retry. A failed post-selection expedite is returned as an explicit recovery
warning without rolling back a provider selection that has already been
applied.

## SQLite model

The v1 schema keeps bounded operational history instead of overwriting the
only row for a thread:

```sql
CREATE TABLE quota_recovery_jobs (
    job_id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    blocked_run_id TEXT NOT NULL,
    blocked_seq INTEGER NOT NULL CHECK (blocked_seq > 0),
    quota_window TEXT,
    reset_at TEXT,
    due_at TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN (
        'waiting', 'claimed', 'delivered', 'superseded', 'cancelled'
    )),
    wake_reason TEXT NOT NULL CHECK (wake_reason IN (
        'quota_reset', 'account_switch', 'manual'
    )),
    claim_token TEXT,
    claim_expires_at TEXT,
    dispatch_intent_id TEXT NOT NULL,
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    last_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    settled_at TEXT,
    UNIQUE(thread_id, blocked_run_id)
) STRICT;

CREATE UNIQUE INDEX idx_quota_recovery_active_thread
    ON quota_recovery_jobs(thread_id)
    WHERE state IN ('waiting', 'claimed');

CREATE INDEX idx_quota_recovery_due
    ON quota_recovery_jobs(state, due_at);

CREATE INDEX idx_quota_recovery_provider_waiting
    ON quota_recovery_jobs(provider, state, due_at);
```

`blocked_run_id` is the recovery generation and `blocked_seq` is its monotonic
transcript position. Replaying the same committed terminal event is
idempotent. A higher sequence supersedes an older active job; a delayed lower
sequence is ignored even if an account-switch reconciliation already projected
the newer terminal.

A terminal without a trustworthy automatic reset is still recorded, with no
`reset_at` and a non-runnable sentinel deadline. Timer lookup excludes these
parked rows. Account switch or manual Continue changes their wake reason and
deadline to `now`, so “all quota-paused threads” includes providers that could
not report a reset time without inventing an automatic retry.

No account id, config directory, or credential data is stored in this table or
thread metadata.

Delivered/superseded/cancelled rows are retained for diagnostics and removed
by bounded retention after 30 days.

## State machine and dispatch

```text
rate-limited event
      |
      v
   waiting -- atomic claim --> claimed -- durable admission --> delivered
      ^                            |
      |                            +-- transient failure --> waiting + backoff
      |
new generation / user turn ------> superseded
thread deletion ------------------> cancelled
```

The recovery worker is SQL-backed and replaces quota-specific cron jobs. It
reads the earliest due row, sleeps until that deadline, and also listens on a
`Notify` so rate-limit events and account switches wake it immediately. It
claims with a short lease in a `BEGIN IMMEDIATE` transaction. At process
startup, all claims owned by the previous gateway process return to `waiting`;
there cannot be a live worker from that process to retain them.

Every dispatch uses the existing durable admission ledger with an internal
scope and deterministic intent:

```text
scope_identity   = __quota_recovery__
scope_epoch      = 1
client_intent_id = quota-recovery:<blocked_run_id>
```

Only an idle thread whose active recovery generation still matches may be
dispatched. The claim token is passed out-of-band in gateway process memory and
validated in the same SQLite transaction as durable dispatch admission. It is
never included in request metadata or its idempotency fingerprint, so a new
claim after restart reuses the exact same durable intent. A thread that already
started another run supersedes the recovery instead of queueing an extra
`continue` into the active run.

Manual Continue is moved from a generic client send to a quota-recovery retry
endpoint. It claims the same SQL row and therefore shares the same duplicate
fence as reset and account-switch recovery. Clients distinguish an accepted
wake from a settled-generation 404 and an unknown-route 404 from an older
gateway; the card renders all terminal and transport failures inline.

The internal message remains the literal `continue`, with internal metadata:

- `internal_dispatch = true`;
- `quota_recovery = true`.

Job id, generation, wake reason, and claim token remain exclusively in the SQL
recovery record and gateway logs; they are not sent through provider metadata.

Provider failures before durable admission release the lease with bounded
backoff. A new `rate_limited` completion creates a new generation and its own
provider reset deadline.

## Render-state ownership

The immutable transcript control retains the blocked run generation together
with provider/reset/window/message. The gateway overlays SQL state only when
the row's `blocked_run_id` matches that render generation; clients do not infer
scheduling from a reset time. This avoids an older settled row briefly masking
a newly committed rate-limit event before its asynchronous SQL projection.

The render contract gains optional recovery fields while keeping
`willAutoResend` for compatibility:

```json
{
  "recoveryGeneration": "run-id-that-hit-the-limit",
  "provider": "claude",
  "resetAt": "2026-07-23T01:30:00Z",
  "window": "primary",
  "willAutoResend": true,
  "recoveryState": "waiting",
  "recoveryAt": "2026-07-23T01:31:00Z"
}
```

`willAutoResend` is true only while SQL has a waiting or claimed matching job.
A fresh run still clears the entire rate-limit render state through the
existing reducer contract.

## Existing cron migration

At startup, after cron files are loaded but before the automation scheduler is
started:

1. enumerate system jobs whose id begins `quota-resend:`;
2. verify that the thread's latest transcript state is still rate-limited and
   matches the payload's `originating_run_id`;
3. insert the corresponding SQL generation using the cron's next run as
   `due_at`;
4. delete the cron only after the SQL insert commits.

Stale cron jobs whose thread has already started another run are deleted
without import. New code never creates quota cron jobs. Startup ordering makes
the SQL import authoritative before the scheduler starts; the generic cron
executor therefore treats any surviving system `quota-resend:` job as an
already-migrated no-op. This closes the SQL-commit/cron-delete crash window
without a second dispatch path.

Startup also scans canonical thread records and projects any still-current
committed rate-limit terminal into SQLite. This closes the crash window between
the transcript commit and the normal asynchronous event projection.

## Failure and race semantics

- Account selection is applied before jobs become due, so every newly started
  recovery resolves the new provider account.
- A recovery already claimed and started at the exact switch boundary is an
  existing run and remains untouched, matching the provider launch-snapshot
  contract.
- Timer and account switch racing for the same generation result in one SQL
  claim and one admission intent.
- A stale deadline for an older `blocked_run_id` cannot claim a newer
  generation.
- A user send admitted before recovery supersedes the waiting generation in
  the same SQLite admission transaction.
- Thread archive/delete removes or cancels pending recovery work.
- A gateway restart recovers claims left by the previous process. Reusing the deterministic
  dispatch intent prevents a second committed synthetic user input.

## Delivery plan

1. Add schema/migration, typed DB methods, state-machine tests, and worker.
2. Replace quota cron scheduling with SQL job registration and migrate legacy
   jobs at startup.
3. Extend account selection response and call the shared expedite operation.
4. Extend render contracts and the manual retry route.
5. Extract/reuse the desktop account selector and enlarge the desktop card.
6. Reuse the mobile account model/sheet and enlarge the iOS card.
7. Validate deterministic timer/switch/manual races, restart recovery,
   settings parity, desktop packaged UI, and iOS light-mode interaction.

## Acceptance tests

- Account switch one second before the original deadline commits one and only
  one `continue`.
- The old deadline produces no input after a successful account-switch
  recovery.
- Two rapid switches do not duplicate a recovery generation.
- A second rate-limited run on the same thread creates and dispatches a new
  generation.
- A user message before the deadline supersedes the old job.
- Restart after claim but before settlement does not duplicate the admitted
  input.
- Provider Settings and thread-card selection return the same recovery counts
  and behavior.
- Existing active runs keep their original account snapshot.
- Desktop card/dialog and iOS card/sheet expose the same account and quota
  meaning using native platform layout.
