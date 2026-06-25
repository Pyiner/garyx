import test from "node:test";
import assert from "node:assert/strict";

import {
  buildThreadAvatarCatalog,
  resolveTaskAvatarIdentity,
} from "./thread-avatar.ts";

function agent(overrides) {
  return {
    agentId: "reviewer",
    displayName: "Reviewer",
    description: "",
    providerType: "claude",
    providerIcon: null,
    builtIn: false,
    avatarDataUrl: null,
    ...overrides,
  };
}

function team(overrides) {
  return {
    teamId: "product-team",
    displayName: "Product Team",
    description: "",
    leaderAgentId: null,
    agents: [],
    avatarDataUrl: null,
    ...overrides,
  };
}

function task(overrides) {
  return {
    assignee: null,
    executor: null,
    runtimeAgentId: "",
    ...overrides,
  };
}

test("task executor avatar uses catalog display name and image", () => {
  const catalog = buildThreadAvatarCatalog(
    [
      agent({
        agentId: "reviewer",
        displayName: "Reviewer",
        avatarDataUrl: "data:image/png;base64,cmV2aWV3ZXI=",
      }),
    ],
    [],
  );

  assert.deepEqual(
    resolveTaskAvatarIdentity(
      task({ executor: { type: "agent", agentId: "reviewer" } }),
      catalog,
    ),
    {
      agentId: "reviewer",
      avatarDataUrl: "data:image/png;base64,cmV2aWV3ZXI=",
      kind: "agent",
      label: "Reviewer",
      providerIcon: null,
      providerType: "claude",
    },
  );
});
test("task team executor avatar uses team identity", () => {
  const catalog = buildThreadAvatarCatalog(
    [],
    [
      team({
        teamId: "product-team",
        displayName: "Product Team",
        avatarDataUrl: "data:image/png;base64,dGVhbQ==",
      }),
    ],
  );

  assert.deepEqual(
    resolveTaskAvatarIdentity(
      task({ executor: { type: "team", teamId: "product-team" } }),
      catalog,
    ),
    {
      agentId: "product-team",
      avatarDataUrl: "data:image/png;base64,dGVhbQ==",
      kind: "team",
      label: "Product Team",
      providerIcon: null,
      providerType: null,
    },
  );
});
