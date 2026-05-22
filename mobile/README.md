# Garyx Mobile

This directory contains the Garyx-owned iOS app and mobile gateway adapter.

The phone connects directly to the Garyx gateway on the same LAN, using the
same `gatewayUrl` / `gatewayAuthToken` concept as the Mac app. Provider API
keys stay on the Mac/gateway; iOS only stores the gateway token in Keychain.

Use the Mac app's Gary X Mobile QR/link in Desktop Settings, or enter the Mac's
LAN address manually, for example `http://192.168.1.20:31337`. `127.0.0.1` only
works from the iOS simulator on the Mac itself.

The iOS app is not a shrunken Mac window. It is the mobile control surface for
the same Garyx gateway: chat threads, thread history, run interruption, active
agent/team selection, task creation and status changes, automation run control,
skills visibility, and gateway settings are available as touch-first panels.
Deeper desktop-only editing surfaces still remain on the Mac app.

Open the iOS project at `mobile/garyx-mobile/GaryxMobile.xcodeproj`, or
regenerate it from `mobile/garyx-mobile/project.yml` with:

```bash
cd mobile/garyx-mobile
xcodegen generate
```
