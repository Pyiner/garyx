# #TASK-2469 Gateway durable delivery admission design

Date: 2026-07-20

Status: design review candidate

Scope: P0-G server batch derived from the P0-A review

## 1. Decision summary

This batch adds one server-owned durable admission boundary in front of every
client-correlated chat dispatch, then builds atomic create-and-dispatch and
managed prompt-attachment ownership on that boundary.

The non-negotiable invariants are:

1. A dispatch identity is exactly
   `(scope_identity, scope_epoch, thread_id, kind, client_intent_id)`.
   Once the provider handoff gate for that identity has been crossed, a replay
   never crosses it again. A settled replay returns the originally allocated
   run and pending-input identifiers.
2. `thread_records`, all of its SQL projections, an optional endpoint-owner
   move, a create-intent claim, the first dispatch-admission row, and attachment
   claims are committed in one SQLite transaction for the new atomic command.
   There is no committed thread without a durable create claim and dispatch
   admission, and no committed claim pointing at a different thread.
3. The unique create mapping is never reused or silently forgotten, including
   after archive/delete. A client can always distinguish “unknown intent”,
   “preparing”, “committed”, and “the claimed thread was later removed” by a
   SQL point query.
4. Only files created by the prompt-attachment upload service are managed.
   Their state is `ready -> claimed -> delete_pending`; physical deletion is a
   retryable outbox operation. Arbitrary workspace paths remain caller-owned
   and are never deleted by this protocol.
5. A committed migration marker is durable protocol. A database bearing a v1
   marker but missing or drifting from the v1 table/index shape fails startup;
   it is not repaired by `CREATE TABLE IF NOT EXISTS`, a read route, a
   backfill, or a reconcile pass.

The provider runtimes do not expose a durable idempotency primitive. Therefore
this design promises **durable at-most-once provider handoff**, not
exactly-once execution. The gateway writes `handoff_started` before invoking a
provider. A process loss in the tiny interval after that commit is reported as
`ambiguous` with the same allocated identifiers and is never automatically
reissued. This deliberately prefers a possible zero dispatch over a duplicate.
Normal HTTP/WS disconnects do not create this interval: a detached gateway
supervisor owns the operation after admission and finishes independently of
the request task.

## 2. Current failure and scope

Today `prepare_chat_request` copies `clientIntentID` only into metadata,
`start_chat_run` allocates a new UUID on every call, and
`add_streaming_input_with_metadata` allocates a new `queued_input:*` UUID on
every call. Neither the SQLite truth store nor the bridge records a dispatch
identity before provider side effects. Repeating one logical request can
therefore start or queue it twice.

Thread creation has a separate gap: the client performs create, optional bot
binding, and chat start as multiple requests. Losing the create response leaves
a real thread that cannot be claimed by create intent because the list response
does not expose metadata and there is no unique mapping.

`upload_chat_attachments` writes random files below the global temporary
directory and returns their paths. It records neither an owner nor an expiry,
so an abandoned upload and a completed run both leave an orphan.

This batch does not:

- make a remote provider exactly-once;
- store gateway authentication secrets or provider runtime metadata in an
  idempotency row;
- infer create claims for historical threads from free-form metadata;
- delete ordinary workspace files supplied as prompt attachments;
- automatically delete untracked files from the historical process-global
  temporary root, which has no per-data-directory ownership proof;
- restart or install the running gateway;
- add a complete durable desktop outbox. Section 11 adds the requested Mac
  canonical-state consumer and explicitly separates product persistence.

## 3. Shared wire identities and validation

### 3.1 Explicit scope

The additive wire type is:

```json
{
  "idempotencyScope": {
    "identity": "stable-client-gateway-partition",
    "epoch": 1
  }
}
```

It maps exactly to `scope_identity TEXT` and `scope_epoch INTEGER`. It is a
correlation namespace, not authorization; existing gateway authentication is
still the security boundary. The server validates:

- `identity`: trimmed, 1..256 UTF-8 bytes;
- `epoch`: 1..`i64::MAX` for explicit scopes;
- `clientIntentId` and `createIntentId`: trimmed, 1..256 UTF-8 bytes;
- thread IDs through the existing canonical thread-key parser.

The reserved database epoch `0` is used only for the compatibility scope
`__legacy_api__`. New clients cannot submit epoch zero or that reserved
identity.

HTTP and WebSocket start/input use the same parser and application service.
For chat start the top-level `clientIntentId` becomes canonical. During the
compatibility window, `metadata.client_intent_id` is accepted only when the
top-level field is absent or byte-equal; disagreement is `400` rather than a
last-writer-wins choice.

### 3.2 Compatibility behavior

Requests with no `clientIntentId` retain current non-idempotent behavior.
Requests with an intent but no explicit scope use
`(__legacy_api__, 0)`. This immediately protects the existing desktop and iOS
payloads, both of which already send an intent, without pretending that the
legacy namespace survives an authentication-epoch change. Capability-gated
clients must send an explicit scope before enabling automatic retry.

The health response adds, without removing existing fields:

```json
{
  "deliveryCapabilities": {
    "dispatchAdmission": 1,
    "atomicCreateDispatch": 1,
    "createIntentClaim": 1,
    "promptAttachmentLifecycle": 1,
    "explicitScopeRequiredForRecovery": true
  }
}
```

