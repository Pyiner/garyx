import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  cancelClaudeCodeAuth,
  fetchAutomationActivity,
  fetchBotConsoles,
  fetchConfiguredBots,
  fetchThreadLogs,
  fetchThreadPins,
  fetchWorkspaces,
  deleteClaudeCodeAccount,
  getCodingUsage,
  getSkillEditor,
  interruptThread,
  listCapsules,
  listClaudeCodeAccounts,
  listCustomAgents,
  listMcpServers,
  listProviderRecentSessions,
  listSkills,
  listSlashCommands,
  listTasks,
  listWorkspaceFiles,
  openChatStream,
  reorderRemoteThreadPins,
  retryThreadQuotaRecovery,
  saveGatewaySettings,
  sendStreamingInput,
  setDefaultCustomAgent,
  setGatewayFetch,
  selectClaudeCodeAccount,
  startClaudeCodeAuth,
  toggleCustomAgent,
} from "./gary-client.ts";

const settings = {
  gatewayUrl: "https://gateway.example.test",
  gatewayAuthToken: "",
};

function jsonResponse(payload, status = 200) {
  return new Response(JSON.stringify(payload), {
    status,
    statusText: status === 200 ? "OK" : "Created",
    headers: { "content-type": "application/json" },
  });
}

async function withGatewayFetch(fetchImpl, run) {
  setGatewayFetch(fetchImpl);
  try {
    return await run();
  } finally {
    setGatewayFetch(null);
  }
}

function canonicalTaskSummary(overrides = {}) {
  return {
    thread_id: "thread::task-synthetic",
    task_id: "#TASK-100",
    number: 100,
    title: "Synthetic task",
    status: "in_progress",
    creator: { kind: "human", user_id: "1000000001" },
    updated_at: "2026-01-02T00:00:00Z",
    updated_by: { kind: "agent", agent_id: "test-agent" },
    runtime_agent_id: "test-agent",
    reply_count: 2,
    ...overrides,
  };
}

function canonicalAgent(overrides = {}) {
  return {
    agent_id: "test-agent",
    display_name: "Test Agent",
    provider_type: "claude_code",
    model: "test-model",
    model_reasoning_effort: "high",
    model_service_tier: "standard",
    provider_env: {},
    default_workspace_dir: "/Users/test/project",
    avatar_data_url: null,
    provider_icon: null,
    system_prompt: "Synthetic prompt",
    built_in: false,
    standalone: false,
    enabled: true,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-02T00:00:00Z",
    ...overrides,
  };
}

function canonicalSkill(overrides = {}) {
  return {
    id: "test-skill",
    name: "Test Skill",
    description: "Synthetic skill",
    installed: true,
    enabled: true,
    source_path: "/Users/test/.garyx/skills/test-skill",
    ...overrides,
  };
}

function canonicalCapsule(overrides = {}) {
  return {
    id: "capsule-test",
    title: "Test capsule",
    description: "Synthetic capsule",
    thread_id: null,
    run_id: null,
    agent_id: null,
    provider_type: null,
    html_sha256: "a".repeat(64),
    byte_size: 128,
    revision: 1,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-02T00:00:00Z",
    favorited_at: null,
    ...overrides,
  };
}

test("task DTOs accept only the canonical snake_case principal shape", async () => {
  await withGatewayFetch(
    async () =>
      jsonResponse({
        tasks: [canonicalTaskSummary()],
        total: 1,
        has_more: false,
      }),
    async () => {
      const page = await listTasks(settings);
      assert.equal(page.tasks[0].creator.userId, "1000000001");
    },
  );

  await withGatewayFetch(
    async () =>
      jsonResponse({
        tasks: [
          canonicalTaskSummary({
            creator: { kind: "human", userId: "1000000001" },
          }),
        ],
        total: 1,
        has_more: false,
      }),
    () =>
      assert.rejects(
        () => listTasks(settings),
        /task list\.tasks\[0\]\.creator\.user_id is required/,
      ),
  );
});

