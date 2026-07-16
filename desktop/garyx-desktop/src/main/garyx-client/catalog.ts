import type {
  CreateSkillInput,
  DeleteMcpServerInput,
  DeleteSlashCommandInput,
  DesktopMcpServer,
  DesktopSettings,
  DesktopSkillEditorState,
  DesktopSkillEntryNode,
  DesktopSkillFileDocument,
  DesktopSkillInfo,
  SlashCommand,
  ToggleMcpServerInput,
  UpdateMcpServerInput,
  UpdateSlashCommandInput,
  UpsertMcpServerInput,
  UpsertSlashCommandInput,
} from "@shared/contracts";
import {
  GatewayContractError,
  requestJson,
  requireContractArray,
  requireContractBoolean,
  requireContractField,
  requireContractRecord,
  requireContractString,
  requireContractNonEmptyString,
} from "./http.ts";

interface SkillPayload {
  id?: string;
  name?: string | null;
  description?: string | null;
  installed?: boolean;
  enabled?: boolean;
  source_path?: string | null;
}

interface SkillsPayload {
  skills?: SkillPayload[];
}

interface SkillEntryPayload {
  path?: string | null;
  name?: string | null;
  entryType?: string | null;
  children?: SkillEntryPayload[] | null;
}

interface SkillEditorPayload {
  skill?: SkillPayload | null;
  entries?: SkillEntryPayload[] | null;
}

interface SkillFileDocumentPayload {
  skill?: SkillPayload | null;
  path?: string | null;
  content?: string | null;
  mediaType?: string | null;
  previewKind?: string | null;
  dataBase64?: string | null;
  editable?: boolean | null;
}

interface SlashCommandPayload {
  name?: string;
  description?: string | null;
  prompt?: string | null;
}

interface SlashCommandsPayload {
  commands?: SlashCommandPayload[];
}

interface McpServerPayload {
  name?: string;
  transport?: string | null;
  command?: string | null;
  args?: unknown;
  env?: unknown;
  enabled?: boolean;
  working_dir?: string | null;
  url?: string | null;
  bearer_token_env?: string | null;
  headers?: unknown;
}

interface McpServersPayload {
  servers?: McpServerPayload[];
}

function mapSkill(value: unknown, path = "skill"): DesktopSkillInfo {
  const record = requireContractRecord(value, path);
  return {
    id: requireContractNonEmptyString(
      requireContractField(record, "id", path),
      `${path}.id`,
    ),
    name: requireContractNonEmptyString(
      requireContractField(record, "name", path),
      `${path}.name`,
    ),
    description: requireContractString(
      requireContractField(record, "description", path),
      `${path}.description`,
    ),
    installed: requireContractBoolean(
      requireContractField(record, "installed", path),
      `${path}.installed`,
    ),
    enabled: requireContractBoolean(
      requireContractField(record, "enabled", path),
      `${path}.enabled`,
    ),
    sourcePath: requireContractNonEmptyString(
      requireContractField(record, "source_path", path),
      `${path}.source_path`,
    ),
  };
}

function mapSkillEntry(value: unknown, path: string): DesktopSkillEntryNode {
  const record = requireContractRecord(value, path);
  const entryType = requireContractString(
    requireContractField(record, "entryType", path),
    `${path}.entryType`,
  );
  if (entryType !== "directory" && entryType !== "file") {
    throw new GatewayContractError(
      `${path}.entryType`,
      "must be directory or file",
    );
  }
  return {
    path: requireContractString(
      requireContractField(record, "path", path),
      `${path}.path`,
    ),
    name: requireContractString(
      requireContractField(record, "name", path),
      `${path}.name`,
    ),
    entryType,
    children: requireContractArray(
      requireContractField(record, "children", path),
      `${path}.children`,
    ).map((child, index) =>
      mapSkillEntry(child, `${path}.children[${index}]`),
    ),
  };
}

function mapSkillEditorState(
  value: unknown,
): DesktopSkillEditorState {
  const path = "skill editor";
  const record = requireContractRecord(value, path);
  return {
    skill: mapSkill(
      requireContractField(record, "skill", path),
      `${path}.skill`,
    ),
    entries: requireContractArray(
      requireContractField(record, "entries", path),
      `${path}.entries`,
    ).map((entry, index) =>
      mapSkillEntry(entry, `${path}.entries[${index}]`),
    ),
  };
}

