# TASK-2164: Deterministic iOS Markdown URL Links

## Reproduction and root cause

The captured assistant message is a bullet list whose two visible HTTP URLs
are wrapped in Markdown inline-code delimiters. A headless SwiftPM reproduction
uses that transcript shape and the same `AttributedString(markdown:options:)`
conversion as the app. Before the fix it fails with zero link runs where two
are expected.

Foundation currently autolinks ordinary bare URLs on the development host, but
it intentionally leaves URLs inside inline code as code-only runs. Depending on
Foundation autolinking also makes ordinary bare-URL behavior an OS-owned
implementation detail rather than an app contract. The app then only adds
post-parse links for file paths in
`GaryxMobileMarkdownViews.GaryxMarkdownRenderCache`; it has no deterministic
HTTP(S) annotation pass. The visible inline-code URLs therefore have no
`AttributeScopes.FoundationAttributes.LinkAttribute`, so SwiftUI has nothing
to route through the existing `openURL` action.

This is a client presentation defect. Server `render_state`, transcript row
grouping, and message bodies remain unchanged.

## Design

1. Add one pure `GaryxMobileCore` renderer for an inline Markdown fragment. It
   keeps the app's current Foundation Markdown parsing options, then performs
   link annotation on the rendered character stream.
2. Use Foundation's link data detector over rendered text, but accept a match
   only when the **matched source text** begins (ASCII case-insensitively) with
   `http://`, `https://`, or `www.`. Filtering on the detector result URL is
   insufficient because filename/TLD collisions such as `main.rs` are
   normalized to HTTP URLs. Bare domains, email addresses, and all other
   detector matches are rejected. Character offsets are computed from the
   accepted detector range and mapped back onto the `AttributedString` in
   Core.
3. Reconcile each accepted candidate with existing link attributes:
   - an existing link with the exact candidate range is already correct and is
     left unchanged;
   - a labeled/explicit link that does not look like a self-link is always
     preserved and the overlapping candidate is skipped;
   - an existing HTTP(S) link is an auto-link-shaped range eligible for repair
     only when it starts at the candidate start, strictly extends beyond the
     detector candidate, its visible text starts with an accepted prefix, and
     its target is the canonical URL encoding of that entire visible range
     (`www.` additionally allows the detector's normalized `http://` prefix).
     Core clears `.link` across only that proven self-link range, then annotates
     the detector's narrower candidate.

   The last rule repairs Foundation's reachable failure where a bare URL
   absorbs adjacent Chinese punctuation and prose. A normal explicit
   `[label](destination)` cannot satisfy it: its visible label does not resolve
   to the destination and usually has no URL candidate at all. An explicit
   same-label link whose range already equals the candidate is unchanged. If
   an author deliberately makes a same-label destination include trailing
   prose that the system detector excludes, it is normalized like the
   observationally identical Foundation autolink; a distinct label remains the
   escape hatch for that unusual intent.

   Candidates in fenced code-block presentation intents are skipped.
   Inline-code candidates are eligible: assigning only `.link` preserves their
   existing inline-code presentation intent (while link tint may change their
   color) and makes the captured URLs tappable.
4. Move the existing bare-file-path annotation into the same Core renderer.
   External URL annotation runs first; file-path annotation remains optional
   and keeps its `garyx-path:` target and existing-link guard.
5. Keep `GaryxMarkdownRenderCache` in the app as a cache only. Its uncached
   conversion delegates to the Core renderer. SwiftUI continues to map the
   resulting `AttributedString` into `Text`; the existing `OpenURLAction`
   routes `garyx-path:` links to file preview and returns `.systemAction` for
   HTTP(S), matching current app behavior.

The detector operates after Markdown parsing. Delimiters are already removed,
so inline-code backticks cannot be swallowed into a URL. Detector boundaries
also provide the source of truth used to repair an over-wide Foundation
autolink. Setting a link does not rewrite Markdown or disturb emphasis/code
attributes.

## Behavior and tradeoffs

- Explicit `http://` and `https://` URLs are recognized, including IPv4,
  ports, paths, queries, and fragments.
- Chinese prose before a URL, and trailing Chinese punctuation plus prose
  after that punctuation, remain outside the link. Chinese characters directly
  following a path slash are treated as a valid IRI path instead.
- Visual line wrapping does not split the annotation: the full URL has one
  character range before SwiftUI lays it out.
- `www.` is recognized. Foundation and desktop autolinking already treat that
  form as a link; the system detector supplies its normalized HTTP target.
  This aligns surfaces at the cost of accepting an implicit scheme.
- Bare domains without `www.` are not newly linked. Filename/TLD collisions
  such as `main.rs` and `README.md` are not linked. Foundation's existing
  `mailto:` handling is left intact, but this renderer does not add email
  links itself.
- Inline-code URLs keep their code presentation intent and become tappable.
  Fenced code blocks remain non-interactive and continue through
  `GaryxCodeBlockView`.
- The change applies to all `GaryxMarkdownText` consumers (transcript roles,
  table cells, previews) through the existing shared render/cache seam; no
  view-local regex or server contract change is introduced.

## Validation matrix

Headless `GaryxMobileCore` SwiftPM tests will cover:

- the captured assistant bullet shape with private-IPv4-plus-port and
  domain-plus-port URLs, using reserved synthetic hosts in committed fixtures;
- bare HTTP and HTTPS, IPv4 plus port, and domain plus port/path/query;
- Chinese text plus Chinese/ASCII trailing punctuation with no separating
  whitespace, asserting the punctuation and following prose are outside the
  repaired link;
- an IRI path with Chinese path characters after `/`, asserting those path
  characters remain inside the link;
- a long URL that would wrap in a narrow message bubble, asserting one full
  link range;
- explicit `[label](destination)` preservation;
- inline-code link plus preserved `.code` intent, and fenced-code exclusion;
- `www.` recognition and normalized destination;
- negative cases for prose and inline-code `main.rs`/`README.md`, a bare
  no-scheme domain, and email (no new mail link; an existing Foundation
  `mailto:` link remains untouched);
- existing bare file-path behavior and non-overwrite of existing links.

The same captured-case test must show FAIL before the Core renderer is used and
PASS afterward. Final validation also runs the full SwiftPM suite, regenerates
the Xcode project for the new Core source, and builds the `GaryxMobile` app
target with the iOS simulator SDK.