test("automation activity replays canonical camelCase and maps failed_dropped", async () => {
  const canonical = {
    items: [
      {
        runId: "run::automation-test",
        status: "failed_dropped",
        startedAt: "2026-01-02T03:04:05Z",
        finishedAt: "2026-01-02T03:05:05Z",
        durationMs: 60_000,
        excerpt: "Synthetic failure",
        threadId: "thread::automation-test",
      },
    ],
    threadId: "thread::automation-test",
    count: 1,
  };
  await withGatewayFetch(async () => jsonResponse(canonical), async () => {
    const feed = await fetchAutomationActivity(settings, "automation-test");
    assert.equal(feed.items[0].runId, "run::automation-test");
    assert.equal(feed.items[0].startedAt, "2026-01-02T03:04:05Z");
    assert.equal(feed.items[0].durationMs, 60_000);
    assert.equal(feed.items[0].threadId, "thread::automation-test");
    assert.equal(feed.items[0].status, "failed");
  });

  const wrongCase = {
    items: [
      {
        run_id: "run::automation-test",
        status: "success",
        started_at: "2026-01-02T03:04:05Z",
        thread_id: "thread::automation-test",
      },
    ],
    threadId: "thread::automation-test",
    count: 1,
  };
  await withGatewayFetch(async () => jsonResponse(wrongCase), () =>
    assert.rejects(
      () => fetchAutomationActivity(settings, "automation-test"),
      /automation activity\.items\[0\]\.runId is required/,
    ),
  );
});

test("workspace roots stay snake_case while file listings stay camelCase", async () => {
  await withGatewayFetch(
    async (url) => {
      const path = new URL(String(url)).pathname;
      if (path === "/api/workspaces") {
        return jsonResponse({
          workspace_state_initialized: true,
          gateway_home: "/Users/test",
          workspaces: [
            {
              name: "Test workspace",
              path: "/Users/test/project",
              pinned: false,
              thread_count: 3,
              last_activity_at: "2026-07-20T00:00:00Z",
              git_repo: true,
            },
          ],
        });
      }
      return jsonResponse({
        workspaceDir: "/Users/test/project",
        directoryPath: "src",
        entries: [
          {
            path: "src/main.ts",
            name: "main.ts",
            entryType: "file",
            size: null,
            modifiedAt: null,
            mediaType: null,
            hasChildren: false,
          },
        ],
      });
    },
    async () => {
      const catalog = await fetchWorkspaces(settings);
      assert.equal(catalog.gatewayHome, "/Users/test");
      assert.equal(catalog.workspaces[0].path, "/Users/test/project");
      assert.equal(catalog.workspaces[0].name, "Test workspace");
      assert.equal(catalog.workspaces[0].pinned, false);
      assert.equal(catalog.workspaces[0].threadCount, 3);
      assert.equal(catalog.workspaces[0].lastActivityAt, "2026-07-20T00:00:00Z");
      assert.equal(catalog.workspaces[0].gitRepo, true);
      const files = await listWorkspaceFiles(settings, {
        workspacePath: "/Users/test/project",
        directoryPath: "src",
      });
      assert.equal(files.entries[0].name, "main.ts");
      assert.equal(files.entries[0].size, null);
    },
  );

  await withGatewayFetch(
    async () =>
      jsonResponse({
        workspace_dir: "/Users/test/project",
        directory_path: "src",
        entries: [],
      }),
    () =>
      assert.rejects(
        () =>
          listWorkspaceFiles(settings, {
            workspacePath: "/Users/test/project",
            directoryPath: "src",
          }),
        /workspace file listing\.workspaceDir is required/,
      ),
  );
});

test("capsules require snake_case nullable keys instead of camel aliases", async () => {
  await withGatewayFetch(
    async () => jsonResponse({ capsules: [canonicalCapsule()] }),
    async () => {
      const page = await listCapsules(settings);
      assert.equal(page.capsules[0].threadId, null);
    },
  );
  const wrongCase = canonicalCapsule();
  delete wrongCase.thread_id;
  wrongCase.threadId = null;
  await withGatewayFetch(
    async () => jsonResponse({ capsules: [wrongCase] }),
    () =>
      assert.rejects(
        () => listCapsules(settings),
        /capsule list\.capsules\[0\]\.thread_id is required/,
      ),
  );
  const wrongFavoriteCase = canonicalCapsule();
  delete wrongFavoriteCase.favorited_at;
  wrongFavoriteCase.favoritedAt = null;
  await withGatewayFetch(
    async () => jsonResponse({ capsules: [wrongFavoriteCase] }),
    () =>
      assert.rejects(
        () => listCapsules(settings),
        /capsule list\.capsules\[0\]\.favorited_at is required/,
      ),
  );
});