function normalizeSkillFilePreviewKind(
  value: unknown,
): DesktopSkillFileDocument["previewKind"] {
  switch (value) {
    case "markdown":
    case "text":
    case "image":
    case "unsupported":
      return value;
    default:
      throw new GatewayContractError(
        "skill file.previewKind",
        "must be a current preview kind",
      );
  }
}

function mapSkillFileDocument(
  value: unknown,
): DesktopSkillFileDocument {
  const path = "skill file";
  const record = requireContractRecord(value, path);
  const dataBase64 = requireContractField(record, "dataBase64", path);
  return {
    skill: mapSkill(
      requireContractField(record, "skill", path),
      `${path}.skill`,
    ),
    path: requireContractString(
      requireContractField(record, "path", path),
      `${path}.path`,
    ),
    content: requireContractString(
      requireContractField(record, "content", path),
      `${path}.content`,
    ),
    mediaType: requireContractNonEmptyString(
      requireContractField(record, "mediaType", path),
      `${path}.mediaType`,
    ),
    previewKind: normalizeSkillFilePreviewKind(
      requireContractField(record, "previewKind", path),
    ),
    dataBase64: dataBase64 === null
      ? null
      : requireContractString(dataBase64, `${path}.dataBase64`),
    editable: requireContractBoolean(
      requireContractField(record, "editable", path),
      `${path}.editable`,
    ),
  };
}

function mapSlashCommand(value: unknown, path = "shortcut command"): SlashCommand {
  const record = requireContractRecord(value, path);
  const prompt = requireContractField(record, "prompt", path);
  return {
    name: requireContractNonEmptyString(
      requireContractField(record, "name", path),
      `${path}.name`,
    ),
    description: requireContractString(
      requireContractField(record, "description", path),
      `${path}.description`,
    ),
    prompt: prompt === null
      ? null
      : requireContractString(prompt, `${path}.prompt`),
  };
}

function mapStringRecord(value: unknown, path: string): Record<string, string> {
  const record = requireContractRecord(value, path);
  return Object.fromEntries(
    Object.entries(record).map(([key, entry]) => [
      key,
      requireContractString(entry, `${path}.${key}`),
    ]),
  );
}

function mapMcpServer(value: unknown, index?: number): DesktopMcpServer {
  const path = index === undefined
    ? "MCP server"
    : `MCP server list.servers[${index}]`;
  const record = requireContractRecord(value, path);
  const transport = requireContractString(
    requireContractField(record, "transport", path),
    `${path}.transport`,
  );
  if (transport !== "stdio" && transport !== "streamable_http") {
    throw new GatewayContractError(
      `${path}.transport`,
      "must be stdio or streamable_http",
    );
  }
  const nullableString = (field: string): string | null => {
    const fieldValue = requireContractField(record, field, path);
    return fieldValue === null
      ? null
      : requireContractString(fieldValue, `${path}.${field}`);
  };
  // Present on every response even though the current desktop view does not
  // expose it.
  nullableString("bearer_token_env");
  return {
    name: requireContractNonEmptyString(
      requireContractField(record, "name", path),
      `${path}.name`,
    ),
    transport,
    command: requireContractString(
      requireContractField(record, "command", path),
      `${path}.command`,
    ),
    args: requireContractArray(
      requireContractField(record, "args", path),
      `${path}.args`,
    ).map((entry, entryIndex) =>
      requireContractString(entry, `${path}.args[${entryIndex}]`),
    ),
    env: mapStringRecord(
      requireContractField(record, "env", path),
      `${path}.env`,
    ),
    enabled: requireContractBoolean(
      requireContractField(record, "enabled", path),
      `${path}.enabled`,
    ),
    workingDir: nullableString("working_dir"),
    url: nullableString("url"),
    headers: mapStringRecord(
      requireContractField(record, "headers", path),
      `${path}.headers`,
    ),
  };
}

