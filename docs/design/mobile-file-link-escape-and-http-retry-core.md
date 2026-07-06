# Mobile: reject workspace-escaping relative file links + single HTTP retry core

Task: #TASK-1754. Two audited GaryxMobileCore fixes, both behavior-first:
a failing test reproduces #1 before the fix; #2 is frozen by
current-behavior tests before the refactor.

## 1. Relative file links that escape the workspace root must be rejected

### Current behavior (bug)

`GaryxMobileFileLink.previewTarget(fromLink:...)` resolves a relative
markdown link against the current preview file, then folds the combined
path in `normalizeRelativePath` (GaryxMobileFileLinks.swift:187). When a
`..` component arrives with an empty stack, the loop silently drops it:

```swift
if part == ".." {
    if !stack.isEmpty {
        stack.removeLast()
    }
    continue   // out-of-root `..` vanishes
}
```

From `docs/readme.md`, a link to `../../secret.txt` folds to
`docs/../../secret.txt` → stack `[docs]` → pop → drop `..` → `secret.txt`.
The preview then requests workspace-root `secret.txt` — a real file the
gateway happily serves. The user taps a link that points outside the
workspace and gets shown an unrelated in-workspace file of the same name.

### Semantics elsewhere

- Gateway (`garyx-gateway/src/workspace_files.rs`,
  `normalize_relative_path`): any `ParentDir`/`RootDir`/`Prefix` component
  is a 400 `bad_request` ("path must stay within the workspace root").
  The gateway never folds `..`; it expects clients to send already-folded
  in-root paths.
- Desktop: message links only intercept absolute local file paths
  (`localFilePathFromMessageLinkHref`); relative links are not resolved
  into workspace preview targets at all, so there is no desktop folding
  precedent to match.

Mobile must keep client-side folding (a relative `../sibling.md` link from
`docs/readme.md` is legitimate and the gateway rejects raw `..`), so the
folding step is exactly where escape detection belongs.

### Fix

`normalizeRelativePath` returns `nil` when folding underflows the root:

```swift
if part == ".." {
    guard !stack.isEmpty else { return nil }   // escapes the root → reject
    stack.removeLast()
    continue
}
```

`nil` propagates through `workspaceRelativePath` →
`previewTarget(fromLink:...)` returns `nil` → the link is not treated as a
workspace file link (no preview opens, same as any other unresolvable
link). This matches the gateway's "reject" semantics rather than invent a
new "clamp to root" semantic, and it never sends a request that either
400s (`..` kept) or serves the wrong file (`..` dropped).

In-bounds traversal is unchanged: `docs/../readme.md` → `readme.md`, and
`../../index.html` from `docs/notes/readme.md` → `index.html` (folds to
exactly the root, no underflow).

### Impact surface

`normalizeRelativePath` has two callers, both inside
GaryxMobileFileLinks.swift:

- `workspaceRelativePath` (the bug path): behavior change is
  reject-instead-of-collapse, covered by new tests.
- `parentRelativeDirectory(currentFilePath)`: normalizes the *current
  file* path before taking its parent. `currentFilePath` is app-managed
  state (the workspace-relative path of the file already being previewed),
  produced by earlier previewTarget calls, so it cannot legitimately
  escape. If it ever does (upstream bug), normalize now yields `nil` and
  the existing `?? ""` fallback treats it as the workspace root — same
  fallback already used for empty paths today. No caller outside the file;
  no UI change.

Failing test written first (red on current code):
`testMobileFileLinkRejectsRelativeLinksEscapingWorkspaceRoot` — covers
nested-file escape (`../../secret.txt` from `docs/readme.md`), root-file
escape, no-context escape, and deep escape (`../../../etc/passwd`).
Guard test `testMobileFileLinkKeepsInBoundsParentTraversalWorking` pins
in-bounds folding (including traversal to exactly the root).

## 2. Single HTTP request core for JSON and text sends

### Current behavior (duplication)

`GaryxGatewayClient.send<Response>` (JSON, :1017) and `sendText` (:1065)
duplicate the entire retry loop line-for-line: attempt counting,
2xx gating, `httpStatus` construction, `isRetryableStatus` +
`sleepForRetry` (Retry-After aware), `GaryxGatewayError` rethrow,
cancellation passthrough, connection-establishment retry, and
idempotent-only ambiguous-network retry. The only real difference is the
success-path body handling:

- `send`: empty-body `GaryxEmptyResponse` special case, else JSON decode.
- `sendText`: UTF-8 decode, else `GaryxGatewayError.encodingFailed`.

Any future change to the retry classifier, Retry-After handling, or
cancellation semantics has to be made twice and can silently drift.

### Refactor shape

Extract the retry loop into one private core that returns the successful
response body; both entrypoints become thin decoders:

```swift
private func sendRaw(_ request: URLRequest, idempotent: Bool) async throws -> Data {
    // existing retry loop, verbatim; 2xx returns data
}

private func send<Response: Decodable>(_ request: URLRequest, idempotent: Bool) async throws -> Response {
    let data = try await sendRaw(request, idempotent: idempotent)
    if data.isEmpty, Response.self == GaryxEmptyResponse.self {
        return GaryxEmptyResponse() as! Response
    }
    return try decoder.decode(Response.self, from: data)
}

private func sendText(_ request: URLRequest, idempotent: Bool) async throws -> String {
    let data = try await sendRaw(request, idempotent: idempotent)
    guard let text = String(data: data, encoding: .utf8) else {
        throw GaryxGatewayError.encodingFailed("The Garyx gateway returned non-UTF-8 text.")
    }
    return text
}
```

### Behavior conservation argument

Moving body decoding out of the loop is behavior-neutral because a 2xx
response always terminates the loop today:

- JSON decode failure on 2xx currently throws `DecodingError` inside the
  `do`, falls into the generic `catch`, classifies as non-retryable
  (not cancellation / not connection-establishment / not ambiguous+idempotent),
  and rethrows without sleeping. After the refactor the same
  `DecodingError` is thrown outside the loop — same error, same
  single-attempt, no delay.
- Non-UTF-8 text on 2xx currently throws `encodingFailed`, is caught by
  `catch let error as GaryxGatewayError` and rethrown without retry. After
  the refactor it is thrown outside the loop — identical.
- Every non-2xx and transport-error path moves verbatim into `sendRaw`:
  retry counts, `isRetryableStatus` idempotency gating, Retry-After
  max(retryAfter, computed) delay, cancellation checks before/after the
  sleep, and error mapping are untouched.

Frozen-behavior tests (green before and after the refactor) cover both
routes: text 503 retry / 404 no-retry / non-UTF-8 no-retry /
connection-error retry; JSON decode-failure no-retry; Retry-After 429
retry; cancellation during the retry delay stops further attempts. The
pre-existing JSON retry tests (503 idempotent, connection-lost POST,
503-vs-502 non-idempotent POST) continue to apply.

### Impact surface

Private methods only; every public API keeps its exact signature and
error surface. `sendText` has a single call site (`getText`, used by
`capsuleHTML`). No new files, so no xcodegen/pbxproj changes.