test("custom agents require snake_case names and present provider_icon", async () => {
  await withGatewayFetch(
    async () => jsonResponse({
      agents: [
        canonicalAgent(),
        canonicalAgent({
          agent_id: "grok",
          display_name: "Grok",
          provider_type: "grok_build",
          provider_icon: {
            key: "grok",
            provider_type: "grok_build",
            label: "Grok",
          },
        }),
      ],
      default_agent_id: null,
      effective_default_agent_id: "test-agent",
    }),
    async () => {
      const catalog = await listCustomAgents(settings);
      assert.equal(catalog.agents[0].displayName, "Test Agent");
      assert.equal(catalog.agents[0].providerIcon, null);
      assert.deepEqual(catalog.agents[1].providerIcon, {
        key: "grok",
        providerType: "grok_build",
        label: "Grok",
      });
      assert.equal(catalog.defaultAgentId, null);
      assert.equal(catalog.effectiveDefaultAgentId, "test-agent");
    },
  );
  const wrongCase = canonicalAgent();
  delete wrongCase.display_name;
  wrongCase.displayName = "Test Agent";
  await withGatewayFetch(
    async () => jsonResponse({
      agents: [wrongCase],
      default_agent_id: null,
      effective_default_agent_id: null,
    }),
    () =>
      assert.rejects(
        () => listCustomAgents(settings),
        /custom agent list\.agents\[0\]\.display_name is required/,
      ),
  );

  const missingAvatar = canonicalAgent();
  delete missingAvatar.avatar_data_url;
  await withGatewayFetch(
    async () => jsonResponse({
      agents: [missingAvatar],
      default_agent_id: null,
      effective_default_agent_id: null,
    }),
    () =>
      assert.rejects(
        () => listCustomAgents(settings),
        /custom agent list\.agents\[0\]\.avatar_data_url is required/,
      ),
  );
});

test("custom-agent availability mutations use the typed PATCH endpoints", async () => {
  const requests = [];
  await withGatewayFetch(
    async (url, init) => {
      requests.push({
        path: new URL(String(url)).pathname,
        method: init?.method,
        body: init?.body ? JSON.parse(String(init.body)) : null,
      });
      return jsonResponse(canonicalAgent({
        agent_id: "codex",
        enabled: requests.length > 1,
        standalone: true,
      }));
    },
    async () => {
      const toggled = await toggleCustomAgent(settings, {
        agentId: "codex",
        enabled: false,
      });
      const selected = await setDefaultCustomAgent(settings, { agentId: "codex" });
      assert.equal(toggled.enabled, false);
      assert.equal(selected.enabled, true);
    },
  );
  assert.deepEqual(requests, [
    {
      path: "/api/custom-agents/codex/toggle",
      method: "PATCH",
      body: { enabled: false },
    },
    {
      path: "/api/custom-agents/codex/default",
      method: "PATCH",
      body: null,
    },
  ]);
});

test("catalog endpoints replay their distinct current casing contracts", async () => {
  await withGatewayFetch(
    async (url) => {
      const path = new URL(String(url)).pathname;
      if (path === "/api/skills") {
        return jsonResponse({ skills: [canonicalSkill()] });
      }
      if (path.endsWith("/tree")) {
        return jsonResponse({
          skill: canonicalSkill(),
          entries: [
            {
              path: "references",
              name: "references",
              entryType: "directory",
              children: [],
            },
          ],
        });
      }
      if (path === "/api/commands/shortcuts") {
        return jsonResponse({
          commands: [
            { name: "test-command", description: "Synthetic", prompt: null },
          ],
        });
      }
      return jsonResponse({
        servers: [
          {
            name: "test-mcp",
            transport: "stdio",
            command: "test-command",
            args: [],
            env: {},
            enabled: true,
            working_dir: null,
            url: null,
            bearer_token_env: null,
            headers: {},
          },
        ],
      });
    },
    async () => {
      assert.equal((await listSkills(settings))[0].sourcePath.includes("test-skill"), true);
      assert.equal((await getSkillEditor(settings, "test-skill")).entries[0].children.length, 0);
      assert.equal((await listSlashCommands(settings))[0].prompt, null);
      assert.equal((await listMcpServers(settings))[0].workingDir, null);
    },
  );

  const wrongCase = canonicalSkill();
  delete wrongCase.source_path;
  wrongCase.sourcePath = "/Users/test/.garyx/skills/test-skill";
  await withGatewayFetch(
    async () => jsonResponse({ skills: [wrongCase] }),
    () =>
      assert.rejects(
        () => listSkills(settings),
        /skill list\.skills\[0\]\.source_path is required/,
      ),
  );
});

