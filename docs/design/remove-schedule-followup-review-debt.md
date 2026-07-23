# Delayed Followup Removal Review Debt

Date: 2026-07-23.

## Existing Workspace Warnings

Implementation validation found two pre-existing `dead_code` warnings during
`cargo check --workspace`:

- `SqlEndpointBindingMutator::new` in
  `garyx-gateway/src/endpoint_binding_mutator.rs`.
- `workspace_display_name` in `garyx-gateway/src/workspaces.rs`.

Neither file is touched by the removal, and neither warning is caused by its
call-graph changes. Deciding whether to remove, use, or restructure these
items is separate work and must not expand this task.
