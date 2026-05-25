# iOS TestFlight Release

Garyx iOS TestFlight releases are driven by the `iOS TestFlight` GitHub Actions
workflow. This workflow is intentionally separate from the macOS/gateway release
workflow; iOS builds are uploaded only when this workflow is run manually.

## Required GitHub Secrets

- `APPLE_TEAM_ID`
- `APP_STORE_CONNECT_API_KEY_ID`
- `APP_STORE_CONNECT_API_ISSUER_ID`
- `APP_STORE_CONNECT_API_KEY_P8`
- `IOS_DISTRIBUTION_CERTIFICATE_P12_BASE64`
- `IOS_DISTRIBUTION_CERTIFICATE_PASSWORD`
- `TESTFLIGHT_TESTER_EMAILS`

`APP_STORE_CONNECT_API_KEY_P8` may be stored either with real newlines or with
escaped `\n` newlines. The workflow writes the key to the temporary runner
keychain path and never prints it.

The iOS signing secrets are used only when `upload_build` is enabled. The
workflow imports the distribution certificate into a temporary keychain, then
uses Xcode automatic provisioning with the App Store Connect API key for the app
and its widget extension. Automatic provisioning is required because the iOS app
and widget share an App Group entitlement, so stale single-target provisioning
profiles can break archive signing. The temporary keychain is deleted at the end
of the job.

## Optional GitHub Variables

- `GARYX_APP_NAME` defaults to `Garyx`.
- `GARYX_APP_SKU` defaults to `garyx-ios`.
- `IOS_BUNDLE_ID` defaults to `com.garyx.mobile`.
- `IOS_WIDGET_BUNDLE_ID` defaults to `<IOS_BUNDLE_ID>.RecentThreadsWidget`.
- `TESTFLIGHT_GROUP_NAME` defaults to `Garyx Experimental`.

## Workflow Behavior

The workflow can do two independent things:

- Register/update the App Store Connect app, app and widget Bundle IDs,
  internal beta group, and internal testers.
- Archive, export, upload the iOS app to TestFlight, wait for Apple processing,
  and assign the build to the internal beta group.

The setup step is idempotent. It reuses existing App Store Connect resources
when they already exist. `TESTFLIGHT_TESTER_EMAILS` must refer to App Store
Connect users who can be assigned as internal testers. If Apple rejects a tester
assignment because of the account state, the workflow logs a warning and keeps
publishing the build to the internal beta group.

The shared App Group must be configured in Apple Developer manually for both the
app Bundle ID and the widget Bundle ID. For the default bundle IDs, assign
`group.com.garyx.mobile` to `com.garyx.mobile` and
`com.garyx.mobile.RecentThreadsWidget`. App Store Connect API keys can register
Bundle IDs and let Xcode manage provisioning profiles, but they cannot assign a
specific App Group identifier through the supported Bundle ID capabilities API.

The upload path verifies that the archived app contains AppIcon outputs before
exporting the IPA. After upload, it waits for the requested build number to reach
`VALID` processing state and then confirms the build is attached to the internal
TestFlight group. It does not submit builds for external TestFlight Beta App
Review.

Run it from GitHub Actions with `workflow_dispatch`. The optional build number
input maps to `CFBundleVersion`; when omitted, the GitHub run number is used.