The capability object is published only after every corresponding migration
has run and its schema has validated. A client that does not see it keeps its
current ambiguous/manual-recovery behavior.

### 3.3 Request fingerprint

Every durable key also stores a versioned SHA-256 request fingerprint. Version
1 serializes an explicit fingerprint struct after recursively sorting JSON
object keys. It includes the raw client-semantic values: raw message, thread,
channel/account/from identity, requested agent/provider/workspace, attachment
IDs (or lexically normalized unmanaged path strings), inline image/file
content hashes, and
client metadata after server-owned keys are stripped. It excludes the key's
intent/scope fields, timestamps allocated by the server, auth tokens, resolved
runtime configuration, and generated IDs.

Fingerprint normalization is pure: unmanaged attachment paths are normalized
lexically, not by opening/canonicalizing the file. This lets a replay prove its
identity after an already-consumed file has been deleted.

The fingerprint uses the raw message rather than the current slash-command
expansion, so a replay after a configuration change still identifies the
original operation. The original prepared payload stays owned by the detached
in-process supervisor; the ledger does not become a second transcript or a
secret-bearing request archive.

The same key with a different fingerprint returns `409 idempotency_conflict`
and does not mutate the original row, thread, binding, attachment, or provider.

## 4. Versioned database migrations

The tables in this section are created by marker-aware startup migrations, not
by the unconditional base-schema batch.

### 4.1 Dispatch-admission ledger

Marker: `dispatch_admission_ledger_v1`, version `1`.

```sql
CREATE TABLE dispatch_admissions (
    scope_identity TEXT NOT NULL,
    scope_epoch INTEGER NOT NULL CHECK (scope_epoch >= 0),
    thread_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('chat_start', 'stream_input')),
    client_intent_id TEXT NOT NULL,

    fingerprint_version INTEGER NOT NULL CHECK (fingerprint_version = 1),
    request_fingerprint TEXT NOT NULL,

    admission_state TEXT NOT NULL CHECK (admission_state IN (
        'admitted',
        'handoff_started',
        'accepted',
        'not_dispatched',
        'rejected',
        'ambiguous'
    )),
    handoff_attempt INTEGER NOT NULL DEFAULT 0 CHECK (handoff_attempt >= 0),
    outcome TEXT CHECK (outcome IS NULL OR outcome IN (
        'started', 'queued_to_active_run', 'no_active_session'
    )),

    requested_run_id TEXT,
    effective_run_id TEXT,
    pending_input_id TEXT,
    result_http_status INTEGER,
    result_error_code TEXT,
    result_error_message TEXT,

    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    handoff_started_at TEXT,
    settled_at TEXT,

    CHECK (
        (scope_epoch = 0 AND scope_identity = '__legacy_api__')
        OR
        (scope_epoch > 0 AND scope_identity <> '__legacy_api__')
    ),
    CHECK (kind <> 'chat_start' OR requested_run_id IS NOT NULL),
    CHECK (
        outcome <> 'queued_to_active_run'
        OR (effective_run_id IS NOT NULL AND pending_input_id IS NOT NULL)
    ),
    CHECK (outcome <> 'started' OR requested_run_id IS NOT NULL),
    CHECK (
        outcome <> 'no_active_session'
        OR (kind = 'stream_input' AND admission_state = 'not_dispatched')
    ),
    CHECK (
        admission_state NOT IN ('accepted', 'not_dispatched')
        OR outcome IS NOT NULL
    ),
    CHECK (
        admission_state <> 'not_dispatched'
        OR outcome = 'no_active_session'
    ),
    CHECK (
        admission_state <> 'accepted'
        OR outcome IN ('started', 'queued_to_active_run')
    ),
    CHECK (
        admission_state NOT IN ('handoff_started', 'accepted', 'ambiguous')
        OR handoff_attempt > 0
    ),
    PRIMARY KEY (
        scope_identity, scope_epoch, thread_id, kind, client_intent_id
    )
) STRICT;

CREATE INDEX idx_dispatch_admissions_thread_state
    ON dispatch_admissions(thread_id, admission_state);
```

`requested_run_id` is allocated before insertion for `chat_start`. A queued
plan allocates `effective_run_id` and `pending_input_id` before changing the
gate to `handoff_started`. Thus a settled replay returns exactly the first
result, and a crash-ambiguous replay still returns every identifier known
before the side-effect boundary. Result error text is bounded diagnostic text;
it never contains provider output or request content.

Rows are retained indefinitely in v1. Deleting a settled key would silently
weaken at-most-once, so retention can only be introduced with a new protocol
version/capability and an explicit client horizon.

### 4.2 Create-intent claim and managed preparation resources

Marker: `thread_create_intent_claim_v1`, version `1`.