test("recent provider sessions accept only their canonical camelCase shape", async () => {
  const canonical = {
    sessions: [
      {
        providerType: "codex_app_server",
        providerHint: "codex",
        sessionId: "session-test",
        title: "Synthetic session",
        workspaceDir: "/Users/test/project",
        updatedAt: "2026-01-02T00:00:00Z",
        path: "/Users/test/.codex/sessions/session-test.jsonl",
      },
    ],
  };
  await withGatewayFetch(async () => jsonResponse(canonical), async () => {
    const sessions = await listProviderRecentSessions(settings);
    assert.equal(sessions[0].sessionId, "session-test");
  });

  await withGatewayFetch(
    async () =>
      jsonResponse({
        sessions: [
          {
            provider_type: "codex_app_server",
            provider_hint: "codex",
            session_id: "session-test",
            title: "Synthetic session",
            workspace_dir: "/Users/test/project",
            updated_at: "2026-01-02T00:00:00Z",
            path: "/Users/test/.codex/sessions/session-test.jsonl",
          },
        ],
      }),
    () =>
      assert.rejects(
        () => listProviderRecentSessions(settings),
        /recent provider sessions\.sessions\[0\]\.providerHint is required/,
      ),
  );
});

test("coding usage accepts the signed i64 reset interval emitted by Rust", async () => {
  await withGatewayFetch(
    async () =>
      jsonResponse({
        providers: [
          {
            id: "codex",
            name: "Codex",
            available: true,
            stale: true,
            error: "temporarily throttled",
            error_code: "rate_limited",
            retry_after_seconds: 90,
            weekly: {
              used_percent: 100,
              remaining_percent: 0,
              reset_after_seconds: -1,
            },
            scoped_limits: [{
              id: "weekly_scoped:Fable",
              name: "Fable",
              kind: "weekly_scoped",
              window: {
                used_percent: 82,
                remaining_percent: 18,
                resets_at: "2026-01-03T00:00:00Z",
                reset_after_seconds: 86400,
              },
            }],
          },
        ],
        refreshed_at: "2026-01-02T00:00:00Z",
      }),
    async () => {
      const usage = await getCodingUsage(settings);
      assert.equal(usage.providers[0].weekly.resetAfterSeconds, -1);
      assert.equal(usage.providers[0].errorCode, "rate_limited");
      assert.equal(usage.providers[0].retryAfterSeconds, 90);
      assert.deepEqual(usage.providers[0].scopedLimits, [{
        id: "weekly_scoped:Fable",
        name: "Fable",
        kind: "weekly_scoped",
        window: {
          usedPercent: 82,
          remainingPercent: 18,
          resetsAt: "2026-01-03T00:00:00Z",
          resetAfterSeconds: 86400,
        },
      }]);
    },
  );
});

