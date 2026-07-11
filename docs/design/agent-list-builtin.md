# Design: `garyx agent list` shows built-in agents by default

Task: #TASK-202

## Review outcome (binding)

Decisions overriding earlier proposals in this doc:

1. Default lists all agents; no opt-out flag (`--custom-only` / `--no-builtin`)
   added.
2. `--include-builtin` is removed outright (no hidden no-op).
3. No section headers in text output — one clean list, builtin-first then
   custom, with the existing `(built-in)` row suffix as the only marker.
4. `--json` still gains a per-agent `kind: "builtin" | "custom"` discriminator
   alongside the existing `built_in` field, and the array is sorted to match
   text output.

The narrative below reflects the original analysis; the open questions at
the bottom were resolved by the decisions above.

## TL;DR

The infrastructure for built-in agents already exists end-to-end. The only
reason `garyx agent list` hides them is a CLI flag that defaults to off. The
change is small and CLI-local — no gateway or models edits required.

## Current state (verified)

- `garyx-gateway/src/custom_agents.rs`
  - `CustomAgentStore::new()` and `::file()` seed the built-in provider
    profiles (`claude`, `codex`, `traex`, `antigravity`) before merging
    persisted custom agents on top. (lines 54–90)
  - `list_agents()` returns the union and sorts `built_in=false` first
    (because `false < true` in `bool::cmp`), then by `display_name`.
    (lines 92–106)
- `garyx-gateway/src/api.rs::list_custom_agents` (lines 2998–3010) returns
  `{ "agents": [...] }`. Each entry carries `built_in: bool` plus all the
  fields enumerated in `garyx-models/src/custom_agent.rs::CustomAgentProfile`.
- `garyx-models/src/custom_agent.rs::builtin_provider_agent_profiles()`
  (lines 124–195) is the source of truth for the built-ins. Each is
  constructed with `built_in: true`.
- `garyx/src/cli.rs::AgentAction::List` already exposes
  `--include-builtin` (default `false`) and `--json`. (lines 1140–1150)
- `garyx/src/commands.rs::cmd_agent_list` (lines 3390–3414):
  - Hits `GET /api/custom-agents`.
  - In `--json` mode: pretty-prints the raw payload as-is.
  - In text mode: filters out built-ins unless `--include-builtin`, then
    walks the remaining entries through `print_agent_summary` which already
    appends `" (built-in)"` to the `Agent: <id>` header for builtin rows
    (line 3640–3643).
- Test coverage that already asserts built-ins show up on `/api/custom-agents`:
  `garyx-gateway/src/api/tests.rs::test_create_and_list_custom_agents`
  (lines 799–854) — verifies `claude` is returned with avatar + provider_icon.

So today: `garyx agent list` (no flags) silently drops the three built-ins
even though the gateway is serving them.

## Goal

`garyx agent list` (no flags) should show every agent the system can route
to — built-ins **first**, customs second — with a clear inline marker, and
`--json` should let consumers programmatically tell them apart.

## Design

### Behavioral changes

1. **Flip the default.** Remove the gate on `include_builtin`. The plain
   `garyx agent list` shows built-in **and** custom agents.
2. **Add an opt-out flag** `--custom-only` for users who want today's
   custom-only view (and for any scripts that rely on terse output). This is
   the new way to get the old behavior.
3. **Keep `--include-builtin` as an accepted no-op** so any existing
   scripts/docs that pass it don't break. Hide it from help text via
   `#[arg(long, hide = true)]` and add a deprecation note in the help block
   comment.
4. **Group + sort in the CLI handler.** Don't rely on the gateway's sort
   (which today is `custom < builtin` and orders by `display_name`).
   Re-sort to satisfy the task spec:
   - Builtins first, then customs.
   - Within each group, sort by `agent_id` ascending (ASCII order).
5. **Text output gets a section header.** Two clearly-labeled sections,
   each followed by its agent rows. If a section has zero entries it is
   omitted (so a `--custom-only` user doesn't see a stray `Built-in agents:`
   line). The per-row `(built-in)` suffix on the `Agent:` line stays — it's
   useful when piping into grep — but the section header is what makes the
   layout scannable.

   Sketch:

   ```
   Built-in agents:

   Agent: claude (built-in)
   Name: Claude
   Provider: claude_code
   ...

   Agent: codex (built-in)
   ...

   Custom agents:

   Agent: gary
   Name: Gary
   ...
   ```

6. **JSON output: pass-through + one new field per agent.**
   - Keep every existing field byte-for-byte. The `built_in: bool` field
     stays where it is.
   - Add a sibling `kind: "builtin" | "custom"` per agent, computed from
     `built_in`. Strict superset → backward compatible per task constraint.
   - Apply the same sort as text mode so JSON consumers that iterate the
     array get a deterministic, documented order.
   - **Do not** add a `kind` field at the top level or restructure into
     `{ builtin: [...], custom: [...] }` — that would break consumers.
   - `--custom-only` filters the array before emission.

   Why surface `kind` at all if `built_in` already exists? Two reasons:
   the spec asks for it explicitly, and a string discriminator reads more
   naturally in downstream tooling (`jq 'select(.kind=="builtin")'`) than
   a boolean.