```sql
CREATE TABLE thread_create_intents (
    id INTEGER PRIMARY KEY,
    scope_identity TEXT NOT NULL,
    scope_epoch INTEGER NOT NULL CHECK (scope_epoch >= 0),
    create_intent_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,

    fingerprint_version INTEGER NOT NULL CHECK (fingerprint_version = 1),
    request_fingerprint TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN (
        'reserved', 'preparing', 'committed', 'failed_before_commit'
    )),
    command_kind TEXT NOT NULL CHECK (command_kind IN (
        'create_only', 'create_and_dispatch'
    )),
    dispatch_client_intent_id TEXT,
    owner_boot_id TEXT,
    lease_expires_at TEXT,
    failure_code TEXT,
    failure_message TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    committed_at TEXT,

    CHECK (
        (scope_epoch = 0 AND scope_identity = '__legacy_api__')
        OR
        (scope_epoch > 0 AND scope_identity <> '__legacy_api__')
    ),
    CHECK (
        (command_kind = 'create_only' AND dispatch_client_intent_id IS NULL)
        OR
        (command_kind = 'create_and_dispatch'
            AND dispatch_client_intent_id IS NOT NULL)
    ),
    CHECK (state <> 'preparing' OR owner_boot_id IS NOT NULL),
    CHECK (state <> 'preparing' OR lease_expires_at IS NOT NULL),
    CHECK ((state = 'committed') = (committed_at IS NOT NULL)),
    UNIQUE (thread_id)
) STRICT;

CREATE UNIQUE INDEX idx_thread_create_intents_scope_intent
    ON thread_create_intents(
        scope_identity, scope_epoch, create_intent_id
    );

CREATE TABLE thread_create_resources (
    create_intent_row_id INTEGER NOT NULL,
    resource_kind TEXT NOT NULL CHECK (resource_kind IN (
        'managed_workspace', 'worktree', 'imported_transcript'
    )),
    resource_path TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN (
        'reserved', 'materializing', 'materialized',
        'adopted', 'delete_pending', 'deleted'
    )),
    owner_marker TEXT NOT NULL,
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    next_attempt_at TEXT,
    last_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (create_intent_row_id, resource_kind, resource_path),
    FOREIGN KEY (create_intent_row_id)
        REFERENCES thread_create_intents(id) ON DELETE RESTRICT
) STRICT;

CREATE INDEX idx_thread_create_resources_cleanup
    ON thread_create_resources(state, next_attempt_at)
    WHERE state = 'delete_pending';
```

The named unique index is the canonical
`(scope, createIntentID) -> threadID` contract. The extra unique constraint on
`thread_id` prevents one idempotently created thread from being claimed by two
intents. `command_kind` prevents a create-only replay from being confused with
an atomic create-and-dispatch request. Neither row is cascaded away when a
thread is archived/deleted.

Preparation resources exist because Git worktree creation, managed-directory
creation, and transcript-file import cannot participate in a SQLite
transaction. Before each external action, the service records a deterministic
path and owner marker. Only a path with that exact marker and under a
Garyx-managed root may be cleaned. User-selected local workspaces never enter
this table.

`owner_boot_id` plus `lease_expires_at` fences preparation ownership. Before
listener bind, rows owned by an older boot are not resumed blindly: their
marked managed resources converge to `delete_pending`, then the claim returns
to `reserved` with the same thread ID. The request body is not persisted, so
only an equal-fingerprint POST replay can start a new preparation lease.

### 4.3 Prompt-attachment lifecycle

Marker: `prompt_attachment_lifecycle_v1`, version `1`.

```sql
CREATE TABLE prompt_attachments (
    attachment_id TEXT PRIMARY KEY,
    scope_identity TEXT NOT NULL,
    scope_epoch INTEGER NOT NULL CHECK (scope_epoch >= 0),
    relative_path TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK (kind IN ('image', 'file')),
    original_name TEXT NOT NULL,
    media_type TEXT NOT NULL,
    byte_size INTEGER NOT NULL CHECK (byte_size >= 0),
    sha256 TEXT NOT NULL,

    state TEXT NOT NULL CHECK (state IN (
        'ready', 'claimed', 'delete_pending'
    )),
    expires_at TEXT NOT NULL,
    lease_expires_at TEXT,
    owner_thread_id TEXT,
    owner_kind TEXT CHECK (owner_kind IS NULL OR owner_kind IN (
        'chat_start', 'stream_input'
    )),
    owner_client_intent_id TEXT,
    owner_requested_run_id TEXT,
    owner_effective_run_id TEXT,

    delete_attempt_count INTEGER NOT NULL DEFAULT 0
        CHECK (delete_attempt_count >= 0),
    next_delete_at TEXT,
    last_delete_error TEXT,
    created_at TEXT NOT NULL,
    claimed_at TEXT,
    delete_pending_at TEXT,
    updated_at TEXT NOT NULL,

    CHECK (
        (scope_epoch = 0 AND scope_identity = '__legacy_api__')
        OR
        (scope_epoch > 0 AND scope_identity <> '__legacy_api__')
    ),
    CHECK (
        state <> 'ready'
        OR (
            lease_expires_at IS NULL
            AND owner_thread_id IS NULL
            AND owner_kind IS NULL
            AND owner_client_intent_id IS NULL
            AND owner_requested_run_id IS NULL
            AND owner_effective_run_id IS NULL
            AND claimed_at IS NULL
        )
    ),
    CHECK (
        state <> 'claimed'
        OR (
            lease_expires_at IS NOT NULL
            AND owner_thread_id IS NOT NULL
            AND owner_kind IS NOT NULL
            AND owner_effective_run_id IS NOT NULL
            AND claimed_at IS NOT NULL
        )
    )
) STRICT;

CREATE INDEX idx_prompt_attachments_ready_expiry
    ON prompt_attachments(state, expires_at)
    WHERE state = 'ready';

CREATE INDEX idx_prompt_attachments_claim_lease
    ON prompt_attachments(state, lease_expires_at)
    WHERE state = 'claimed';

CREATE INDEX idx_prompt_attachments_owner_run
    ON prompt_attachments(owner_effective_run_id, state)
    WHERE state = 'claimed';

CREATE INDEX idx_prompt_attachments_delete_pending
    ON prompt_attachments(state, next_delete_at)
    WHERE state = 'delete_pending';
```