test("Claude account list keeps per-account Fable quota and auth session fields", async () => {
  await withGatewayFetch(
    async (url, init) => {
      const path = new URL(String(url)).pathname;
      if (path.endsWith("/auth/start")) {
        assert.deepEqual(JSON.parse(String(init?.body)), {
          mode: "claudeai",
          sso: false,
          email: null,
          managed_account_name: "Work",
          account_id: null,
        });
        return jsonResponse({
          login_id: "login-1",
          account_id: "account-1",
          status: "waiting_for_code",
          url: "https://claude.ai/oauth/authorize?state=test",
          auth_status: null,
          error: null,
          exit_code: null,
        }, 201);
      }
      if (path.endsWith("/auth/login-1")) {
        assert.equal(init?.method, "DELETE");
        return jsonResponse({
          login_id: "login-1",
          account_id: "account-1",
          status: "failed",
          url: "https://claude.ai/oauth/authorize?state=test",
          auth_status: null,
          error: "Claude Code sign-in was cancelled.",
          exit_code: null,
        });
      }
      if (path.endsWith("/accounts/account-1")) {
        assert.equal(init?.method, "DELETE");
        return jsonResponse({ deleted_account_id: "account-1" });
      }
      if (path.endsWith("/accounts/active")) {
        assert.equal(init?.method, "PUT");
        return jsonResponse({
          active_account_id: "account-1",
          selection_changed: true,
          recovery: {
            matched_threads: 3,
            expedited_threads: 2,
            already_claimed_threads: 1,
          },
        });
      }
      if (path.endsWith("/threads/thread%3A%3Aquota/quota-recovery/retry")) {
        assert.equal(init?.method, "POST");
        return jsonResponse({ status: "accepted" }, 202);
      }
      return jsonResponse({
        active_account_id: "account-1",
        refreshed_at: "2026-07-21T12:00:00Z",
        accounts: [{
          id: "account-1",
          name: "Work",
          system_default: false,
          selected: true,
          email: "user@example.com",
          plan: "max",
          usage: {
            id: "claude_code",
            name: "Claude Code",
            available: true,
            session: {
              used_percent: 12,
              remaining_percent: 88,
              resets_at: "2030-01-01T05:00:00Z",
              reset_after_seconds: 18000,
            },
            weekly: {
              used_percent: 23,
              remaining_percent: 77,
              resets_at: "2030-01-07T11:00:00Z",
              reset_after_seconds: 558000,
            },
            scoped_limits: [{
              id: "weekly_scoped:fable",
              name: "Fable",
              kind: "weekly_scoped",
              window: {
                used_percent: 25,
                remaining_percent: 75,
                resets_at: "2030-01-07T11:00:00Z",
                reset_after_seconds: 558000,
              },
            }],
          },
        }],
      });
    },
    async () => {
      const accounts = await listClaudeCodeAccounts(settings);
      assert.equal(accounts.activeAccountId, "account-1");
      assert.equal(accounts.accounts[0].usage.session.resetsAt, "2030-01-01T05:00:00Z");
      assert.equal(accounts.accounts[0].usage.session.resetAfterSeconds, 18000);
      assert.equal(accounts.accounts[0].usage.weekly.resetsAt, "2030-01-07T11:00:00Z");
      assert.equal(accounts.accounts[0].usage.scopedLimits[0].name, "Fable");
      assert.equal(accounts.accounts[0].usage.scopedLimits[0].window.remainingPercent, 75);
      assert.equal(
        accounts.accounts[0].usage.scopedLimits[0].window.resetsAt,
        "2030-01-07T11:00:00Z",
      );
      assert.equal(
        accounts.accounts[0].usage.scopedLimits[0].window.resetAfterSeconds,
        558000,
      );
      const selection = await selectClaudeCodeAccount(settings, "account-1");
      assert.equal(selection.selectionChanged, true);
      assert.deepEqual(selection.recovery, {
        matchedThreads: 3,
        expeditedThreads: 2,
        alreadyClaimedThreads: 1,
      });
      assert.deepEqual(await retryThreadQuotaRecovery(settings, "thread::quota"), {
        status: "accepted",
      });
      const auth = await startClaudeCodeAuth(settings, { managedAccountName: "Work" });
      assert.equal(auth.loginId, "login-1");
      assert.equal(auth.authorizationUrl, "https://claude.ai/oauth/authorize?state=test");
      const cancelled = await cancelClaudeCodeAuth(settings, auth.loginId);
      assert.equal(cancelled.status, "failed");
      assert.equal(cancelled.error, "Claude Code sign-in was cancelled.");
      await deleteClaudeCodeAccount(settings, "account-1");
    },
  );
});

test("quota recovery retry distinguishes a settled generation from an old gateway", async () => {
  await withGatewayFetch(
    async () => jsonResponse({ error: "quota_recovery_not_found" }, 404),
    async () => {
      assert.deepEqual(await retryThreadQuotaRecovery(settings, "thread::settled"), {
        status: "settled",
      });
    },
  );

  await withGatewayFetch(
    async () => jsonResponse({ error: "not_found" }, 404),
    async () => {
      assert.deepEqual(await retryThreadQuotaRecovery(settings, "thread::old-gateway"), {
        status: "unsupported",
      });
    },
  );
});

