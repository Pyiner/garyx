export interface DesktopSkillInfo {
  id: string;
  name: string;
  description: string;
  installed: boolean;
  enabled: boolean;
  sourcePath: string;
}

export interface DesktopSkillEntryNode {
  path: string;
  name: string;
  entryType: "file" | "directory";
  children: DesktopSkillEntryNode[];
}

export interface DesktopSkillEditorState {
  skill: DesktopSkillInfo;
  entries: DesktopSkillEntryNode[];
}

export type DesktopSkillFilePreviewKind =
  | "markdown"
  | "text"
  | "image"
  | "unsupported";

export interface DesktopSkillFileDocument {
  skill: DesktopSkillInfo;
  path: string;
  content: string;
  mediaType: string;
  previewKind: DesktopSkillFilePreviewKind;
  dataBase64?: string | null;
  editable: boolean;
}

export type DesktopMemoryDocumentScope = "agent" | "automation";

export interface DesktopMemoryDocument {
  scope: DesktopMemoryDocumentScope;
  agentId?: string | null;
  automationId?: string | null;
  path: string;
  content: string;
  exists: boolean;
  modifiedAt?: string | null;
}

export interface ReadMemoryDocumentInput {
  scope: DesktopMemoryDocumentScope;
  agentId?: string;
  automationId?: string;
}

export interface SaveMemoryDocumentInput extends ReadMemoryDocumentInput {
  content: string;
}

export interface SlashCommand {
  name: string;
  description: string;
  prompt?: string | null;
}

export interface UpsertSlashCommandInput {
  name: string;
  description: string;
  prompt?: string | null;
}

export interface UpdateSlashCommandInput extends UpsertSlashCommandInput {
  currentName: string;
}

export interface DeleteSlashCommandInput {
  name: string;
}

export type McpTransportType = "stdio" | "streamable_http";

export interface DesktopMcpServer {
  name: string;
  transport: McpTransportType;
  // STDIO fields
  command: string;
  args: string[];
  env: Record<string, string>;
  workingDir?: string | null;
  // Streamable HTTP fields
  url?: string | null;
  headers?: Record<string, string>;
  // Common
  enabled: boolean;
}

export interface UpsertMcpServerInput {
  name: string;
  transport: McpTransportType;
  // STDIO fields
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  workingDir?: string | null;
  // Streamable HTTP fields
  url?: string | null;
  headers?: Record<string, string>;
  // Common
  enabled: boolean;
}

export interface UpdateMcpServerInput extends UpsertMcpServerInput {
  currentName: string;
}

export interface DeleteMcpServerInput {
  name: string;
}

export interface ToggleMcpServerInput {
  name: string;
  enabled: boolean;
}

export interface CreateSkillInput {
  id: string;
  name: string;
  description: string;
  body: string;
}

export interface UpdateSkillInput {
  skillId: string;
  name: string;
  description: string;
}

export interface ToggleSkillInput {
  skillId: string;
}

export interface DeleteSkillInput {
  skillId: string;
}

export interface GetSkillEditorInput {
  skillId: string;
}

export interface ReadSkillFileInput {
  skillId: string;
  path: string;
}

export interface SaveSkillFileInput {
  skillId: string;
  path: string;
  content: string;
}

export interface CreateSkillEntryInput {
  skillId: string;
  path: string;
  entryType: "file" | "directory";
}

export interface DeleteSkillEntryInput {
  skillId: string;
  path: string;
}