`relative_path` is relative to the configured data directory's dedicated
`prompt-attachments-v1` root. It is never interpreted relative to a client
workspace. The application derives the absolute compatibility `path`; the
database does not persist machine-specific duplicate prefixes.

### 4.4 Marker execution and validation

Each migration runs in one `BEGIN IMMEDIATE` transaction:

1. read its marker from `projection_states`;
2. if absent, create the table/index set, validate it, then insert the marker
   with `source_row_count = 0` in the same transaction;
3. if present, validate exact required columns, constraints, and named unique
   indexes; mismatch is `GaryxDbError::Configuration` and startup stops.

There is no row backfill for dispatch/create: historical metadata is not a
unique truth source. Prompt-file adoption is a runtime file-lifecycle action,
not a truth-table backfill. A future repair of a marked database must use a new
marker such as `*_v2`; v1 is never silently weakened.

### 4.5 One transaction domain

The gateway adds a `ConversationAdmissionRepository` constructed from the
same `GaryxDbService` as production `SqliteThreadStore`. Its atomic methods
receive projection sets from the same shared projection deriver used by
`SqliteThreadStore`; they do not open a second database or derive a competing
truth source. `AppStateBuilder` publishes delivery capabilities only when the
thread store and admission repository share that transaction domain. Tests
that inject a custom non-SQL store either inject a matching repository or run
without these capabilities; they may not silently combine that store with an
unrelated SQLite ledger.

## 5. Dispatch admission and provider handoff

### 5.1 Application boundary

`prepare_chat_request` is split into:

- a read-only normalize/validate phase that produces `PreparedDispatch`, its
  record patch/binding plan, managed attachment IDs, and fingerprint;
- a side-effect-free bridge plan made under the bridge's per-thread dispatch
  guard; and
- a SQLite admission commit that applies any thread patch plus its projections,
  applies any endpoint-owner mutation under the existing endpoint mutator's
  serialization domain, inserts the plan/ledger row, and claims attachments
  only when that plan can consume them.

Parsing and pure fingerprint construction happen first, followed immediately
by the durable-key lookup. A settled replay returns before checking attachment
existence/expiry or reapplying current configuration. Only a genuinely new
admission performs filesystem, thread, binding, and provider-plan validation.
This is what allows an accepted request to replay after its managed attachment
has correctly been deleted.

`/api/chat/start` can historically omit `threadId`. A correlated omission
cannot address the required thread-keyed ledger until routing has resolved an
existing thread or created one. Read-only routing runs first. If it resolves an
existing thread, normal dispatch admission applies. If it would implicitly
create, the server internally uses the same atomic command with the reserved
create intent `implicit:<sha256(clientIntentId)>` in that scope. Public create
intent validation rejects the `implicit:` prefix. This gives legacy
threadless chat a durable mapping (including legacy epoch zero) without
changing its response shape. An uncorrelated threadless request keeps current
behavior and receives no idempotency promise.

No provider call, interrupt, queue write, run-index publication, callback
subscription, or response delivery happens before the admission transaction
commits. A fingerprint conflict is discovered before any of those mutations.

For an existing thread, the supervisor acquires the current
`ThreadRunCoordinator` admission lease before the ledger transaction and the
transaction rechecks the SQL archived/deleted state. The lease is transferred
to a fresh run or dropped after a queued/no-active result. Consequently a
lifecycle commit cannot land between the durable admission decision and the
bridge handoff. Section 6 extends the coordinator with a creation reservation
for the record-does-not-yet-exist case.

The endpoint binding mutator remains the single owner of endpoint uniqueness.
It gains a transaction-oriented entry point used by dispatch/create rather
than being bypassed. Previous owner, new owner, known-endpoint registry,
`thread_records`, and every affected projection are one write transaction.

### 5.2 Detached owner and duplicate join

The gateway maintains a bounded in-process operation cell keyed by the exact
ledger key. The first caller spawns a detached supervisor; HTTP and WS handlers
only await its watch result. Concurrent duplicates with the same fingerprint
join the cell. Settled cells are evicted; in-progress cells are never silently
evicted, and admission returns overload before creating a new cell when the
bound is exhausted. After eviction, duplicates read the durable row.

This matters for a lost response: cancelling a request task or closing a socket
does not cancel provider handoff or the ledger settlement. It also prevents two
same-process calls from racing between the durable read and insert. SQLite's
primary key is the final cross-task/restart authority.