test("thread log chunks stay camelCase and thread pins stay snake_case", async () => {
  await withGatewayFetch(
    async (url) => {
      const path = new URL(String(url)).pathname;
      return path === "/api/thread-pins"
        ? jsonResponse({ thread_ids: ["thread::pinned-test"], pins: [], revision: 7 })
        : jsonResponse({
            threadId: "thread::log-test",
            path: "/Users/test/.garyx/logs/thread-test.log",
            text: "Synthetic log line",
            cursor: 18,
            reset: true,
          });
    },
    async () => {
      assert.equal((await fetchThreadLogs(settings, "thread::log-test")).cursor, 18);
      assert.deepEqual(await fetchThreadPins(settings), {
        threadIds: ["thread::pinned-test"],
        revision: 7,
      });
    },
  );

  await withGatewayFetch(
    async () =>
      jsonResponse({
        thread_id: "thread::log-test",
        path: "/Users/test/.garyx/logs/thread-test.log",
        text: "Synthetic log line",
        cursor: 18,
        reset: true,
      }),
    () =>
      assert.rejects(
        () => fetchThreadLogs(settings, "thread::log-test"),
        /thread log chunk\.threadId is required/,
      ),
  );
});

test("thread pin reorder sends the revision CAS body and returns 409 pages", async () => {
  const requests = [];
  await withGatewayFetch(
    async (url, init) => {
      requests.push({
        path: new URL(String(url)).pathname,
        method: init?.method,
        body: JSON.parse(String(init?.body)),
      });
      return jsonResponse(
        {
          thread_ids: ["thread::b", "thread::a"],
          pins: [],
          revision: 12,
        },
        409,
      );
    },
    async () => {
      const result = await reorderRemoteThreadPins(
        settings,
        ["thread::b", "thread::a"],
        11,
      );
      assert.deepEqual(result, {
        kind: "conflict",
        page: {
          threadIds: ["thread::b", "thread::a"],
          revision: 12,
        },
      });
      assert.deepEqual(requests, [
        {
          path: "/api/thread-pins",
          method: "PUT",
          body: {
            thread_ids: ["thread::b", "thread::a"],
            expected_revision: 11,
          },
        },
      ]);
    },
  );
});

test("chat HTTP responses use canonical camelCase and expose schema drift", async () => {
  await withGatewayFetch(
    async (url) => {
      const path = new URL(String(url)).pathname;
      if (path === "/api/chat/start") {
        return jsonResponse({
          status: "accepted",
          runId: "run::chat-test",
          threadId: "thread::chat-test",
        });
      }
      if (path === "/api/chat/stream-input") {
        return jsonResponse({
          status: "queued",
          threadStatus: "queued",
          clientIntentId: "intent-test",
          pendingInputId: "pending-test",
          threadId: "thread::chat-test",
        });
      }
      return jsonResponse({
        status: "interrupted",
        threadId: "thread::chat-test",
        abortedRuns: ["run::chat-test"],
      });
    },
    async () => {
      const input = {
        threadId: "thread::chat-test",
        clientIntentId: "intent-test",
        message: "Synthetic message",
      };
      const opened = await openChatStream(settings, input);
      assert.equal(opened.runId, "run::chat-test");
      assert.equal(Object.hasOwn(opened, "sessionId"), false);
      const queued = await sendStreamingInput(settings, input);
      assert.equal(queued.pendingInputId, "pending-test");
      assert.equal(Object.hasOwn(queued, "sessionId"), false);
      const interrupted = await interruptThread(settings, "thread::chat-test");
      assert.deepEqual(interrupted.abortedRuns, ["run::chat-test"]);
      assert.equal(Object.hasOwn(interrupted, "sessionId"), false);
    },
  );

  await withGatewayFetch(
    async () =>
      jsonResponse({
        status: "queued",
        thread_status: "queued",
        client_intent_id: "intent-test",
        pending_input_id: "pending-test",
        thread_id: "thread::chat-test",
      }),
    () =>
      assert.rejects(
        () =>
          sendStreamingInput(settings, {
            threadId: "thread::chat-test",
            clientIntentId: "intent-test",
            message: "Synthetic message",
          }),
        /chat stream-input response\.threadId is required/,
      ),
  );
});

