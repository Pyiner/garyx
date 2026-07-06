import test from "node:test";
import assert from "node:assert/strict";

import {
  customAgentDeleteConfirmationFor,
  runCustomAgentDeleteConfirmation,
} from "./agents-hub-delete-model.ts";

test("custom agent delete confirmation skips built-in agents", () => {
  assert.equal(
    customAgentDeleteConfirmationFor({
      agentId: "builtin",
      displayName: "Built In",
      builtIn: true,
    }),
    null,
  );
});

test("custom agent delete confirmation uses display name and falls back to id", () => {
  assert.deepEqual(
    customAgentDeleteConfirmationFor({
      agentId: "custom-agent",
      displayName: "Custom Agent",
      builtIn: false,
    }),
    { agentId: "custom-agent", displayName: "Custom Agent" },
  );
  assert.deepEqual(
    customAgentDeleteConfirmationFor({
      agentId: "fallback-agent",
      displayName: " ",
      builtIn: false,
    }),
    { agentId: "fallback-agent", displayName: "fallback-agent" },
  );
});

test("cancelled custom agent delete never calls delete or refresh", async () => {
  const calls = [];
  const result = await runCustomAgentDeleteConfirmation({
    confirmation: null,
    deleteCustomAgent: async () => {
      calls.push("delete");
    },
    closeConfirmation: () => {
      calls.push("close-confirmation");
    },
    closeAgentDialog: () => {
      calls.push("close-agent-dialog");
    },
    loadData: async () => {
      calls.push("load-data");
    },
    refreshAgentTargets: async () => {
      calls.push("refresh-targets");
    },
  });

  assert.equal(result, "cancelled");
  assert.deepEqual(calls, []);
});

test("confirmed custom agent delete deletes before refreshing local and parent catalogs", async () => {
  const calls = [];
  const result = await runCustomAgentDeleteConfirmation({
    confirmation: { agentId: "custom-agent", displayName: "Custom Agent" },
    deleteCustomAgent: async ({ agentId }) => {
      calls.push(`delete:${agentId}`);
    },
    closeConfirmation: () => {
      calls.push("close-confirmation");
    },
    closeAgentDialog: () => {
      calls.push("close-agent-dialog");
    },
    loadData: async () => {
      calls.push("load-data");
    },
    refreshAgentTargets: async () => {
      calls.push("refresh-targets");
    },
  });

  assert.equal(result, "deleted");
  assert.deepEqual(calls, [
    "delete:custom-agent",
    "close-confirmation",
    "close-agent-dialog",
    "load-data",
    "refresh-targets",
  ]);
});