### 5.3 Bridge plan and gate

The bridge's existing per-thread dispatch guard remains the ordering lock. A
durable replay is checked before planning. A first admission is refactored into
a side-effect-free plan and an exact-plan executor:

1. Under the guard, resolve provider/active-run state and allocate all IDs.
2. In one DB transaction, apply the prepared record/binding changes, insert
   the plan as `admitted`, and claim its managed attachments. A direct
   stream-input plan with no active session is instead inserted already
   settled as `not_dispatched/no_active_session` and claims no attachment.
3. In one DB transaction, CAS `admitted -> handoff_started`, increment
   `handoff_attempt`, record the plan/outcome candidates, and set the
   attachment effective-run lease. Losing the CAS means return the durable
   replay; do not call the provider.
4. Execute that exact plan once. Supplied run/pending IDs replace UUID
   allocation inside `run_management.rs`.
5. Settle to `accepted`, `not_dispatched`, `rejected`, or `ambiguous` and return
   the stored identifiers.

A provider API's typed `NotAccepted` result is the only proof that allows an
in-process supervisor to CAS back to `admitted`, recompute a plan, and increment
the attempt. A timeout, cancellation, panic, transport error, or untyped false
after `handoff_started` is ambiguous and never reissued. The current “queue,
then silently interrupt/start if queueing might have failed” path is split so
fallback is allowed only after a definitive no-side-effect result. It can
never produce a second call merely because the client repeated the request.

`rejected` is reserved for a deterministic terminal failure proven before any
provider side effect (for example, the target became archived between pure
validation and commit). Transient pre-gate infrastructure failure leaves the
row `admitted` so the same request can resume safely. Stored diagnostic text is
bounded to 2 KiB.

For direct `stream_input`, a side-effect-free absence of an active session is
settled as `not_dispatched/outcome=no_active_session`. A later replay returns
that same result even if a session has since started. This is required for an
idempotent operation identity.

### 5.4 Restart states

Before listener bind, a startup transition (not a read-route repair) changes
rows left in `handoff_started` by a prior boot to `ambiguous`. `accepted`,
`not_dispatched`, and `rejected` are replay-only. `admitted` proves that the
side-effect gate was never crossed and may be resumed when the same request is
replayed; because request bodies are not duplicated into this ledger, startup
does not autonomously dispatch an `admitted` row.

The state matrix is:

| Durable state | Provider may have seen it | Same-key behavior |
|---|---:|---|
| `admitted` | no | detached owner/replay may safely continue |
| `handoff_started` on current boot | yes/unknown | join owner; never start a second owner |
| `handoff_started` from old boot | yes/unknown | startup changes to `ambiguous`; return same IDs |
| `accepted` | yes | return original run/pending result |
| `not_dispatched` | no | return original no-active result |
| `rejected` | no | return the stored terminal rejection |
| `ambiguous` | yes/unknown | return same correlation and explicit ambiguity |

### 5.5 Additive responses

Existing `status`, `runId`, and `threadId` remain. Start responses add:

```json
{
  "dispatchOutcome": "started",
  "effectiveRunId": "...",
  "pendingInputId": null,
  "deliveryState": "accepted",
  "idempotencyReplay": false
}
```

Queued chat starts expose `dispatchOutcome=queued_to_active_run`, the active
`effectiveRunId`, and the stable `pendingInputId`. Stream-input retains
`queued|no_active_session` and adds `deliveryState` plus
`idempotencyReplay`. An old client ignores these additive fields. An ambiguous
replay is an HTTP `409` with `error=dispatch_ambiguous`, `deliveryState`, and
the same known IDs; it is not misreported as a fresh acceptance.

## 6. Atomic create-and-dispatch command

### 6.1 Route and payload

The additive route is `POST /api/threads/create-and-dispatch`:

```json
{
  "idempotencyScope": { "identity": "...", "epoch": 1 },
  "createIntentId": "...",
  "clientIntentId": "...",
  "thread": {
    "label": null,
    "workspaceDir": null,
    "workspaceMode": "local",
    "agentId": null,
    "model": null,
    "modelReasoningEffort": null,
    "modelServiceTier": null,
    "sdkSessionId": null,
    "sdkSessionProviderHint": null,
    "forkFromThreadId": null,
    "metadata": {}
  },
  "binding": { "botId": "channel:account" },
  "dispatch": {
    "message": "...",
    "attachments": [],
    "images": [],
    "files": [],
    "accountId": "main",
    "fromId": "api-user",
    "metadata": {}
  }
}
```

`binding` is optional. Public clients specify `botId`; the server resolves its
main endpoint and validates compatibility before commit. Initial resolution is
advisory; the final owner and enabled endpoint are re-read under the endpoint
mutator lock immediately before the transaction. Internal tests may construct
the same resolved endpoint plan, but the public command does not add a second
endpoint-key ownership API.

The route supports the existing fresh-thread modes, including managed private
workspace, worktree, agent/model overrides, SDK-session resume, and fork.
Their non-SQL preparation is a durable reservation saga, while client-visible
thread/create/bind/dispatch state still has one commit point.