export async function listSkills(
  settings: DesktopSettings,
): Promise<DesktopSkillInfo[]> {
  const payload = await requestJson<SkillsPayload>(
    settings,
    "/api/skills",
    "readRetryable",
    { signal: AbortSignal.timeout(8000) },
  );

  const record = requireContractRecord(payload, "skill list");
  return requireContractArray(
    requireContractField(record, "skills", "skill list"),
    "skill list.skills",
  ).map((skill, index) => mapSkill(skill, `skill list.skills[${index}]`));
}

export async function createSkill(
  settings: DesktopSettings,
  input: CreateSkillInput,
): Promise<DesktopSkillInfo> {
  const payload = await requestJson<SkillPayload>(
    settings,
    "/api/skills",
    "mutationSingleAttempt",
    {
      method: "POST",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify(input),
    },
  );

  return mapSkill(payload);
}

export async function toggleSkill(
  settings: DesktopSettings,
  skillId: string,
): Promise<DesktopSkillInfo> {
  const payload = await requestJson<SkillPayload>(
    settings,
    `/api/skills/${encodeURIComponent(skillId)}/toggle`,
    "mutationSingleAttempt",
    {
      method: "PATCH",
      signal: AbortSignal.timeout(8000),
    },
  );

  return mapSkill(payload);
}

export async function deleteSkill(
  settings: DesktopSettings,
  skillId: string,
): Promise<void> {
  await requestJson<unknown>(
    settings,
    `/api/skills/${encodeURIComponent(skillId)}`,
    "mutationSingleAttempt",
    {
      method: "DELETE",
      signal: AbortSignal.timeout(8000),
    },
  );
}

export async function getSkillEditor(
  settings: DesktopSettings,
  skillId: string,
): Promise<DesktopSkillEditorState> {
  const payload = await requestJson<SkillEditorPayload>(
    settings,
    `/api/skills/${encodeURIComponent(skillId)}/tree`,
    "readRetryable",
    {
      signal: AbortSignal.timeout(8000),
    },
  );

  return mapSkillEditorState(payload);
}

export async function readSkillFile(
  settings: DesktopSettings,
  skillId: string,
  path: string,
): Promise<DesktopSkillFileDocument> {
  const payload = await requestJson<SkillFileDocumentPayload>(
    settings,
    `/api/skills/${encodeURIComponent(skillId)}/file?path=${encodeURIComponent(path)}`,
    "readRetryable",
    {
      signal: AbortSignal.timeout(8000),
    },
  );

  return mapSkillFileDocument(payload);
}

export async function saveSkillFile(
  settings: DesktopSettings,
  input: { skillId: string; path: string; content: string },
): Promise<DesktopSkillFileDocument> {
  const payload = await requestJson<SkillFileDocumentPayload>(
    settings,
    `/api/skills/${encodeURIComponent(input.skillId)}/file`,
    "mutationSingleAttempt",
    {
      method: "PUT",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        path: input.path,
        content: input.content,
      }),
    },
  );

  return mapSkillFileDocument(payload);
}

export async function createSkillEntry(
  settings: DesktopSettings,
  input: { skillId: string; path: string; entryType: "file" | "directory" },
): Promise<DesktopSkillEditorState> {
  const payload = await requestJson<SkillEditorPayload>(
    settings,
    `/api/skills/${encodeURIComponent(input.skillId)}/entries`,
    "mutationSingleAttempt",
    {
      method: "POST",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        path: input.path,
        entryType: input.entryType,
      }),
    },
  );

  return mapSkillEditorState(payload);
}

export async function deleteSkillEntry(
  settings: DesktopSettings,
  input: { skillId: string; path: string },
): Promise<DesktopSkillEditorState> {
  const payload = await requestJson<SkillEditorPayload>(
    settings,
    `/api/skills/${encodeURIComponent(input.skillId)}/entries?path=${encodeURIComponent(input.path)}`,
    "mutationSingleAttempt",
    {
      method: "DELETE",
      signal: AbortSignal.timeout(8000),
    },
  );

  return mapSkillEditorState(payload);
}

export async function listSlashCommands(
  settings: DesktopSettings,
): Promise<SlashCommand[]> {
  const payload = await requestJson<SlashCommandsPayload>(
    settings,
    "/api/commands/shortcuts",
    "readRetryable",
    {
      signal: AbortSignal.timeout(8000),
    },
  );

  const record = requireContractRecord(payload, "shortcut command list");
  return requireContractArray(
    requireContractField(record, "commands", "shortcut command list"),
    "shortcut command list.commands",
  ).map((command, index) =>
    mapSlashCommand(command, `shortcut command list.commands[${index}]`),
  );
}

