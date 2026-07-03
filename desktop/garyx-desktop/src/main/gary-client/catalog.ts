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
  UpdateSkillInput,
  UpdateSlashCommandInput,
  UpsertMcpServerInput,
  UpsertSlashCommandInput,
} from "@shared/contracts";
import { parseRecord, requestJson } from "./http.ts";

interface SkillPayload {
  id?: string;
  name?: string | null;
  description?: string | null;
  installed?: boolean;
  enabled?: boolean;
  source_path?: string | null;
  sourcePath?: string | null;
}

interface SkillsPayload {
  skills?: SkillPayload[];
}

interface SkillEntryPayload {
  path?: string | null;
  name?: string | null;
  entry_type?: string | null;
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
  media_type?: string | null;
  previewKind?: string | null;
  preview_kind?: string | null;
  dataBase64?: string | null;
  data_base64?: string | null;
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
  workingDir?: string | null;
  url?: string | null;
  bearer_token_env?: string | null;
  bearerTokenEnv?: string | null;
  headers?: unknown;
}

interface McpServersPayload {
  servers?: McpServerPayload[];
}

function mapSkill(value: SkillPayload): DesktopSkillInfo {
  return {
    id: value.id || "",
    name:
      typeof value.name === "string" && value.name.trim()
        ? value.name.trim()
        : value.id || "",
    description:
      typeof value.description === "string" && value.description.trim()
        ? value.description.trim()
        : "",
    installed: value.installed !== false,
    enabled: value.enabled !== false,
    sourcePath:
      (typeof value.source_path === "string" && value.source_path) ||
      (typeof value.sourcePath === "string" && value.sourcePath) ||
      "",
  };
}

function mapSkillEntry(value: SkillEntryPayload): DesktopSkillEntryNode {
  const entryType =
    value.entry_type === "directory" || value.entryType === "directory"
      ? "directory"
      : "file";
  return {
    path: (typeof value.path === "string" && value.path.trim()) || "",
    name: (typeof value.name === "string" && value.name.trim()) || "",
    entryType,
    children: Array.isArray(value.children)
      ? value.children.map(mapSkillEntry)
      : [],
  };
}

function mapSkillEditorState(
  value: SkillEditorPayload,
): DesktopSkillEditorState {
  return {
    skill: mapSkill(value.skill || {}),
    entries: Array.isArray(value.entries)
      ? value.entries.map(mapSkillEntry)
      : [],
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
      return "unsupported";
  }
}

function mapSkillFileDocument(
  value: SkillFileDocumentPayload,
): DesktopSkillFileDocument {
  return {
    skill: mapSkill(value.skill || {}),
    path: typeof value.path === "string" ? value.path : "",
    content: typeof value.content === "string" ? value.content : "",
    mediaType:
      (typeof value.mediaType === "string" && value.mediaType) ||
      (typeof value.media_type === "string" ? value.media_type : "") ||
      "text/plain",
    previewKind: normalizeSkillFilePreviewKind(
      value.previewKind || value.preview_kind,
    ),
    dataBase64:
      typeof value.dataBase64 === "string"
        ? value.dataBase64
        : typeof value.data_base64 === "string"
          ? value.data_base64
          : null,
    editable: value.editable !== false,
  };
}

function mapSlashCommand(value: SlashCommandPayload): SlashCommand {
  return {
    name: value.name || "",
    description:
      typeof value.description === "string" && value.description.trim()
        ? value.description.trim()
        : "",
    prompt:
      typeof value.prompt === "string" && value.prompt.trim()
        ? value.prompt
        : null,
  };
}

function mapMcpServer(value: McpServerPayload): DesktopMcpServer {
  const envRecord = parseRecord(value.env);
  const headersRecord = parseRecord(value.headers);
  const transport =
    value.transport === "streamable_http"
      ? ("streamable_http" as const)
      : ("stdio" as const);
  return {
    name: value.name || "",
    transport,
    command:
      typeof value.command === "string" && value.command.trim()
        ? value.command.trim()
        : "",
    args: Array.isArray(value.args)
      ? value.args.filter((entry): entry is string => typeof entry === "string")
      : [],
    env: Object.fromEntries(
      Object.entries(envRecord).flatMap(([key, entryValue]) => {
        return typeof entryValue === "string" ? [[key, entryValue]] : [];
      }),
    ),
    enabled: value.enabled !== false,
    workingDir:
      (typeof value.working_dir === "string" && value.working_dir.trim()) ||
      (typeof value.workingDir === "string" && value.workingDir.trim()) ||
      null,
    url:
      typeof value.url === "string" && value.url.trim()
        ? value.url.trim()
        : null,
    headers: Object.fromEntries(
      Object.entries(headersRecord).flatMap(([key, entryValue]) => {
        return typeof entryValue === "string" ? [[key, entryValue]] : [];
      }),
    ),
  };
}

export async function listSkills(
  settings: DesktopSettings,
): Promise<DesktopSkillInfo[]> {
  const payload = await requestJson<SkillsPayload>(settings, "/api/skills", {
    signal: AbortSignal.timeout(8000),
  });

  return Array.isArray(payload.skills) ? payload.skills.map(mapSkill) : [];
}

export async function createSkill(
  settings: DesktopSettings,
  input: CreateSkillInput,
): Promise<DesktopSkillInfo> {
  const payload = await requestJson<SkillPayload>(settings, "/api/skills", {
    method: "POST",
    signal: AbortSignal.timeout(8000),
    body: JSON.stringify(input),
  });

  return mapSkill(payload);
}

export async function updateSkill(
  settings: DesktopSettings,
  input: UpdateSkillInput,
): Promise<DesktopSkillInfo> {
  const payload = await requestJson<SkillPayload>(
    settings,
    `/api/skills/${encodeURIComponent(input.skillId)}`,
    {
      method: "PATCH",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        name: input.name,
        description: input.description,
      }),
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
    {
      signal: AbortSignal.timeout(8000),
    },
  );

  return Array.isArray(payload.commands)
    ? payload.commands.map(mapSlashCommand)
    : [];
}

export async function createSlashCommand(
  settings: DesktopSettings,
  input: UpsertSlashCommandInput,
): Promise<SlashCommand> {
  const payload = await requestJson<SlashCommandPayload>(
    settings,
    "/api/commands/shortcuts",
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
    {
      signal: AbortSignal.timeout(8000),
    },
  );

  return Array.isArray(payload.servers)
    ? payload.servers.map(mapMcpServer)
    : [];
}

export async function createMcpServer(
  settings: DesktopSettings,
  input: UpsertMcpServerInput,
): Promise<DesktopMcpServer> {
  const payload = await requestJson<McpServerPayload>(
    settings,
    "/api/mcp-servers",
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