For clients that intentionally create an empty thread, existing
`POST /api/threads` also accepts the optional pair
`idempotencyScope + createIntentId`. Both or neither must be present. The pair
uses `command_kind=create_only` and commits the thread/projections plus claim
atomically, without inventing a dispatch row. Its fingerprint cannot later be
replayed as `create_and_dispatch`. A message sent after a successful
create-only operation uses normal idempotent chat start on that claimed
thread; a still-threadless message draft uses the atomic command with its own
fresh create intent. Existing bodies with neither field keep current behavior.

### 6.2 Reservation and preparation

After pure validation, one transaction inserts a create-intent row with a
random fixed `thread_id`, fingerprint, and `state=reserved`. A duplicate:

- with another fingerprint returns `409`;
- with the same fingerprint joins the live owner or resumes the same row;
- never allocates another thread ID.

Thread construction is refactored into a draft builder that does not call
`ThreadStore::set`. Managed workspace/worktree/transcript actions use
deterministic thread-ID-based paths and the resource table. The service writes
`materializing` before the action and `materialized` after it. A crash scanner
may clean only a matching owner marker beneath an approved managed root.
Retry uses the same path and thread ID. A transient preparation failure leaves
the row resumable; a definitive validation conflict records
`failed_before_commit` and requires a new create intent.

The detached owner renews the create lease. On startup, an old-boot or expired
preparation lease is recovered before serving: owned materialized resources
are moved through durable `delete_pending`, the create row returns to
`reserved`, and a later equal-fingerprint POST can rematerialize them under the
same fixed thread ID. No request payload is replayed from the database.

### 6.3 Single commit point

Once preparation is complete, the service acquires the endpoint mutator lock
when needed and commits one SQLite transaction containing:

1. the new canonical `thread_records` body;
2. every thread/recent/task/endpoint projection derived from that body;
3. optional previous-owner removal, target binding, and known-endpoint registry
   update;
4. the `chat_start` dispatch-admission row with its fixed requested run ID;
5. every prompt-attachment `ready -> claimed` transition;
6. `thread_create_intents.state = committed`;
7. `thread_create_resources.state = adopted`.

This transaction is the create-and-dispatch linearization point. The command
does not publish a summary, update an in-memory affinity cache, subscribe a
response callback, or call a provider before it commits. After commit, cache
and bridge-affinity updates are derivations; failures do not roll back or hide
the canonical thread. The same detached supervisor performs the provider
handoff through Section 5's gate.

The new-thread ID cannot already have an active run, so its initial exact plan
is a fresh start. The run coordinator receives a creation reservation before
publication and promotes it after the DB commit, preventing lifecycle mutation
from slipping between record creation and `AdmittedRun` ownership.

Lock order is fixed and tested:

1. create-operation cell;
2. optional endpoint mutation lock;
3. SQLite writer transaction;
4. release endpoint lock;
5. bridge per-thread dispatch guard;
6. short SQLite gate/settlement transactions.

No path holds the SQLite writer while waiting for a provider or filesystem.

### 6.4 Response loss and process loss

The first successful response is `201`; a settled replay is `200`. Both carry
the same `threadId`, `createIntentId`, run/pending result, binding result, and
thread summary, plus `idempotencyReplay`.

If the client dies after the single commit but before reading the response, the
detached supervisor continues. Query or POST replay finds the same thread and
the same dispatch admission. It cannot create a ghost or duplicate thread.

If the gateway process dies:

- before the commit, no canonical thread exists; the durable reservation fixes
  the future thread ID and a replay resumes/cleans its managed resources;
- after the commit but before provider gate, the claim is committed and the
  dispatch is safely `admitted`;
- after provider gate, the claim is committed and dispatch becomes
  `ambiguous`, never duplicated.

Thus the thread identity guarantee survives restart even though provider
exactly-once is impossible.

## 7. Create-intent query

The point query is:

`GET /api/threads/by-create-intent?scopeIdentity=...&scopeEpoch=...&createIntentId=...`

It uses the named SQL unique index and never enumerates thread records. `404`
means there is no claim. Claim, current thread body/projections, archived
tombstone, and dispatch row are read from one WAL snapshot so lifecycle cannot
tear the response. A found row returns `200` with:

```json
{
  "createIntentId": "...",
  "threadId": "thread::...",
  "state": "preparing",
  "threadLifecycle": "not_committed",
  "thread": null,
  "dispatch": null
}
```

For `committed`, `thread` is the current summary and `dispatch` is the durable
admission result. If the thread was later archived or deleted, the mapping
still returns `state=committed`, `threadLifecycle=archived|deleted`, and no
summary. It never treats removal as permission to create a replacement for the
same intent. A reserved/preparing claim is not a usable thread; the client
replays the atomic POST with the same fingerprint instead of adopting it.

The old `POST /api/threads`, list shape, point-read shape, bot-bind route, and
chat routes remain compatible. The old create route manufactures a claim only
when it receives the complete new explicit pair described in Section 6.1; new
send clients use the atomic route rather than composing old writes.

## 8. Prompt-attachment ownership and cleanup

### 8.1 Upload contract

`POST /api/chat/attachments/upload` accepts optional
`idempotencyScope`. Explicit-scope recovery clients must provide it. Each
result keeps the compatibility fields and adds:

```json
{
  "attachmentId": "attachment:...",
  "kind": "file",
  "path": "/derived/absolute/path",
  "name": "notes.txt",
  "mediaType": "text/plain",
  "expiresAt": "..."
}
```

`PromptAttachment` gains optional `attachment_id`. Existing path-only clients
continue to work: a path matching a managed row resolves to that row; a path
outside the managed root is unmanaged; a path inside the managed root without
a matching row is rejected. New clients send the ID and the server verifies
ID/path/scope/kind/hash metadata rather than trusting client copies.

Uploads go to `<data-dir>/prompt-attachments-v1/<attachment-id>/payload`.
For a batch, the service writes and fsyncs staging files, atomically renames
them inside that root, fsyncs the affected parent directories, then inserts
every `ready` row in one DB transaction before responding. DB failure removes
the files. A crash between rename and insert leaves an unreferenced file that
the root scanner deletes after a grace period; the scanner never follows
symlinks or leaves the managed root.

The ready TTL is 24 hours. The response exposes the deadline so clients can
re-upload from their local durable payload before use.

### 8.2 Claim and ownership

Managed attachments are single-use by logical dispatch. The admission
transaction verifies `ready`, unexpired, and same explicit scope, then writes
the exact owner tuple and a two-hour renewable lease. A replay by the same
ledger key is allowed; another key or scope receives `409
attachment_already_claimed`. A `no_active_session` stream-input result does not
consume attachments and leaves them `ready`.

An uncorrelated legacy dispatch still receives lifecycle safety: after its
side-effect-free bridge plan, a standalone claim transaction uses the
allocated run/pending identity as owner before provider handoff. It does not
gain dispatch idempotency, but its server-managed file is still released on
terminal/lease expiry.

One upgrade exception is permitted: an unclaimed `ready` row uploaded into
`(__legacy_api__, 0)` may be transferred once to the authenticated request's
explicit scope in the same claim transaction, after exact ID/path/hash match.
A claimed row is never transferred. This covers an app upgrade between upload
and send without making general cross-scope attachment access legal.

For queued input, the bridge plan writes the active `effective_run_id` before
provider handoff. For a fresh run it equals `requested_run_id`. A gateway
run-lifecycle event renews the lease while the run is live.

### 8.3 Terminal and TTL cleanup

The bridge run task owns a non-async drop guard that emits a terminal lifecycle
event through a gateway channel on success, failure, interrupt, abort, or
panic. The lifecycle worker changes all of that effective run's claimed rows
to `delete_pending` before touching the filesystem.

The same transition occurs when:

- an unclaimed row reaches `expires_at`;
- a claimed row's renewable `lease_expires_at` passes (covers process death or
  a missed terminal event);
- a provider accepted the handoff and the owning run reached a terminal event.

A typed provider `NotAccepted` proof before consumption atomically returns the
attachment to `ready` with its original `expires_at` before a replacement plan
is considered. An untyped/post-gate failure remains claimed and ambiguous
until its lease expires; it is never made available to a second logical
dispatch while the first may have consumed it.

The delete worker resolves the stored relative path under the fixed root,
refuses traversal/symlinks, removes the exact file/directory, and deletes the
row only after success. Missing files count as success. Failures increment a
counter and set bounded exponential `next_delete_at`, so a crash after either
the DB transition or physical unlink converges on restart.

The historical root is process-global, while Garyx permits distinct configured
data directories. An old binary does not participate in a new ownership lock,
so a v1 scanner cannot prove that an unreferenced-looking legacy file is not in
use by another gateway. V1 therefore never bulk-deletes that root. When an
authenticated request actually references a legacy UUID path, the gateway may
copy it into its own managed root, verify the content hash, and claim the copy;
it does not delete the unowned original. This makes the referenced delivery
safe without inventing destructive ownership. Historical orphan cleanup needs
an explicit operator/offline migration and is not hidden behind the v1 marker.
Ordinary workspace paths are never adopted, copied, or deleted.

## 9. Compatibility, rollout, and rollback

All wire changes are additive and all new behavior is behind the presence of a
client intent. Existing uncorrelated API, bot, automation, cron, MCP, and
router dispatch remains unchanged. Correlated HTTP and WS requests share the
new service, so transport choice cannot bypass the ledger.

Client impact:

- iOS can keep its current honest ambiguous model. After capability discovery
  it may send explicit scope, use create-and-dispatch, query create claims, and
  upgrade safe ambiguous retries as described in P0-A section 1.6.
- Desktop's existing metadata intent receives legacy-scope dedupe immediately.
  A future durable desktop transport must send explicit scope before enabling
  automatic restart retry.
- Older clients ignore added response fields. Their uploaded files now expire,
  which is an intentional lifecycle contract; the path remains valid until
  the advertised expiry or its owning run settles.
- Internal dispatchers without a client intent do not accidentally accumulate
  ledger rows or change semantics.

An old binary can read the same database because it ignores the additive
tables. It cannot honor new admissions or serve the new route. Therefore
rollback is structurally safe but behaviorally capability-gated: once the
capability disappears, clients stop automatic retry and return to ambiguous
recovery. Rows and files are not down-migrated or dropped; re-upgrade resumes
the recorded v1 contract. The guarantee is suspended while an old binary is
serving: a manual same-key dispatch through that binary is not represented in
the ledger and must not be retried as though v1 were active. A rollback may
delay attachment GC but cannot delete a live attachment. No gateway restart is
part of this task's validation or handoff.

