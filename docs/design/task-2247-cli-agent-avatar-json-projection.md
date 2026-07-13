# TASK-2247: omit agent avatar blobs from CLI JSON

## Problem and baseline

The custom-agent Gateway API includes `avatar_data_url` because desktop and
mobile clients render agent avatars from that response. The CLI currently
pretty-prints the same response without projecting it for terminal use.

The prescribed local reproduction was deterministic before any source edit:

| Command | Result |
| --- | ---: |
| `garyx agent get <avatar-agent> --json \| wc -c` | 148,306 bytes |
| `.avatar_data_url \| length` | 137,186 characters |
| `garyx agent list --json \| wc -c` | 542,205 bytes |
| data URLs in that list | 8 entries / 518,416 characters |

The single-agent value began with `data:image/`, so the size is the encoded
image body rather than useful agent metadata.

## Output-path audit

`garyx/src/commands/agent.rs` owns every custom-agent CLI response:

| CLI path | Pre-fix behavior | Avatar risk |
| --- | --- | --- |
| `agent list --json` | decorates/sorts, then prints the Gateway payload | every agent row |
| `agent get --json` | prints the Gateway payload | one agent row |
| `agent create --json` | prints the mutation response | response field (normally null) |
| `agent update --json` | prints the mutation response | preserved avatar body |
| `agent upsert --json` | prints the mutation response | preserved avatar body |
| `agent delete --json` | prints the delete response | none today, but same output boundary |
| all human-readable agent output | prints selected scalar fields only | none |

The raw body originates in `garyx-gateway/src/api.rs::custom_agent_response`.
That server contract must not change: the desktop main-process client and the
iOS Gateway client call `/api/custom-agents` directly and use the full avatar.

Repository-wide reference searches found no desktop, mobile, script, test, or
other internal consumer that shells out to `garyx agent ... --json` for
`avatar_data_url`. `garyx task create` quota preflight fetches and deserializes
the Gateway profile internally but never prints it. The interactive channel
setup reads profiles locally and formats only agent identity labels. No other
CLI JSON route serializes an agent avatar field.

## Decision

CLI agent JSON will omit every object member whose exact key is
`avatar_data_url`. All other fields, values, list ordering, and the list-only
`kind` decoration remain unchanged. Human-readable output remains unchanged.

The projection is CLI-local and recursive within an agent-command response, so
one shared output boundary covers list envelopes, single-agent responses,
mutation responses, and any future nested agent response shape. The Gateway,
stored `CustomAgentProfile`, desktop, mobile, and provider runtime contracts
remain untouched.

### Tradeoffs

- **Omission rather than `null`:** `null` would falsely claim that an agent has
  no avatar. A missing key accurately says the CLI projection did not include
  that expensive field.
- **Omission rather than a summary string:** a placeholder would violate the
  field's data-URL meaning and could be mistaken for a renderable value by a
  loosely typed consumer.
- **No full-body CLI flag:** repository consumers do not need one, while adding
  a permanent escape hatch would expand the CLI solely to reproduce the noisy
  behavior being removed. Consumers that genuinely need image bytes already
  have the authoritative Gateway API used by desktop and mobile.
- **Compatibility:** this intentionally narrows only the undocumented avatar
  member of CLI JSON. Known machine consumers select fields such as
  `system_prompt`; those fields and the top-level shapes do not change. The
  direct Gateway wire contract remains backward compatible.

## Implementation

1. Add one pure helper in `garyx/src/commands/agent.rs` that walks a
   `serde_json::Value` and removes exact `avatar_data_url` object keys.
2. Add one agent-specific JSON print helper and route every agent `--json`
   branch through it. `agent list` applies its existing sort/`kind` decoration
   before the projection.
3. Document the CLI projection in the CLI/configuration reference. Do not
   change the Gateway response or shared agent model.

## Validation

- Unit tests prove the projection removes root and nested/list avatar fields,
  preserves similarly named fields and all unrelated values, and handles null
  and empty shapes.
- Existing agent command tests and `cargo test -p garyx --all-targets` must
  pass; run the repository's changed-Rust fast tier as the broader focused
  check.
- Build the local CLI through the repository script, then repeat the exact
  pre-fix commands. `agent get --json` must be KB-scale, both get/list JSON must
  have zero `avatar_data_url` keys and zero `data:image` bodies, and the human
  output must remain free of image data.
- Record the actual installed-before/worktree-after byte counts from the same
  Gateway state so the reduction is attributable to the CLI projection.