### Code touch points

| File | Change |
| --- | --- |
| `garyx/src/cli.rs` | Replace `--include-builtin` flag with `--custom-only`; keep the old flag as `hide=true` no-op for compat. |
| `garyx/src/commands.rs` | Rewrite `cmd_agent_list`: drop the include_builtin filter; add custom_only filter; sort builtin-first/agent_id-asc; emit section headers in text mode; inject `kind` field in JSON mode. |
| `garyx/src/commands.rs` | `print_agent_summary` is unchanged — the `(built-in)` suffix is already correct. |
| `garyx/src/main.rs` (or wherever the CLI dispatch lives) | Update the call site to forward `custom_only` instead of `include_builtin`. Pass the legacy flag through but ignore it. |
| Tests (see below) | New CLI-level + integration tests. |

No edits to `garyx-gateway`, `garyx-models`, `garyx-router`, `garyx-bridge`,
desktop, or mobile.

### Sort key details

```rust
agents.sort_by(|a, b| {
    let a_builtin = a["built_in"].as_bool().unwrap_or(false);
    let b_builtin = b["built_in"].as_bool().unwrap_or(false);
    // true should sort before false → reverse the natural bool order.
    b_builtin.cmp(&a_builtin)
        .then_with(|| {
            let a_id = a["agent_id"].as_str().unwrap_or("");
            let b_id = b["agent_id"].as_str().unwrap_or("");
            a_id.cmp(b_id)
        })
});
```

### JSON `kind` injection

Operate on a mutable clone of the `agents` array, attach `kind` per element,
re-wrap with the original top-level shape (`{"agents": [...]}`), and
pretty-print. Other top-level fields (none today) pass through untouched.

## Backward compatibility

- JSON: additive only (`kind` added). `built_in` retained. Top-level
  structure preserved. Consumers reading the existing fields are unaffected.
- CLI flags: `--include-builtin` keeps parsing successfully (no-op). Scripts
  passing it continue to work. `--custom-only` is new.
- Text output **does** change shape — section headers appear. Anyone parsing
  the text output (which they shouldn't) might break. Mitigation: text
  output is documented as human-readable; programmatic consumers must use
  `--json`. Acceptable.

## Test strategy

Three layers:

1. **Pure unit test for the sort + filter helper.** Extract the sort/filter
   logic into a small free function `partition_agents(payload, custom_only)
   -> Vec<&Value>` and unit-test it in `garyx/src/commands.rs` `mod tests`
   with synthetic JSON. Covers ordering and `--custom-only` filtering.
2. **Snapshot-ish text rendering test.** Feed a small synthetic payload
   (two builtins + two customs in random order) into the text renderer
   (refactored into a function that writes to a `&mut impl Write`) and
   assert the captured output: header order, row order within each group,
   absence of the builtin header when `--custom-only` is on, etc.
3. **JSON serialization test.** Run the JSON path on the same synthetic
   payload and assert: (a) every original field is preserved, (b) each
   element gains `kind` matching `built_in`, (c) array order matches
   builtin-first / agent_id-asc.

Integration / e2e: The existing gateway test
`test_create_and_list_custom_agents` already pins the gateway side. A new
end-to-end CLI test would require booting a gateway and is out of scope —
the three unit tests above cover the CLI logic deterministically.

Build/check:
- `cargo build -p garyx`
- `cargo test -p garyx commands::tests` (or the relevant module path once
  the test module is in place)
- `cargo fmt` / `cargo clippy -p garyx`

## Out of scope

- Changing the gateway's `list_agents()` sort order (would affect desktop +
  mobile + MCP).
- Surfacing `kind` in the gateway response (server-side change with broader
  blast radius; can be a follow-up if mobile/desktop want the same).
- New agent fields, new providers, new identity surfaces.
- Touching `garyx agent get` (single-agent view) — already shows
  `(built-in)` correctly via `print_agent_summary`.

## Rollback

Revert the single commit. JSON consumers see the `kind` field disappear;
all other behavior reverts cleanly.

## Open questions for review

1. **Flag name for the opt-out:** `--custom-only` reads cleanly. Alternative:
   `--no-builtin`. I prefer `--custom-only` because it mirrors how
   `garyx agent` users talk about the two buckets ("custom agents" vs
   "built-in agents"). Happy to change if reviewer prefers otherwise.
2. **Should `--include-builtin` be removed outright?** Keeping it as a
   hidden no-op is safer; removing it is cleaner. My pick is keep-as-no-op
   for one release cycle, then drop. Reviewer's call.
3. **Section headers vs inline `[builtin]` tags only?** Task description
   accepts either ("先列内置 agent（标注 `[builtin]` 之类的标签），再列
   custom agent；或者用 section 分组显示"). I went with section headers
   because the per-agent block is already multi-line; a single-line
   `[builtin]` tag would get lost in the noise. Open to inline tags if the
   reviewer disagrees.
