# iOS Xcode Project Identity Drift Review Debt

Status: debt register discovered during the `#TASK-2613` review of the
`#TASK-2609` composer notice removal (`6bdc197b2` red test, `d34ac3cab` fix).
The finding is adjacent and pre-existing; it is not caused by either reviewed
commit and did not block the review verdict.

## Finding

Running `xcodegen generate` in `mobile/garyx-mobile` against the committed
`project.yml` produces a `GaryxMobile.xcodeproj/project.pbxproj` diff of
12 insertions and 12 deletions that is identity-only churn: the file
references and build-file entries for

- `GaryxObservableSettlement.swift`
- `GaryxAgentDetailPresentationTests.swift`
- `AgentDetailPresentationInteractionTests.swift`

were hand-added by earlier tasks with synthetic PBX identifiers (for example
`2598A0012598A0012598A001`), so regeneration re-mints them with canonical
xcodegen-hashed identifiers and reorders the adjacent entries. Target
membership, file paths, and build phases are unchanged.

Regeneration does not resurrect the composer notice files retired by
`d34ac3cab` (`GaryxComposerDurableNoticeViews.swift`,
`GaryxComposerDurableNoticeDebugFixture.swift`,
`DurableDeliveryInteractionTests.swift`); the hand-removal in that commit is
consistent with generated output.

## Impact

The acceptance record for the A4d-2 slice previously asserted "the generated
Xcode project has no post-generation drift". That check is currently not
reproducible: any task that runs `xcodegen generate` as a validation step sees
a spurious pbxproj diff and must either revert it or absorb unrelated churn
into its commit.

## Suggested resolution

A standalone housekeeping change runs `xcodegen generate` once, commits the
canonical identifiers, and restores the drift-free regeneration property. No
source or target membership change is involved.
