# TestFlight Release

Garyx iOS TestFlight releases are driven by the `TestFlight` GitHub Actions
workflow.

## Required GitHub Secrets

- `APPLE_TEAM_ID`
- `APP_STORE_CONNECT_API_KEY_ID`
- `APP_STORE_CONNECT_API_ISSUER_ID`
- `APP_STORE_CONNECT_API_KEY_P8`
- `IOS_DISTRIBUTION_CERTIFICATE_P12_BASE64`
- `IOS_DISTRIBUTION_CERTIFICATE_PASSWORD`
- `IOS_PROVISIONING_PROFILE_BASE64`
- `TESTFLIGHT_TESTER_EMAILS`

`APP_STORE_CONNECT_API_KEY_P8` may be stored either with real newlines or with
escaped `\n` newlines. The workflow writes the key to the temporary runner
keychain path and never prints it.

The iOS signing secrets are used only when `upload_build` is enabled. The
workflow imports the distribution certificate and App Store provisioning profile
into a temporary keychain, uses manual signing for archive/export, then deletes
the keychain at the end of the job.

## Optional GitHub Variables

- `GARYX_APP_NAME` defaults to `Garyx`.
- `GARYX_APP_SKU` defaults to `garyx-ios`.
- `IOS_BUNDLE_ID` defaults to `com.garyx.mobile`.
- `TESTFLIGHT_GROUP_NAME` defaults to `Garyx Experimental`.

## Workflow Behavior

The workflow can do two independent things:

- Register/update the App Store Connect app, Bundle ID, beta group, and testers.
- Archive, export, and upload the iOS app to TestFlight.

The setup step is idempotent. It reuses existing App Store Connect resources
when they already exist. If App Store Connect already has an internal group with
the configured `TESTFLIGHT_GROUP_NAME`, the setup step creates or reuses an
external group named `<TESTFLIGHT_GROUP_NAME> External` for beta tester email
invitations, because external beta testers cannot be attached to an internal
tester group.

Run it from GitHub Actions with `workflow_dispatch`. The optional build number
input maps to `CFBundleVersion`; when omitted, the GitHub run number is used.