export async function createSlashCommand(
  settings: DesktopSettings,
  input: UpsertSlashCommandInput,
): Promise<SlashCommand> {
  const payload = await requestJson<SlashCommandPayload>(
    settings,
    "/api/commands/shortcuts",
    "mutationSingleAttempt",
    {
      method: "POST",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        name: input.name,
        description: input.description,
        prompt: input.prompt || null,
      }),
    },
  );

  return mapSlashCommand(payload);
}

export async function updateSlashCommand(
  settings: DesktopSettings,
  input: UpdateSlashCommandInput,
): Promise<SlashCommand> {
  const payload = await requestJson<SlashCommandPayload>(
    settings,
    `/api/commands/shortcuts/${encodeURIComponent(input.currentName)}`,
    "mutationSingleAttempt",
    {
      method: "PUT",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        name: input.name,
        description: input.description,
        prompt: input.prompt || null,
      }),
    },
  );

  return mapSlashCommand(payload);
}

export async function deleteSlashCommand(
  settings: DesktopSettings,
  input: DeleteSlashCommandInput,
): Promise<void> {
  await requestJson<unknown>(
    settings,
    `/api/commands/shortcuts/${encodeURIComponent(input.name)}`,
    "mutationSingleAttempt",
    {
      method: "DELETE",
      signal: AbortSignal.timeout(8000),
    },
  );
}

export async function listMcpServers(
  settings: DesktopSettings,
): Promise<DesktopMcpServer[]> {
  const payload = await requestJson<McpServersPayload>(
    settings,
    "/api/mcp-servers",
    "readRetryable",
    {
      signal: AbortSignal.timeout(8000),
    },
  );

  const record = requireContractRecord(payload, "MCP server list");
  return requireContractArray(
    requireContractField(record, "servers", "MCP server list"),
    "MCP server list.servers",
  ).map(mapMcpServer);
}

export async function createMcpServer(
  settings: DesktopSettings,
  input: UpsertMcpServerInput,
): Promise<DesktopMcpServer> {
  const payload = await requestJson<McpServerPayload>(
    settings,
    "/api/mcp-servers",
    "mutationSingleAttempt",
    {
      method: "POST",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        name: input.name,
        transport: input.transport,
        command: input.command || "",
        args: input.args || [],
        env: input.env || {},
        enabled: input.enabled,
        working_dir: input.workingDir || null,
        url: input.url || null,
        headers: input.headers || {},
      }),
    },
  );

  return mapMcpServer(payload);
}

export async function updateMcpServer(
  settings: DesktopSettings,
  input: UpdateMcpServerInput,
): Promise<DesktopMcpServer> {
  const payload = await requestJson<McpServerPayload>(
    settings,
    `/api/mcp-servers/${encodeURIComponent(input.currentName)}`,
    "mutationSingleAttempt",
    {
      method: "PUT",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        name: input.name,
        transport: input.transport,
        command: input.command || "",
        args: input.args || [],
        env: input.env || {},
        enabled: input.enabled,
        working_dir: input.workingDir || null,
        url: input.url || null,
        headers: input.headers || {},
      }),
    },
  );

  return mapMcpServer(payload);
}

export async function deleteMcpServer(
  settings: DesktopSettings,
  input: DeleteMcpServerInput,
): Promise<void> {
  await requestJson<unknown>(
    settings,
    `/api/mcp-servers/${encodeURIComponent(input.name)}`,
    "mutationSingleAttempt",
    {
      method: "DELETE",
      signal: AbortSignal.timeout(8000),
    },
  );
}

export async function toggleMcpServer(
  settings: DesktopSettings,
  input: ToggleMcpServerInput,
): Promise<DesktopMcpServer> {
  const payload = await requestJson<McpServerPayload>(
    settings,
    `/api/mcp-servers/${encodeURIComponent(input.name)}/toggle`,
    "mutationSingleAttempt",
    {
      method: "PATCH",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        enabled: input.enabled,
      }),
    },
  );

  return mapMcpServer(payload);
}
