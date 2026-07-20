# P2-F spatial and material acceptance evidence

This record covers the iOS 26-only P2-F polish pass. Visual probes used a
synthetic DEBUG snapshot and were removed after capture; no Capsule or Artifact
was created, and no fixture ships in the app.

## Capsule gallery spatial continuity

The gallery thumbnail publishes its bounds through
`GaryxCapsuleGalleryThumbnailAnchorKey`. One clipped destination canvas samples
`GaryxAnchoredFullscreenMorphGeometry` in both directions, so dismissal does
not hand off to a second transition or a different coordinate system.

- The Core test samples progress `0.0 ... 1.0` in eleven steps and asserts that
  every closing frame reversed has the same origin, size, corner radius, and
  content opacity as its opening counterpart.
- An iOS 26.2 simulator recording captured 68 opening frames and 37 closing
  frames on the production `garyx://mobile/capsules` route. Both sequences
  begin/end at the tapped gallery cell; closing returns to the same still
  mounted cell.
- The final destination preserves safe-area insets. The close and favorite
  glass controls resolve to `y = 72 ... 116 pt` on an iPhone 17 Pro instead of
  entering the status bar or Dynamic Island region.
- Open and close use their existing purpose-specific spring timings. Their
  elapsed time may differ, but the sampled spatial path is the same function in
  reverse.

Local review artifacts:

- `/tmp/garyx-task2468-evidence/capsule-open-production-route.mov`
- `/tmp/garyx-task2468-evidence/capsule-close-production-route.mov`
- `/tmp/garyx-task2468-evidence/capsule-open-production-contact-sheet.png`
- `/tmp/garyx-task2468-evidence/capsule-close-production-contact-sheet.png`
- `/tmp/garyx-task2468-evidence/capsule-open-final.png`

## Glass materialization

Before this pass, the scoped transient surfaces used ordinary opacity or
move/opacity transitions, and the global error toast applied system material as
a background. After this pass, a shared transition modifier drives opacity,
scale, and blur as one material arrival/departure; the toast also applies
interactive adaptive glass directly to its shaped content with an explicit
content shape.

Migrated surfaces are the message and thread action menus, Capsule chrome panel
body, global error toast, image-saved confirmation, and scroll-to-latest glass
button. Task-tree/sidebar movement remains direct-manipulation slide motion in
the standard policy, and native Agents detent sheets retain system
materialization.

An iOS 26.2 toast probe captured the blur resolving while the surface scales
into place, then the exact modifier path in reverse on departure:

- `/tmp/garyx-task2468-evidence/materialize-toast-open.mov`
- `/tmp/garyx-task2468-evidence/materialize-toast-contact.png`

### Accessibility degradation matrix

| Preferences | Active transition state | Result |
|---|---|---|
| Standard | opacity `0`, scale below `1`, positive blur | Materialize with blur + scale + opacity. |
| Reduce Transparency | opacity `0`, scale below `1`, blur `0` | Spatial arrival remains, but neither the transition nor adaptive glass introduces transparency blur. |
| Prefer Cross-Fade | opacity `0`, scale `1`, blur `0` | Fade only; no spatial or filter motion. |
| Reduce Motion | identity state and no animation | Immediate insertion/removal; no materialize motion. |
| Reduce Motion + Prefer Cross-Fade | opacity `0`, scale `1`, blur `0` | The explicit cross-fade preference wins and remains fade only. |

`GaryxMaterializeTransitionTests` asserts all five policy combinations plus
invalid-input clamping in `GaryxMobileCore`.

## Compact editor decisions

The complete conversion list and the retained full-screen rationale are in
[ios-fluid-p2f-editor-presentation-audit.md](ios-fluid-p2f-editor-presentation-audit.md).
The bounded Skill Info, Command create/edit, and Gateway Profile edit forms use
detent sheets; document authoring, hierarchical browsing, connection/auth, and
dynamic multi-section management flows remain full-screen.

## Conversation scroll audit

### Confirmed prepend defect, split from P2-F

The first prepend after a cached thread mounts can jump. It reproduced on two
cold simulator launches with the same sequence:

1. Mount the conversation with turns 12 through 23 already cached before the
   view appears.
2. Browse upward until Response 14 is at the top of the viewport.
3. Prepend turns 0 through 11 while retaining all existing rows.

Actual output moved the viewport to Response 2. Probe logs contained the seed
and prepend events but neither an exact-compensation event nor the coarse
`scrollTo` fallback. `rowScrollPreservationThreadId` starts unset and is not
initialized on first appearance, so the first row-ID change is classified as a
thread change and the restore request is never created.

This pass deliberately does not change conversation code. The bug was split to
`#TASK-2488` under the reproduce-first bug workflow.

Local evidence:

- `/tmp/garyx-task2468-evidence/prepend-before.png`
- `/tmp/garyx-task2468-evidence/prepend-after.png`
- `/tmp/garyx-task2468-evidence/prepend-before-repeat.png`
- `/tmp/garyx-task2468-evidence/prepend-after-repeat.png`
- `/tmp/garyx-task2468-evidence/prepend-probe.mov`

### Streaming tail retry result

The streaming-tail jump did not reproduce. Ten updates grew one assistant
reply at 50 ms intervals. Each update invalidated the older retry generation;
only still-current, non-animated attempts ran, and the final generation settled
at 0, 40, and 140 ms. The recorded bottom gap remained visually continuous
with no backward hop or animated double-scroll.

- `/tmp/garyx-task2468-evidence/tail-stream-probe.mov`
- `/tmp/garyx-task2468-evidence/tail-stream-contact.png`

The focused `swift test --filter GaryxConversation` run executed 62 tests with
zero failures, including prepend displacement, user-interaction gates, retry
cancellation after leaving the tail, and rising-edge repair behavior.

## Deterministic validation

- `xcodegen generate` completed and left the generated project unchanged.
- Full `swift test` executed 1,440 tests with zero failures in 305.5 seconds.
- `xcodebuild` built the `GaryxMobile` Debug scheme for a generic iOS Simulator
  from isolated DerivedData with code signing disabled (exit status `0`). The
  emitted warnings are pre-existing iOS 26 deprecations outside this pass.