## 10. Implementation slices and validation evidence

After design approval, implementation is split into independently reviewable
commits:

1. dispatch migration/service/bridge supplied IDs and HTTP+WS contract;
2. create-intent migration, point query, and atomic create-and-dispatch;
3. attachment migration, managed upload root, claims, lifecycle worker, and GC;
4. cross-path/restart/fault integration tests and capability publication;
5. Mac canonical durable-delivery consumer and conformance fixtures.

The required deterministic evidence is:

### Dispatch

- A real Axum route -> gateway admission -> `MultiProviderBridge` -> counting
  `ProviderRuntime` integration sends the identical explicit-scope request
  twice and asserts equal IDs and provider run count exactly `1`.
- The active-run variant asserts equal pending/effective IDs and provider queue
  acceptance count exactly `1`.
- Concurrent duplicates, HTTP then WS replay, fingerprint conflict, and
  `no_active_session` replay are covered.
- An on-disk DB reopen proves `accepted` replays without provider calls and
  old-boot `handoff_started` becomes ambiguous without provider calls.
- Failpoints before admission commit, after commit, before gate, after gate,
  and after provider return assert the state matrix in Section 5.4.

### Create and claim

- A deterministic barrier pauses after the all-or-nothing create commit and
  before response publication. The test kills/drops the requester task and
  connection, releases the detached supervisor, reconnects, queries/replays,
  and asserts one create-intent row, one thread record, one projection set, one
  endpoint owner, the same run ID, and one provider dispatch.
- A second on-disk reopen test covers gateway-process loss before commit, after
  commit/before gate, and after gate. It asserts the same thread mapping and no
  duplicate provider handoff.
- Unique-index collision, same-key fingerprint conflict, preparing query,
  committed query, and archived/deleted query are covered.
- Every SQLite failpoint in the multi-record commit leaves either none of the
  thread/claim/admission/binding/attachment changes or all of them.
- Managed workspace/worktree/transcript preparation crash points prove owner
  markers, cleanup, and same-ID resume.

### Attachments

- A fake clock proves ready TTL, claimed lease renewal, run-terminal cleanup,
  expired-lease cleanup after reopen, and retry after unlink/DB crash.
- Scope mismatch, second-owner conflict, idempotent same-owner replay, path
  traversal/symlink rejection, unmanaged workspace non-deletion, and safe
  legacy lazy-copy without source deletion are covered.
- File/DB failpoints prove no acknowledged upload lacks a row/file and orphan
  staging/final files converge through the scanner.

### Validation ladder

- focused crate/unit and desktop conformance tests while iterating;
- `scripts/test/rust_tier1_fast.sh --changed`;
- `scripts/test/rust_tier2_pr.sh`;
- `RUN_EXTERNAL_AI_TESTS=1 scripts/test/rust_tier3_extended.sh`;
- desktop `npm run test:unit` and `npm run build:ui` for the Mac consumer.

External-provider tier 3 is evidence only; correctness does not depend on
restarting the managed gateway.

## 11. Mac DurableDeliveryState follow-up

This batch includes the bounded Mac parity item because the shared fixture is
already canonical and the completion gate requires desktop conformance.

A production-neutral TypeScript module defines the exact arrays and pure
reducers for:

- durable delivery state/evidence/user disposition;
- create delivery phase/user disposition;
- every action used by
  `spec/conversation-state/scenarios/durable-delivery.json`.

The existing desktop conversation-state conformance test imports that module,
asserts every enum against `states.json`, executes every delivery and create
scenario, and changes `platformConsumers.mac` from `p0_g_follow_up` to
`implemented`. This is a real semantic consumer rather than the current
“fixture exists” assertion.

It does **not** claim process-death durability for the current desktop sender:
the desktop has no persisted payload/outbox equivalent to iOS. Wiring the pure
model into a future desktop durable store is a separate product batch and must
not be simulated by adding volatile fields to `MessageIntent`. Gateway
idempotency and the legacy namespace improve live retries, but do not turn a
volatile desktop queue into a durable one.

## 12. Design self-review checklist

- Idempotency truth is the SQLite primary key plus a pre-provider gate, not
  metadata, an in-memory map, or a response cache.
- No crash state permits automatic re-handoff after `handoff_started`.
- A normal client disconnect cannot cancel the admitted operation.
- New thread truth/projections, claim, optional endpoint move, admission, and
  attachment claims share one transaction.
- Endpoint uniqueness still has one mutator/serialization domain.
- Queries are indexed SQL point reads; no record enumeration or read repair.
- Every marker is same-transaction with schema creation and validates marked
  databases fail-closed.
- Rollback removes capabilities rather than deleting protocol history.
- Attachment deletion is limited to root-confined, server-created files and is
  durable before physical unlink.
- HTTP, WS, iOS, desktop, and legacy clients have an explicit compatibility
  story.
- The Mac deliverable is accurately bounded and does not overclaim a durable
  product transport.
