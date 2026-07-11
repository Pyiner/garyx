# Validation Commands

Use the narrowest reliable validation for the touched area.

## Fast Local Loop

Prefer changed-package or single-file checks while iterating:

```bash
scripts/test/rust_tier1_fast.sh --changed
cd desktop/garyx-desktop && npm run test:unit -- src/renderer/src/render-view-model.test.mjs
cd desktop/garyx-desktop && npm run test:unit -- --list
```

## Broad Checks

```bash
cargo test --workspace --all-targets
cd desktop/garyx-desktop && npm run build:ui
cd desktop/garyx-desktop && npm run test:smoke
```

## Mobile Swift

Run SwiftPM tests from the mobile package and build the app target against the
iOS simulator SDK:

```bash
cd mobile/garyx-mobile && swift test
cd mobile/garyx-mobile && xcodebuild -project GaryxMobile.xcodeproj -target GaryxMobile -sdk iphonesimulator -configuration Debug build
```

If the scheme-level simulator build fails before compilation because Xcode
cannot resolve an eligible destination, use the target-level build above to
validate the same app target.

## Desktop Packaging

When a packaged app is requested, or when validating packaging, install,
release, or startup behavior, run the packaging flow and launch the installed
app:

```bash
cd desktop/garyx-desktop && npm run dist:dir
open -a Garyx
```

## Narrow Rust Checks

Run the package-level target that matches the edit:

```bash
cargo test -p garyx-gateway --lib
cargo test -p garyx-router --all-targets
cargo test -p garyx-channels --lib
```

## Rust Worktree Cache

Garyx disables Cargo incremental output and full debugger symbols for dev/test
profiles, then uses `scripts/sccache-rustc-wrapper.sh` to share compiler output
across concurrent Git worktrees. The wrapper normalizes the current checkout
root before invoking `sccache`; if `sccache` is unavailable it invokes `rustc`
directly, so CI and fresh machines keep working.

Install the local cache on macOS and inspect its effectiveness with:

```bash
brew install sccache
sccache --show-stats
```

The repository config caps the local cache at 20 GiB. Keep each active
worktree's own `target` directory so Cargo builds do not serialize on one build
directory, and remove the whole worktree after its task is approved.