test("configured bots expose only display_name and retired migration code stays deleted", async () => {
  await withGatewayFetch(
    async () =>
      jsonResponse({
        bots: [
          {
            channel: "test-channel",
            account_id: "test-account",
            display_name: "Test Bot",
            enabled: true,
          },
        ],
      }),
    async () => {
      const bots = await fetchConfiguredBots(settings);
      assert.equal(bots[0].display_name, "Test Bot");
      assert.equal(Object.hasOwn(bots[0], "displayName"), false);
    },
  );

  const [botsSource, storeSource] = await Promise.all([
    readFile(new URL("./garyx-client/bots.ts", import.meta.url), "utf8"),
    readFile(new URL("./store.ts", import.meta.url), "utf8"),
  ]);
  assert.doesNotMatch(botsSource, /displayName\??:/);
  assert.doesNotMatch(botsSource, /\bname\??:/);
  assert.doesNotMatch(storeSource, /bot\.displayName/);
  assert.doesNotMatch(storeSource, /bot\.name/);
  assert.match(storeSource, /bot\.display_name\.trim\(\)/);
  assert.doesNotMatch(storeSource, /LEGACY_STATE_FILE_NAME/);
  assert.doesNotMatch(storeSource, /migrateLegacyStateFile/);
});

test("bot console mapping preserves configured and effective agent identities", async () => {
  await withGatewayFetch(
    async () => jsonResponse({
      bots: [{
        id: "test-channel:test-account",
        channel: "test-channel",
        account_id: "test-account",
        title: "Test Bot",
        agent_id: null,
        effective_agent_id: "codex",
        endpoints: [],
        conversation_nodes: [],
      }],
    }),
    async () => {
      const [bot] = await fetchBotConsoles(settings);
      assert.equal(bot.agentId, null);
      assert.equal(bot.effectiveAgentId, "codex");
    },
  );
});

test("gateway settings serializer preserves explicit Claude and omits follow-global null", async () => {
  let savedConfig = null;
  await withGatewayFetch(
    async (url, init) => {
      const path = new URL(String(url)).pathname;
      if (path === "/api/settings" && init?.method === "PUT") {
        savedConfig = JSON.parse(String(init.body));
        return jsonResponse({ ok: true });
      }
      return jsonResponse({ config: savedConfig || {} });
    },
    () => saveGatewaySettings(settings, {
      channels: {
        telegram: {
          accounts: {
            explicit: { agent_id: "claude" },
            inherited: { agent_id: null },
          },
        },
      },
    }),
  );
  assert.equal(
    savedConfig.channels.telegram.accounts.explicit.agent_id,
    "claude",
  );
  assert.equal(
    Object.hasOwn(savedConfig.channels.telegram.accounts.inherited, "agent_id"),
    false,
  );
});

test("source guard rejects the retired DTO alias lookup sites", async () => {
  const files = Object.fromEntries(
    await Promise.all(
      [
        "tasks.ts",
        "threads.ts",
        "stream.ts",
        "provider.ts",
        "automations.ts",
        "workspaces.ts",
        "capsules.ts",
        "agents.ts",
        "catalog.ts",
      ].map(async (name) => [
        name,
        await readFile(new URL(`./garyx-client/${name}`, import.meta.url), "utf8"),
      ]),
    ),
  );

  assert.doesNotMatch(files["tasks.ts"], /record\.(userId|agentId|threadId|taskId)/);
  assert.doesNotMatch(files["threads.ts"], /payload\.(sessions|hasMore|threadIds)/);
  assert.doesNotMatch(files["threads.ts"], /record\.(sourceBranch|worktreeDir|sourceWorkspaceDir|sourceRepoRoot)/);
  assert.doesNotMatch(files["stream.ts"], /payload\.(renderState|renderDelta|sessionId)/);
  assert.doesNotMatch(files["provider.ts"], /record\.(provider_type.*providerType|providerType.*provider_type)/);
  assert.doesNotMatch(files["automations.ts"], /record\.(run_id|started_at|thread_id)/);
  assert.doesNotMatch(files["workspaces.ts"], /record\.(workspaceDir.*workspace_dir|workspace_dir.*workspaceDir)/);
  assert.doesNotMatch(files["capsules.ts"], /record\.(threadId|runId|agentId|providerType)/);
  assert.doesNotMatch(files["agents.ts"], /record\.(agentId|displayName|providerType)/);
  assert.doesNotMatch(files["catalog.ts"], /record\.(sourcePath|entry_type|data_base64)/);
});
