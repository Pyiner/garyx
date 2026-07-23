# iOS Push Notifications Review Debt

## Existing CLI gateway-client test has a port-reuse race

- Source: full `cargo test -p garyx` validation for TASK-2650 on 2026-07-24.
- Existing location: `garyx/src/commands/gateway_client.rs`, test
  `persistent_refusal_exhausts_connect_attempts` (introduced before this task).
- Evidence: the test releases an ephemeral listener before issuing its request.
  During one parallel full-suite run another listener claimed that address, so
  the request received an HTTP response and was classified as `Rejected`
  instead of the expected `Unreachable`. An immediate isolated rerun passed
  (1 passed, 0 failed).
- Disposition: independent test-isolation debt. It is outside the iOS push
  implementation and is intentionally not changed in TASK-2650.
