import { useEffect, useMemo, useRef, useState } from 'react';
import {
  IconBolt,
  IconCheck,
  IconDatabase,
  IconPlus,
  IconRobot,
  IconSearch,
  IconSparkles,
  IconUpload,
  IconUsersGroup,
  IconX,
} from '@tabler/icons-react';

import type {
  CreateCustomAgentInput,
  CreateTeamInput,
  DesktopCustomAgent,
  DesktopProviderIconDescriptor,
  DesktopProviderModels,
  DesktopTeam,
  DesktopWorkflowDefinition,
  DesktopWorkflowSourceDocument,
  DesktopWorkspace,
  UpdateCustomAgentInput,
  UpdateTeamInput,
} from '@shared/contracts';

import { Badge } from '../../components/ui/badge';
import { Button } from '../../components/ui/button';
import { Checkbox } from '../../components/ui/checkbox';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '../../components/ui/table';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../components/ui/dialog';
import { Input } from '../../components/ui/input';
import { Label } from '../../components/ui/label';
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../../components/ui/select';
import { Textarea } from '../../components/ui/textarea';
import { WorkspacePathPicker } from '../../components/WorkspacePathPicker';
import { useI18n } from '../../i18n';
import { ProviderAgentIcon, hasProviderAgentIcon } from './ProviderAgentIcon';

type ProviderType = 'claude_code' | 'codex_app_server' | 'gemini_cli' | 'gpt' | 'anthropic' | 'google' | 'claude_llm' | 'gemini_llm';
type HubTab = 'agents' | 'teams' | 'workflows';
type AgentDialogMode = 'create' | 'edit' | 'view' | null;
type TeamDialogMode = 'create' | 'edit' | 'view' | null;
type WorkflowDialogMode = 'view' | null;
type AvatarStyleId = 'clean_glyph' | 'soft_3d' | 'glass_icon' | 'pixel_badge' | 'ink_line' | 'paper_cut' | 'blueprint' | 'enamel_sticker' | 'custom';

type AgentDraft = {
  agentId: string;
  displayName: string;
  providerType: ProviderType;
  model: string;
  modelReasoningEffort: string;
  modelServiceTier: string;
  authSource: string;
  apiKey: string;
  baseUrl: string;
  defaultWorkspaceDir: string;
  avatarDataUrl: string;
  systemPrompt: string;
};

const PROVIDER_DEFAULT_MODEL_VALUE = '__provider_default__';
const PROVIDER_DEFAULT_REASONING_VALUE = '__provider_default_reasoning__';
const PROVIDER_DEFAULT_SERVICE_TIER_VALUE = '__provider_default_service_tier__';
const AGENT_AVATAR_MAX_BYTES = 3 * 1024 * 1024;
const AGENT_AVATAR_SIZE = 256;
const AGENT_AVATAR_DATA_URL_MAX_LENGTH = 700_000;
const AGENT_AVATAR_ACCEPT = 'image/png,image/jpeg,image/webp,image/svg+xml';
const CUSTOM_AVATAR_STYLE_ID: AvatarStyleId = 'custom';
const DEFAULT_AVATAR_STYLE_ID: AvatarStyleId = 'clean_glyph';
const FALLBACK_REASONING_EFFORTS = [
  { id: 'none', label: 'None', description: 'No reasoning', recommended: false },
  { id: 'minimal', label: 'Minimal', description: 'Minimal reasoning', recommended: false },
  { id: 'low', label: 'Low', description: 'Faster responses', recommended: false },
  { id: 'medium', label: 'Medium', description: 'Balanced speed and reasoning', recommended: true },
  { id: 'high', label: 'High', description: 'Deeper reasoning', recommended: false },
  { id: 'xhigh', label: 'Extra High', description: 'Maximum reasoning', recommended: false },
];

const AVATAR_STYLE_OPTIONS: Array<{
  id: Exclude<AvatarStyleId, 'custom'>;
  label: string;
  prompt: string;
}> = [
  {
    id: 'clean_glyph',
    label: 'Clean glyph',
    prompt: 'minimal vector glyph, simple geometric mark, balanced negative space, charcoal base with one sharp accent color',
  },
  {
    id: 'soft_3d',
    label: 'Soft 3D',
    prompt: 'soft 3D clay icon, rounded abstract forms, gentle studio lighting, compact and friendly without looking childish',
  },
  {
    id: 'glass_icon',
    label: 'Glass icon',
    prompt: 'translucent glassmorphism icon, crisp inner symbol, subtle refraction, clean depth, restrained blue green accent',
  },
  {
    id: 'pixel_badge',
    label: 'Pixel badge',
    prompt: 'premium pixel-art badge, 32-bit style, readable blocky silhouette, limited palette, modern developer-tool feel',
  },
  {
    id: 'ink_line',
    label: 'Ink line',
    prompt: 'monoline ink icon, expressive black linework, small accent fill, simple abstract agent signal, high legibility',
  },
  {
    id: 'paper_cut',
    label: 'Paper cut',
    prompt: 'layered paper-cut icon, crisp stacked shapes, soft shadow, warm neutral base with a bright teal accent, high contrast silhouette',
  },
  {
    id: 'blueprint',
    label: 'Blueprint',
    prompt: 'technical blueprint emblem, precise line grid, subtle cyan ink on deep charcoal, schematic but simple, readable at small sizes',
  },
  {
    id: 'enamel_sticker',
    label: 'Enamel sticker',
    prompt: 'polished enamel sticker badge, bold flat shapes, thick clean outline, optimistic coral and mint accents, crisp app-icon finish',
  },
];

type TeamDraft = {
  teamId: string;
  displayName: string;
  avatarDataUrl: string;
  leaderAgentId: string;
  memberAgentIds: string[];
  workflowText: string;
};

type AgentsHubPanelProps = {
  initialTab?: HubTab;
  workspaces?: DesktopWorkspace[];
  onAddWorkspace?: (path: string) => Promise<DesktopWorkspace | null>;
  onStartThread?: (agentOrTeamId: string) => void;
  onOpenMemory?: (agent: DesktopCustomAgent) => void;
  onToast?: (message: string, tone?: 'success' | 'error' | 'info', durationMs?: number) => void;
};

function emptyAgentDraft(): AgentDraft {
  return {
    agentId: '',
    displayName: '',
    providerType: 'claude_code',
    model: '',
    modelReasoningEffort: '',
    modelServiceTier: '',
    authSource: 'codex',
    apiKey: '',
    baseUrl: '',
    defaultWorkspaceDir: '',
    avatarDataUrl: '',
    systemPrompt: '',
  };
}

function emptyTeamDraft(): TeamDraft {
  return {
    teamId: '',
    displayName: '',
    avatarDataUrl: '',
    leaderAgentId: '',
    memberAgentIds: [],
    workflowText: '',
  };
}

function deriveId(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .replace(/-{2,}/g, '-');
}

function providerLabel(value: ProviderType): string {
  if (value === 'codex_app_server') {
    return 'Codex';
  }
  if (value === 'gemini_cli') {
    return 'Gemini';
  }
  if (value === 'gpt') {
    return 'GPT';
  }
  if (value === 'anthropic' || value === 'claude_llm') {
    return 'Claude';
  }
  if (value === 'google' || value === 'gemini_llm') {
    return 'Gemini';
  }
  return 'Claude';
}

function isNativeModelProvider(value: ProviderType): boolean {
  return value === 'gpt' || value === 'anthropic' || value === 'google' || value === 'claude_llm' || value === 'gemini_llm';
}

function defaultAuthSource(value: ProviderType): string {
  return value === 'gpt' ? 'codex' : 'api_key';
}

function apiKeyEnvName(value: ProviderType): string | null {
  if (value === 'gpt') {
    return 'OPENAI_API_KEY';
  }
  if (value === 'anthropic' || value === 'claude_llm') {
    return 'ANTHROPIC_API_KEY';
  }
  if (value === 'google' || value === 'gemini_llm') {
    return 'GEMINI_API_KEY';
  }
  return null;
}

function apiKeyFromAgent(agent: DesktopCustomAgent): string {
  const envName = apiKeyEnvName(agent.providerType as ProviderType);
  return envName ? agent.providerEnv?.[envName] || '' : '';
}

function previewText(value: string | null | undefined, fallback: string): string {
  const normalized = value?.replace(/\s+/g, ' ').trim() || '';
  if (!normalized) {
    return fallback;
  }
  return normalized.length > 140 ? `${normalized.slice(0, 137)}...` : normalized;
}

function providerModelsWithCurrent(
  providerModels: DesktopProviderModels | undefined,
  currentModel: string,
): DesktopProviderModels['models'] {
  const models = providerModels?.models || [];
  const normalized = currentModel.trim();
  if (!normalized || models.some((model) => model.id === normalized)) {
    return models;
  }
  return [{ id: normalized, label: normalized, description: null, recommended: false }, ...models];
}

function reasoningEffortsWithCurrent(
  providerModels: DesktopProviderModels | undefined,
  currentModel: string,
  currentEffort: string,
): DesktopProviderModels['models'] {
  const selectedModel = providerModels?.models.find((model) => model.id === currentModel.trim());
  const efforts = selectedModel?.supportedReasoningEfforts?.length
    ? selectedModel.supportedReasoningEfforts
    : providerModels?.reasoningEfforts?.length
    ? providerModels.reasoningEfforts
    : FALLBACK_REASONING_EFFORTS;
  const normalized = currentEffort.trim();
  if (!normalized || efforts.some((effort) => effort.id === normalized)) {
    return efforts;
  }
  return [{ id: normalized, label: normalized, description: null, recommended: false }, ...efforts];
}

function serviceTiersWithCurrent(
  providerModels: DesktopProviderModels | undefined,
  currentModel: string,
  currentServiceTier: string,
): DesktopProviderModels['models'] {
  const selectedModel = providerModels?.models.find((model) => model.id === currentModel.trim());
  const tiers = selectedModel?.serviceTiers?.length
    ? selectedModel.serviceTiers
    : providerModels?.serviceTiers?.length
    ? providerModels.serviceTiers
    : [];
  const normalized = currentServiceTier.trim();
  if (!normalized || tiers.some((tier) => tier.id === normalized)) {
    return tiers;
  }
  return [{ id: normalized, label: normalized, description: null, recommended: false }, ...tiers];
}

function avatarLabel(value: string): string {
  return value
    .split(/\s+/)
    .map((part) => part[0] || '')
    .join('')
    .slice(0, 2)
    .toUpperCase();
}

type AgentAvatarProps = {
  agentId?: string | null;
  avatarDataUrl?: string | null;
  builtIn?: boolean;
  className: string;
  label: string;
  providerIcon?: DesktopProviderIconDescriptor | null;
  providerType?: ProviderType | null;
  team?: boolean;
};

function AgentAvatar({
  agentId,
  avatarDataUrl,
  builtIn,
  className,
  label,
  providerIcon,
  providerType,
  team,
}: AgentAvatarProps) {
  const showProviderIcon =
    Boolean(builtIn && !team && !avatarDataUrl)
    && hasProviderAgentIcon(agentId, providerType, providerIcon);
  const classes = [
    className,
    builtIn ? 'builtin' : '',
    team ? 'team' : '',
    avatarDataUrl ? 'image' : '',
    showProviderIcon ? 'provider' : '',
  ].filter(Boolean).join(' ');

  return (
    <span className={classes}>
      {avatarDataUrl ? (
        <img alt="" src={avatarDataUrl} />
      ) : showProviderIcon ? (
        <ProviderAgentIcon
          agentId={agentId}
          className="agents-hub-provider-icon"
          providerIcon={providerIcon}
          providerType={providerType}
          size="100%"
        />
      ) : avatarLabel(label)}
    </span>
  );
}

function readFileAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error('Failed to read avatar image'));
    reader.onload = () => {
      if (typeof reader.result === 'string') {
        resolve(reader.result);
      } else {
        reject(new Error('Failed to read avatar image'));
      }
    };
    reader.readAsDataURL(file);
  });
}

function loadImageFromDataUrl(dataUrl: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onerror = () => reject(new Error('Failed to read avatar image'));
    image.onload = () => resolve(image);
    image.src = dataUrl;
  });
}

async function normalizeAvatarFile(file: File): Promise<string> {
  const sourceDataUrl = await readFileAsDataUrl(file);
  const image = await loadImageFromDataUrl(sourceDataUrl);
  const sourceWidth = image.naturalWidth || image.width;
  const sourceHeight = image.naturalHeight || image.height;
  if (!sourceWidth || !sourceHeight) {
    throw new Error('Failed to read avatar image');
  }

  const canvas = document.createElement('canvas');
  canvas.width = AGENT_AVATAR_SIZE;
  canvas.height = AGENT_AVATAR_SIZE;
  const context = canvas.getContext('2d');
  if (!context) {
    throw new Error('Failed to read avatar image');
  }

  const scale = Math.max(
    AGENT_AVATAR_SIZE / sourceWidth,
    AGENT_AVATAR_SIZE / sourceHeight,
  );
  const drawWidth = Math.round(sourceWidth * scale);
  const drawHeight = Math.round(sourceHeight * scale);
  const drawX = Math.round((AGENT_AVATAR_SIZE - drawWidth) / 2);
  const drawY = Math.round((AGENT_AVATAR_SIZE - drawHeight) / 2);

  context.imageSmoothingEnabled = true;
  context.imageSmoothingQuality = 'high';
  context.clearRect(0, 0, AGENT_AVATAR_SIZE, AGENT_AVATAR_SIZE);
  context.drawImage(image, drawX, drawY, drawWidth, drawHeight);

  const pngDataUrl = canvas.toDataURL('image/png');
  if (pngDataUrl.length <= AGENT_AVATAR_DATA_URL_MAX_LENGTH) {
    return pngDataUrl;
  }

  context.save();
  context.globalCompositeOperation = 'destination-over';
  context.fillStyle = '#f7f8fa';
  context.fillRect(0, 0, AGENT_AVATAR_SIZE, AGENT_AVATAR_SIZE);
  context.restore();

  const jpegDataUrl = canvas.toDataURL('image/jpeg', 0.88);
  if (jpegDataUrl.length <= AGENT_AVATAR_DATA_URL_MAX_LENGTH) {
    return jpegDataUrl;
  }

  throw new Error('Avatar image is too large.');
}

function buildSuggestedWorkflow(
  agents: DesktopCustomAgent[],
  leaderAgentId: string,
  memberAgentIds: string[],
): string {
  const nameById = new Map(agents.map((agent) => [agent.agentId, agent.displayName] as const));
  const leaderName = nameById.get(leaderAgentId) || leaderAgentId || 'Leader';
  const memberNames = memberAgentIds
    .map((agentId) => nameById.get(agentId) || agentId)
    .filter(Boolean);

  return [
    `${leaderName} receives the brief first, breaks the work into clear subtasks, and coordinates the team response.`,
    '',
    memberNames.length
      ? `Selected members: ${memberNames.join(', ')}.`
      : 'Selected members should explore focused slices of the task in parallel.',
    '',
    'Have members surface tradeoffs early, then merge the strongest ideas into one final answer with clear acceptance checks.',
  ].join('\n');
}

function sortedAgents(value: DesktopCustomAgent[]): DesktopCustomAgent[] {
  return [...value]
    .filter((agent) => agent.standalone)
    .sort((left, right) => {
      if (left.builtIn !== right.builtIn) {
        return left.builtIn ? -1 : 1;
      }
      return left.displayName.localeCompare(right.displayName) || left.agentId.localeCompare(right.agentId);
    });
}

function sortedTeams(value: DesktopTeam[]): DesktopTeam[] {
  return [...value].sort((left, right) => {
    return left.displayName.localeCompare(right.displayName) || left.teamId.localeCompare(right.teamId);
  });
}

function sortedWorkflows(value: DesktopWorkflowDefinition[]): DesktopWorkflowDefinition[] {
  return [...value].sort((left, right) => {
    return left.name.localeCompare(right.name) || left.workflowId.localeCompare(right.workflowId);
  });
}

function workflowDefaultWorkspace(workflow: DesktopWorkflowDefinition): string {
  const value = workflow.defaults?.workspaceDir || workflow.defaults?.workspace_dir;
  return typeof value === 'string' && value.trim() ? value.trim() : '';
}

function workflowInputPlaceholder(workflow: DesktopWorkflowDefinition): string {
  const value = workflow.input?.placeholder;
  return typeof value === 'string' && value.trim() ? value.trim() : '';
}

function prettyJson(value: unknown): string {
  if (!value || typeof value !== 'object') {
    return '';
  }
  return JSON.stringify(value, null, 2);
}

function stopEvent(event: React.MouseEvent<HTMLElement>) {
  event.preventDefault();
  event.stopPropagation();
}

export function AgentsHubPanel({
  initialTab = 'agents',
  workspaces = [],
  onAddWorkspace,
  onStartThread,
  onOpenMemory,
  onToast,
}: AgentsHubPanelProps) {
  const { t } = useI18n();
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [agents, setAgents] = useState<DesktopCustomAgent[]>([]);
  const [teams, setTeams] = useState<DesktopTeam[]>([]);
  const [workflows, setWorkflows] = useState<DesktopWorkflowDefinition[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [search, setSearch] = useState('');
  const [activeTab, setActiveTab] = useState<HubTab>(initialTab);

  const [agentDialogMode, setAgentDialogMode] = useState<AgentDialogMode>(null);
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
  const [agentDraft, setAgentDraft] = useState<AgentDraft>(() => emptyAgentDraft());
  const [agentIdTouched, setAgentIdTouched] = useState(false);
  const [avatarGenerating, setAvatarGenerating] = useState(false);
  const [avatarStyleDialogOpen, setAvatarStyleDialogOpen] = useState(false);
  const [avatarStyleTarget, setAvatarStyleTarget] = useState<'agent' | 'team'>('agent');
  const [avatarStyleId, setAvatarStyleId] = useState<AvatarStyleId>(DEFAULT_AVATAR_STYLE_ID);
  const [customAvatarStyle, setCustomAvatarStyle] = useState('');
  const avatarFileInputRef = useRef<HTMLInputElement | null>(null);
  const teamAvatarFileInputRef = useRef<HTMLInputElement | null>(null);
  const workflowSourceRequestId = useRef(0);
  const [providerModelsByType, setProviderModelsByType] = useState<
    Partial<Record<ProviderType, DesktopProviderModels>>
  >({});
  const [providerModelsLoading, setProviderModelsLoading] = useState<
    Partial<Record<ProviderType, boolean>>
  >({});

  const [teamDialogMode, setTeamDialogMode] = useState<TeamDialogMode>(null);
  const [selectedTeamId, setSelectedTeamId] = useState<string | null>(null);
  const [teamDraft, setTeamDraft] = useState<TeamDraft>(() => emptyTeamDraft());
  const [teamIdTouched, setTeamIdTouched] = useState(false);
  const [workflowDialogMode, setWorkflowDialogMode] = useState<WorkflowDialogMode>(null);
  const [selectedWorkflowId, setSelectedWorkflowId] = useState<string | null>(null);
  const [workflowSource, setWorkflowSource] = useState<DesktopWorkflowSourceDocument | null>(null);
  const [workflowSourceLoading, setWorkflowSourceLoading] = useState(false);
  const [workflowSourceError, setWorkflowSourceError] = useState<string | null>(null);

  useEffect(() => {
    setActiveTab(initialTab);
  }, [initialTab]);

  async function loadData() {
    setLoading(true);
    setLoadError(null);
    try {
      const [agentsResult, teamsResult, workflowsResult] = await Promise.allSettled([
        window.garyxDesktop.listCustomAgents(),
        window.garyxDesktop.listTeams(),
        window.garyxDesktop.listWorkflowDefinitions(),
      ]);

      const nextAgents = agentsResult.status === 'fulfilled' ? sortedAgents(agentsResult.value) : [];
      const nextTeams = teamsResult.status === 'fulfilled' ? sortedTeams(teamsResult.value) : [];
      const nextWorkflows = workflowsResult.status === 'fulfilled' ? sortedWorkflows(workflowsResult.value) : [];
      setAgents(nextAgents);
      setTeams(nextTeams);
      setWorkflows(nextWorkflows);

      const failures = [
        agentsResult.status === 'rejected' ? 'agents' : null,
        teamsResult.status === 'rejected' ? 'teams' : null,
        workflowsResult.status === 'rejected' ? 'workflows' : null,
      ].filter(Boolean);

      if (failures.length) {
        const message = `Failed to fully load ${failures.join(' and ')}.`;
        setLoadError(message);
        onToast?.(message, 'error');
      }
    } finally {
      setLoading(false);
    }
  }

  async function ensureProviderModels(providerType: ProviderType) {
    if (providerModelsByType[providerType] || providerModelsLoading[providerType]) {
      return;
    }
    setProviderModelsLoading((current) => ({ ...current, [providerType]: true }));
    try {
      const result = await window.garyxDesktop.listProviderModels(providerType);
      setProviderModelsByType((current) => ({ ...current, [providerType]: result }));
    } catch (error) {
      setProviderModelsByType((current) => ({
        ...current,
        [providerType]: {
          providerType,
          supportsModelSelection: false,
          models: [],
          defaultModel: null,
          source: 'desktop',
          error: error instanceof Error ? error.message : t('Failed to load models'),
        },
      }));
    } finally {
      setProviderModelsLoading((current) => ({ ...current, [providerType]: false }));
    }
  }

  useEffect(() => {
    void loadData();
  }, []);

  useEffect(() => {
    if (agentDialogMode === 'create' || agentDialogMode === 'edit') {
      void ensureProviderModels(agentDraft.providerType);
    }
  }, [agentDialogMode, agentDraft.providerType]);

  useEffect(() => {
    if (agentDialogMode !== 'create' || agentIdTouched) {
      return;
    }
    const nextId = deriveId(agentDraft.displayName);
    setAgentDraft((current) => (current.agentId === nextId ? current : { ...current, agentId: nextId }));
  }, [agentDialogMode, agentDraft.displayName, agentIdTouched]);

  useEffect(() => {
    if (teamDialogMode !== 'create' || teamIdTouched) {
      return;
    }
    const nextId = deriveId(teamDraft.displayName);
    setTeamDraft((current) => (current.teamId === nextId ? current : { ...current, teamId: nextId }));
  }, [teamDialogMode, teamDraft.displayName, teamIdTouched]);

  const agentMap = useMemo(() => {
    return new Map(agents.map((agent) => [agent.agentId, agent] as const));
  }, [agents]);

  const selectedAgent = useMemo(
    () => agents.find((agent) => agent.agentId === selectedAgentId) || null,
    [agents, selectedAgentId],
  );
  const selectedTeam = useMemo(
    () => teams.find((team) => team.teamId === selectedTeamId) || null,
    [teams, selectedTeamId],
  );
  const selectedWorkflow = useMemo(
    () => workflows.find((workflow) => workflow.workflowId === selectedWorkflowId) || null,
    [selectedWorkflowId, workflows],
  );

  const filteredAgents = useMemo(() => {
    const needle = search.trim().toLowerCase();
    if (!needle) {
      return agents;
    }
    return agents.filter((agent) => {
      return [
        agent.displayName,
        agent.agentId,
        providerLabel(agent.providerType),
        agent.systemPrompt,
        agent.builtIn ? 'built-in' : 'custom',
      ].some((value) => value.toLowerCase().includes(needle));
    });
  }, [agents, search]);

  const filteredTeams = useMemo(() => {
    const needle = search.trim().toLowerCase();
    if (!needle) {
      return teams;
    }
    return teams.filter((team) => {
      const memberLabels = team.memberAgentIds
        .map((agentId) => agentMap.get(agentId)?.displayName || agentId)
        .join(' ');
      return [
        team.displayName,
        team.teamId,
        team.workflowText,
        agentMap.get(team.leaderAgentId)?.displayName || team.leaderAgentId,
        memberLabels,
      ].some((value) => value.toLowerCase().includes(needle));
    });
  }, [agentMap, search, teams]);

  const filteredWorkflows = useMemo(() => {
    const needle = search.trim().toLowerCase();
    if (!needle) {
      return workflows;
    }
    return workflows.filter((workflow) => {
      return [
        workflow.name,
        workflow.workflowId,
        workflow.description,
        workflow.packageDir || '',
        workflowDefaultWorkspace(workflow),
      ].some((value) => value.toLowerCase().includes(needle));
    });
  }, [search, workflows]);

  const teamSelectionCount = useMemo(() => {
    return agents.filter((agent) => teamDraft.memberAgentIds.includes(agent.agentId)).length;
  }, [agents, teamDraft.memberAgentIds]);

  const allAgentsSelected = agents.length > 0 && teamSelectionCount === agents.length;
  const teamMemberSelectionState = allAgentsSelected
    ? true
    : teamSelectionCount > 0
      ? 'indeterminate'
      : false;
  const activeAgentProviderModels = providerModelsByType[agentDraft.providerType];
  const agentProviderModelsLoading = providerModelsLoading[agentDraft.providerType] === true;
  const agentModelOptions = providerModelsWithCurrent(activeAgentProviderModels, agentDraft.model);
  const agentSupportsModelSelection =
    activeAgentProviderModels?.supportsModelSelection === true && agentModelOptions.length > 0;
  const agentReasoningEffortOptions = reasoningEffortsWithCurrent(
    activeAgentProviderModels,
    agentDraft.model || activeAgentProviderModels?.defaultModel || '',
    agentDraft.modelReasoningEffort,
  );
  const agentSupportsReasoningEffortSelection =
    (agentDraft.providerType === 'codex_app_server' || isNativeModelProvider(agentDraft.providerType))
    && agentReasoningEffortOptions.length > 0;
  const agentServiceTierOptions = serviceTiersWithCurrent(
    activeAgentProviderModels,
    agentDraft.model || activeAgentProviderModels?.defaultModel || '',
    agentDraft.modelServiceTier,
  );
  const agentSupportsServiceTierSelection =
    agentDraft.providerType === 'gpt'
    && (activeAgentProviderModels?.supportsServiceTierSelection === true || agentDraft.modelServiceTier.trim().length > 0)
    && agentServiceTierOptions.length > 0;
  const agentModelStatus =
    agentDraft.providerType === 'gemini_cli' && !agentSupportsModelSelection
      ? agentProviderModelsLoading
        ? t('Loading models from local Gemini ACP...')
        : activeAgentProviderModels?.error
          ? t('Local Gemini ACP does not expose a model list yet.')
          : null
      : null;
  const activeAvatarStylePrompt = avatarStyleId === CUSTOM_AVATAR_STYLE_ID
    ? customAvatarStyle.trim()
    : AVATAR_STYLE_OPTIONS.find((option) => option.id === avatarStyleId)?.prompt || '';
  const avatarStyleValidationError =
    avatarStyleId === CUSTOM_AVATAR_STYLE_ID && !customAvatarStyle.trim()
      ? t('Custom style is required.')
      : null;

  function closeAgentDialog() {
    setAgentDialogMode(null);
    setSelectedAgentId(null);
    setAgentDraft(emptyAgentDraft());
    setAgentIdTouched(false);
    setAvatarStyleDialogOpen(false);
    setAvatarStyleTarget('agent');
    setAvatarStyleId(DEFAULT_AVATAR_STYLE_ID);
    setCustomAvatarStyle('');
  }

  function closeTeamDialog() {
    setTeamDialogMode(null);
    setSelectedTeamId(null);
    setTeamDraft(emptyTeamDraft());
    setTeamIdTouched(false);
    setAvatarStyleDialogOpen(false);
    setAvatarStyleTarget('agent');
    setAvatarStyleId(DEFAULT_AVATAR_STYLE_ID);
    setCustomAvatarStyle('');
  }

  function closeWorkflowDialog() {
    workflowSourceRequestId.current += 1;
    setWorkflowDialogMode(null);
    setSelectedWorkflowId(null);
    setWorkflowSource(null);
    setWorkflowSourceLoading(false);
    setWorkflowSourceError(null);
  }

  async function loadWorkflowSource(workflowId: string) {
    const requestId = workflowSourceRequestId.current + 1;
    workflowSourceRequestId.current = requestId;
    setWorkflowSource(null);
    setWorkflowSourceError(null);
    setWorkflowSourceLoading(true);
    try {
      const source = await window.garyxDesktop.getWorkflowDefinitionSource({ workflowId });
      if (workflowSourceRequestId.current === requestId) {
        setWorkflowSource(source);
      }
    } catch (error) {
      if (workflowSourceRequestId.current === requestId) {
        setWorkflowSourceError(error instanceof Error ? error.message : t('Failed to load workflow source'));
      }
    } finally {
      if (workflowSourceRequestId.current === requestId) {
        setWorkflowSourceLoading(false);
      }
    }
  }

  function openCreateAgentDialog() {
    setAgentDialogMode('create');
    setSelectedAgentId(null);
    setAgentDraft(emptyAgentDraft());
    setAgentIdTouched(false);
    setAvatarStyleTarget('agent');
    setAvatarStyleId(DEFAULT_AVATAR_STYLE_ID);
    setCustomAvatarStyle('');
  }

  function openViewAgentDialog(agent: DesktopCustomAgent) {
    setAgentDialogMode('view');
    setSelectedAgentId(agent.agentId);
    setAgentDraft({
      agentId: agent.agentId,
      displayName: agent.displayName,
      providerType: agent.providerType,
      model: agent.model,
      modelReasoningEffort: agent.modelReasoningEffort,
      modelServiceTier: agent.modelServiceTier,
      authSource: agent.authSource || defaultAuthSource(agent.providerType as ProviderType),
      apiKey: apiKeyFromAgent(agent),
      baseUrl: agent.baseUrl || '',
      defaultWorkspaceDir: agent.defaultWorkspaceDir,
      avatarDataUrl: agent.avatarDataUrl,
      systemPrompt: agent.systemPrompt,
    });
    setAgentIdTouched(true);
    setAvatarStyleTarget('agent');
    setAvatarStyleId(DEFAULT_AVATAR_STYLE_ID);
    setCustomAvatarStyle('');
  }

  function openEditAgentDialog(agent: DesktopCustomAgent) {
    if (agent.builtIn) {
      openViewAgentDialog(agent);
      return;
    }
    setAgentDialogMode('edit');
    setSelectedAgentId(agent.agentId);
    setAgentDraft({
      agentId: agent.agentId,
      displayName: agent.displayName,
      providerType: agent.providerType,
      model: agent.model,
      modelReasoningEffort: agent.modelReasoningEffort,
      modelServiceTier: agent.modelServiceTier,
      authSource: agent.authSource || defaultAuthSource(agent.providerType as ProviderType),
      apiKey: apiKeyFromAgent(agent),
      baseUrl: agent.baseUrl || '',
      defaultWorkspaceDir: agent.defaultWorkspaceDir,
      avatarDataUrl: agent.avatarDataUrl,
      systemPrompt: agent.systemPrompt,
    });
    setAgentIdTouched(true);
    setAvatarStyleTarget('agent');
    setAvatarStyleId(DEFAULT_AVATAR_STYLE_ID);
    setCustomAvatarStyle('');
  }

  function openCreateTeamDialog(seedAgentId?: string) {
    const seedAgent = seedAgentId ? agentMap.get(seedAgentId) || null : null;
    const nextDisplayName = seedAgent ? `${seedAgent.displayName} Team` : '';
    const nextLeaderAgentId = seedAgent?.agentId || '';
    const nextMemberAgentIds = seedAgent ? [seedAgent.agentId] : [];
    setTeamDialogMode('create');
    setSelectedTeamId(null);
    setTeamDraft({
      teamId: '',
      displayName: nextDisplayName,
      avatarDataUrl: '',
      leaderAgentId: nextLeaderAgentId,
      memberAgentIds: nextMemberAgentIds,
      workflowText: buildSuggestedWorkflow(agents, nextLeaderAgentId, nextMemberAgentIds),
    });
    setTeamIdTouched(false);
    setAvatarStyleTarget('team');
    setAvatarStyleId(DEFAULT_AVATAR_STYLE_ID);
    setCustomAvatarStyle('');
    setActiveTab('teams');
  }

  function openViewTeamDialog(team: DesktopTeam) {
    setTeamDialogMode('view');
    setSelectedTeamId(team.teamId);
    setTeamDraft({
      teamId: team.teamId,
      displayName: team.displayName,
      avatarDataUrl: team.avatarDataUrl,
      leaderAgentId: team.leaderAgentId,
      memberAgentIds: [...team.memberAgentIds],
      workflowText: team.workflowText,
    });
    setTeamIdTouched(true);
    setAvatarStyleTarget('team');
    setAvatarStyleId(DEFAULT_AVATAR_STYLE_ID);
    setCustomAvatarStyle('');
  }

  function openEditTeamDialog(team: DesktopTeam) {
    setTeamDialogMode('edit');
    setSelectedTeamId(team.teamId);
    setTeamDraft({
      teamId: team.teamId,
      displayName: team.displayName,
      avatarDataUrl: team.avatarDataUrl,
      leaderAgentId: team.leaderAgentId,
      memberAgentIds: [...team.memberAgentIds],
      workflowText: team.workflowText,
    });
    setTeamIdTouched(true);
    setAvatarStyleTarget('team');
    setAvatarStyleId(DEFAULT_AVATAR_STYLE_ID);
    setCustomAvatarStyle('');
    setActiveTab('teams');
  }

  function openViewWorkflowDialog(workflow: DesktopWorkflowDefinition) {
    setWorkflowDialogMode('view');
    setSelectedWorkflowId(workflow.workflowId);
    void loadWorkflowSource(workflow.workflowId);
  }

  function toggleTeamMember(agentId: string) {
    setTeamDraft((current) => {
      const exists = current.memberAgentIds.includes(agentId);
      const memberAgentIds = exists
        ? current.memberAgentIds.filter((entry) => entry !== agentId)
        : [...current.memberAgentIds, agentId];
      // If leader was removed or no leader set, default to first member
      const leaderAgentId = memberAgentIds.includes(current.leaderAgentId)
        ? current.leaderAgentId
        : memberAgentIds[0] || '';
      return { ...current, memberAgentIds, leaderAgentId };
    });
  }

  function selectTeamLeader(agentId: string) {
    setTeamDraft((current) => {
      const memberAgentIds = current.memberAgentIds.includes(agentId)
        ? current.memberAgentIds
        : [agentId, ...current.memberAgentIds];
      return { ...current, leaderAgentId: agentId, memberAgentIds };
    });
  }

  function selectAllTeamMembers(nextChecked: boolean) {
    setTeamDraft((current) => {
      const preservedLeaderIds = current.leaderAgentId ? [current.leaderAgentId] : [];
      return {
        ...current,
        memberAgentIds: nextChecked
          ? Array.from(new Set([...preservedLeaderIds, ...agents.map((agent) => agent.agentId)]))
          : preservedLeaderIds,
      };
    });
  }

  async function handleAvatarFileChange(
    event: React.ChangeEvent<HTMLInputElement>,
    target: 'agent' | 'team' = 'agent',
  ) {
    const file = event.target.files?.[0] || null;
    event.target.value = '';
    if (!file) {
      return;
    }
    if (file.size > AGENT_AVATAR_MAX_BYTES) {
      onToast?.(t('Avatar image is too large.'), 'error');
      return;
    }
    if (!file.type.startsWith('image/')) {
      onToast?.(t('Choose an image file.'), 'error');
      return;
    }
    try {
      const avatarDataUrl = await normalizeAvatarFile(file);
      if (target === 'team') {
        setTeamDraft((current) => ({ ...current, avatarDataUrl }));
      } else {
        setAgentDraft((current) => ({ ...current, avatarDataUrl }));
      }
    } catch (error) {
      const message = error instanceof Error && error.message === 'Avatar image is too large.'
        ? error.message
        : 'Failed to read avatar image';
      onToast?.(t(message), 'error');
    }
  }

  async function handleGenerateAvatar(stylePrompt: string) {
    const target = avatarStyleTarget;
    const displayName = target === 'team'
      ? teamDraft.displayName.trim()
      : agentDraft.displayName.trim();
    const agentId = target === 'team'
      ? teamDraft.teamId.trim()
      : agentDraft.agentId.trim();
    if (!displayName && !agentId) {
      onToast?.(t('Name is required.'), 'error');
      return;
    }
    setAvatarGenerating(true);
    try {
      const result = await window.garyxDesktop.generateCustomAgentAvatar({
        agentId,
        displayName: displayName || agentId,
        kind: target,
        stylePrompt,
      });
      if (target === 'team') {
        setTeamDraft((current) => ({
          ...current,
          avatarDataUrl: result.avatarDataUrl,
        }));
      } else {
        setAgentDraft((current) => ({
          ...current,
          avatarDataUrl: result.avatarDataUrl,
        }));
      }
      setAvatarStyleDialogOpen(false);
      onToast?.(t('Avatar generated'), 'success');
    } catch {
      onToast?.(t('Failed to generate avatar'), 'error');
    } finally {
      setAvatarGenerating(false);
    }
  }

  async function handleAgentSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const avatarDataUrl = agentDraft.avatarDataUrl.trim();
    if (avatarDataUrl.length > AGENT_AVATAR_DATA_URL_MAX_LENGTH) {
      onToast?.(t('Avatar image is too large.'), 'error');
      return;
    }
    setSaving(true);
    try {
      const nativeProvider = isNativeModelProvider(agentDraft.providerType);
      const apiKeyEnv = apiKeyEnvName(agentDraft.providerType);
      const providerEnv = nativeProvider && apiKeyEnv && agentDraft.apiKey.trim()
        ? { [apiKeyEnv]: agentDraft.apiKey.trim() }
        : null;
      const payload: CreateCustomAgentInput = {
        agentId: agentDraft.agentId.trim(),
        displayName: agentDraft.displayName.trim(),
        providerType: agentDraft.providerType,
        model: agentSupportsModelSelection ? agentDraft.model.trim() : '',
        modelReasoningEffort: agentSupportsReasoningEffortSelection ? agentDraft.modelReasoningEffort.trim() : '',
        modelServiceTier: agentSupportsServiceTierSelection ? agentDraft.modelServiceTier.trim() : '',
        providerEnv,
        authSource: nativeProvider
          ? (agentDraft.authSource.trim() || defaultAuthSource(agentDraft.providerType))
          : null,
        baseUrl: nativeProvider ? agentDraft.baseUrl.trim() : null,
        defaultWorkspaceDir: agentDraft.defaultWorkspaceDir.trim(),
        avatarDataUrl,
        systemPrompt: agentDraft.systemPrompt.trim(),
      };

      if (agentDialogMode === 'create') {
        await window.garyxDesktop.createCustomAgent(payload);
        onToast?.(t('Custom agent created'), 'success');
      } else {
        const updatePayload: UpdateCustomAgentInput = {
          ...payload,
          currentAgentId: selectedAgent?.agentId || payload.agentId,
        };
        await window.garyxDesktop.updateCustomAgent(updatePayload);
        onToast?.(t('Custom agent updated'), 'success');
      }

      closeAgentDialog();
      await loadData();
    } catch (error) {
      onToast?.(error instanceof Error ? error.message : t('Failed to save custom agent'), 'error');
    } finally {
      setSaving(false);
    }
  }

  async function handleDeleteAgent(agent: DesktopCustomAgent) {
    if (agent.builtIn) {
      return;
    }
    setSaving(true);
    try {
      await window.garyxDesktop.deleteCustomAgent({ agentId: agent.agentId });
      onToast?.(t('Custom agent deleted'), 'success');
      closeAgentDialog();
      await loadData();
    } catch (error) {
      onToast?.(error instanceof Error ? error.message : t('Failed to delete custom agent'), 'error');
    } finally {
      setSaving(false);
    }
  }

  async function handleTeamSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const avatarDataUrl = teamDraft.avatarDataUrl.trim();
    if (avatarDataUrl.length > AGENT_AVATAR_DATA_URL_MAX_LENGTH) {
      onToast?.(t('Avatar image is too large.'), 'error');
      return;
    }
    setSaving(true);
    try {
      const workflowText = teamDraft.workflowText.trim()
        || buildSuggestedWorkflow(agents, teamDraft.leaderAgentId, teamDraft.memberAgentIds);
      const payload: CreateTeamInput = {
        teamId: teamDraft.teamId.trim(),
        displayName: teamDraft.displayName.trim(),
        leaderAgentId: teamDraft.leaderAgentId.trim(),
        memberAgentIds: teamDraft.memberAgentIds,
        workflowText,
        avatarDataUrl,
      };

      if (teamDialogMode === 'create') {
        await window.garyxDesktop.createTeam(payload);
        onToast?.(t('Agent team created'), 'success');
      } else {
        const updatePayload: UpdateTeamInput = {
          ...payload,
          currentTeamId: selectedTeam?.teamId || payload.teamId,
        };
        await window.garyxDesktop.updateTeam(updatePayload);
        onToast?.(t('Agent team updated'), 'success');
      }

      closeTeamDialog();
      await loadData();
    } catch (error) {
      onToast?.(error instanceof Error ? error.message : t('Failed to save team'), 'error');
    } finally {
      setSaving(false);
    }
  }

  async function handleDeleteTeam(team: DesktopTeam) {
    setSaving(true);
    try {
      await window.garyxDesktop.deleteTeam({ teamId: team.teamId });
      onToast?.(t('Agent team deleted'), 'success');
      closeTeamDialog();
      await loadData();
    } catch (error) {
      onToast?.(error instanceof Error ? error.message : t('Failed to delete team'), 'error');
    } finally {
      setSaving(false);
    }
  }

  const agentValidationError =
    !agentDraft.displayName.trim()
      ? t('Name is required.')
      : !agentDraft.agentId.trim()
        ? t('Agent ID is required.')
        : !agentDraft.systemPrompt.trim()
          ? t('System prompt is required.')
          : null;

  const teamValidationError =
    !teamDraft.displayName.trim()
      ? t('Team name is required.')
      : !teamDraft.teamId.trim()
        ? t('Team ID is required.')
        : teamDraft.memberAgentIds.length === 0
          ? t('Select at least one member.')
          : !teamDraft.leaderAgentId.trim()
            ? t('Select a leader.')
            : !teamDraft.memberAgentIds.includes(teamDraft.leaderAgentId)
              ? t('Leader must be part of the team.')
              : null;

  const showingAgents = activeTab === 'agents';
  const showingTeams = activeTab === 'teams';
  const showingWorkflows = activeTab === 'workflows';
  const visibleAgents = filteredAgents;
  const visibleTeams = filteredTeams;
  const visibleWorkflows = filteredWorkflows;

  return (
    <div className="agents-hub">
      <div className="agents-hub-hero">
        <div className="agents-hub-tabs" role="tablist" aria-label={t("Agent registry sections")}>
          <button
            className={`agents-hub-tab ${showingAgents ? 'active' : ''}`}
            onClick={() => {
              setActiveTab('agents');
            }}
            role="tab"
            type="button"
          >
            <span>{t("Agent")}</span>
            <Badge className="agents-hub-tab-badge" variant="outline">{agents.length}</Badge>
          </button>
          <button
            className={`agents-hub-tab ${showingTeams ? 'active' : ''}`}
            onClick={() => {
              setActiveTab('teams');
            }}
            role="tab"
            type="button"
          >
            <span>{t("Agent Team")}</span>
            <Badge className="agents-hub-tab-badge" variant="outline">{teams.length}</Badge>
          </button>
          <button
            className={`agents-hub-tab ${showingWorkflows ? 'active' : ''}`}
            onClick={() => {
              setActiveTab('workflows');
            }}
            role="tab"
            type="button"
          >
            <span>{t("Workflow")}</span>
            <Badge className="agents-hub-tab-badge" variant="outline">{workflows.length}</Badge>
          </button>
        </div>

        <div className="agents-hub-controls">
          <div className="agents-hub-search">
            <IconSearch aria-hidden size={16} stroke={1.8} />
            <Input
              className="agents-hub-search-input"
              onChange={(event) => {
                setSearch(event.target.value);
              }}
              placeholder={t("Search...")}
              value={search}
            />
          </div>

          {!showingWorkflows ? (
            <Button
              onClick={showingAgents ? openCreateAgentDialog : () => openCreateTeamDialog()}
              size="sm"
            >
              <IconPlus aria-hidden size={15} stroke={2} />
              {showingAgents ? t('New Agent') : t('New Team')}
            </Button>
          ) : null}
        </div>
      </div>

      {loadError ? (
        <div className="codex-inline-hint" style={{ color: 'var(--color-token-error-foreground)' }}>{loadError}</div>
      ) : null}

      {loading ? (
        <div className="agents-hub-empty-state">{t('Loading...')}</div>
      ) : (
        <Table className="agents-hub-table">
          <TableHeader>
            <TableRow>
              <TableHead style={{ width: '40%' }}>{t('Name')}</TableHead>
              <TableHead style={{ width: '20%' }}>
                {showingAgents ? t('Provider') : showingTeams ? t('Leader') : t('Version')}
              </TableHead>
              <TableHead style={{ width: '20%' }}>
                {showingAgents ? t('Type') : showingTeams ? t('Members') : t('Workspace')}
              </TableHead>
              <TableHead style={{ width: '20%' }} className="text-right">
                {showingWorkflows ? t('Package') : t('Actions')}
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {showingAgents ? (
              visibleAgents.length ? (
                visibleAgents.map((agent) => (
                  <TableRow
                    className="cursor-pointer"
                    key={agent.agentId}
                    onClick={() => openViewAgentDialog(agent)}
                  >
                    <TableCell>
                      <div className="agents-hub-name-cell">
                        <AgentAvatar
                          agentId={agent.agentId}
                          avatarDataUrl={agent.avatarDataUrl}
                          builtIn={agent.builtIn}
                          className="agents-hub-avatar-sm"
                          label={agent.displayName || agent.agentId}
                          providerIcon={agent.providerIcon}
                          providerType={agent.providerType}
                        />
                        <div>
                          <div className="agents-hub-cell-name">{agent.displayName}</div>
                          <div className="agents-hub-cell-id">{agent.agentId}</div>
                        </div>
                      </div>
                    </TableCell>
                    <TableCell>{providerLabel(agent.providerType)}</TableCell>
                    <TableCell>
                      <Badge variant="outline">{agent.builtIn ? t('Built-in') : t('Custom')}</Badge>
                    </TableCell>
                    <TableCell className="text-right">
                      <div className="agents-hub-row-actions">
                        <Button
                          onClick={(e) => { stopEvent(e); onStartThread?.(agent.agentId); }}
                          size="sm"
                          variant="outline"
                        >
                          {t('Chat')}
                        </Button>
                        {!agent.builtIn ? (
                          <Button
                            onClick={(e) => { stopEvent(e); onOpenMemory?.(agent); }}
                            size="sm"
                            variant="ghost"
                          >
                            <IconDatabase aria-hidden size={15} stroke={1.8} />
                            {t('Memory')}
                          </Button>
                        ) : null}
                        <Button
                          onClick={(e) => { stopEvent(e); openCreateTeamDialog(agent.agentId); }}
                          size="sm"
                          variant="ghost"
                        >
                          {t('Team')}
                        </Button>
                        {!agent.builtIn ? (
                          <Button
                            disabled={saving}
                            onClick={(e) => { stopEvent(e); void handleDeleteAgent(agent); }}
                            size="sm"
                            variant="ghost"
                            className="text-destructive"
                          >
                            {t('Delete')}
                          </Button>
                        ) : null}
                      </div>
                    </TableCell>
                  </TableRow>
                ))
              ) : search.trim() ? (
                <TableRow>
                  <TableCell className="text-center text-muted-foreground" colSpan={4}>
                    {t('No agents matching "{query}"', { query: search.trim() })}
                  </TableCell>
                </TableRow>
              ) : null
            ) : showingTeams ? (
              visibleTeams.length ? (
                visibleTeams.map((team) => {
                  const leaderLabel = agentMap.get(team.leaderAgentId)?.displayName || team.leaderAgentId;
                  return (
                    <TableRow
                      className="cursor-pointer"
                      key={team.teamId}
                      onClick={() => openViewTeamDialog(team)}
                    >
                      <TableCell>
                        <div className="agents-hub-name-cell">
                          <AgentAvatar
                            avatarDataUrl={team.avatarDataUrl}
                            className="agents-hub-avatar-sm"
                            label={team.displayName || team.teamId}
                            team
                          />
                          <div>
                            <div className="agents-hub-cell-name">{team.displayName}</div>
                            <div className="agents-hub-cell-id">{team.teamId}</div>
                          </div>
                        </div>
                      </TableCell>
                      <TableCell>{leaderLabel}</TableCell>
                      <TableCell>
                        {team.memberAgentIds
                          .slice(0, 3)
                          .map((id) => agentMap.get(id)?.displayName || id)
                          .join(', ')}
                        {team.memberAgentIds.length > 3 ? ` +${team.memberAgentIds.length - 3}` : ''}
                      </TableCell>
                      <TableCell className="text-right">
                        <div className="agents-hub-row-actions">
                          <Button
                            onClick={(e) => { stopEvent(e); onStartThread?.(team.teamId); }}
                            size="sm"
                            variant="outline"
                          >
                            {t('Chat')}
                          </Button>
                          <Button
                            onClick={(e) => { stopEvent(e); openEditTeamDialog(team); }}
                            size="sm"
                            variant="ghost"
                          >
                            {t('Edit')}
                          </Button>
                          <Button
                            disabled={saving}
                            onClick={(e) => { stopEvent(e); void handleDeleteTeam(team); }}
                            size="sm"
                            variant="ghost"
                            className="text-destructive"
                          >
                            {t('Delete')}
                          </Button>
                        </div>
                      </TableCell>
                    </TableRow>
                  );
                })
              ) : search.trim() ? (
                <TableRow>
                  <TableCell className="text-center text-muted-foreground" colSpan={4}>
                    {t('No teams matching "{query}"', { query: search.trim() })}
                  </TableCell>
                </TableRow>
              ) : null
            ) : (
              visibleWorkflows.length ? (
                visibleWorkflows.map((workflow) => {
                  const workspace = workflowDefaultWorkspace(workflow);
                  return (
                    <TableRow
                      className="cursor-pointer"
                      key={workflow.workflowId}
                      onClick={() => openViewWorkflowDialog(workflow)}
                    >
                      <TableCell>
                        <div className="agents-hub-name-cell">
                          <span className="agents-hub-avatar-sm workflow">WF</span>
                          <div>
                            <div className="agents-hub-cell-name">{workflow.name}</div>
                            <div className="agents-hub-cell-id">{workflow.workflowId}</div>
                            {workflow.description ? (
                              <div className="agents-hub-cell-description">
                                {previewText(workflow.description, '')}
                              </div>
                            ) : null}
                          </div>
                        </div>
                      </TableCell>
                      <TableCell>
                        <Badge variant="outline">v{workflow.version}</Badge>
                      </TableCell>
                      <TableCell>
                        <span className="agents-hub-cell-id">
                          {workspace || t('Task workspace')}
                        </span>
                      </TableCell>
                      <TableCell className="text-right">
                        <span
                          className="agents-hub-cell-id agents-hub-package-path"
                          title={workflow.packageDir || undefined}
                        >
                          {workflow.packageDir ? t('File package') : t('Installed')}
                        </span>
                      </TableCell>
                    </TableRow>
                  );
                })
              ) : search.trim() ? (
                <TableRow>
                  <TableCell className="text-center text-muted-foreground" colSpan={4}>
                    {t('No workflows matching "{query}"', { query: search.trim() })}
                  </TableCell>
                </TableRow>
              ) : (
                <TableRow>
                  <TableCell className="text-center text-muted-foreground" colSpan={4}>
                    {t('No workflow definitions installed')}
                    <div className="agents-hub-install-hint">
                      <code>garyx workflow definition upsert --file &lt;path&gt;</code>
                    </div>
                  </TableCell>
                </TableRow>
              )
            )}
          </TableBody>
        </Table>
      )}

      <Dialog
        open={Boolean(agentDialogMode)}
        onOpenChange={(open) => {
          if (!open) {
            closeAgentDialog();
          }
        }}
      >
        <DialogContent className="agents-hub-agent-dialog" size="form">
          <DialogHeader className="agents-hub-dialog-header">
            <DialogDescription className="agents-hub-dialog-kicker">
              {t('Agent')}
            </DialogDescription>
            <DialogTitle className="agents-hub-dialog-title">
              {agentDialogMode === 'create'
                ? t('Create agent')
                : agentDialogMode === 'edit'
                  ? t('Edit agent')
                  : selectedAgent?.displayName || t('Agent')}
            </DialogTitle>
            <DialogDescription className="agents-hub-dialog-description">
              {agentDialogMode === 'create'
                ? t('Create a reusable agent identity with its own provider and system prompt.')
                : t('Inspect or adjust how this agent shows up in the desktop app.')}
            </DialogDescription>
          </DialogHeader>

          {agentDialogMode === 'create' || agentDialogMode === 'edit' ? (
            <form className="agents-hub-dialog-form" onSubmit={handleAgentSubmit}>
              <div className="agents-hub-avatar-editor">
                <AgentAvatar
                  agentId={agentDialogMode === 'edit' ? agentDraft.agentId : undefined}
                  avatarDataUrl={agentDraft.avatarDataUrl}
                  builtIn={agentDialogMode === 'edit' ? selectedAgent?.builtIn : undefined}
                  className="agents-hub-avatar-centered large"
                  label={agentDraft.displayName || agentDraft.agentId || 'A'}
                  providerType={agentDraft.providerType}
                />
                <div className="agents-hub-avatar-editor-actions">
                  <input
                    accept={AGENT_AVATAR_ACCEPT}
                    className="sr-only"
                    onChange={(event) => {
                      void handleAvatarFileChange(event, 'agent');
                    }}
                    ref={avatarFileInputRef}
                    type="file"
                  />
                  <Button
                    onClick={() => avatarFileInputRef.current?.click()}
                    type="button"
                    variant="outline"
                  >
                    <IconUpload aria-hidden size={15} stroke={1.8} />
                    {t('Upload avatar')}
                  </Button>
                  <Button
                    disabled={avatarGenerating}
                    onClick={() => {
                      setAvatarStyleTarget('agent');
                      setAvatarStyleDialogOpen(true);
                    }}
                    type="button"
                    variant="outline"
                  >
                    <IconSparkles aria-hidden size={15} stroke={1.8} />
                    {avatarGenerating ? t('Generating...') : t('Generate avatar')}
                  </Button>
                  {agentDraft.avatarDataUrl ? (
                    <Button
                      onClick={() => {
                        setAgentDraft((current) => ({ ...current, avatarDataUrl: '' }));
                      }}
                      type="button"
                      variant="ghost"
                    >
                      {t('Clear')}
                    </Button>
                  ) : null}
                </div>
              </div>

              <div className="agents-hub-form-grid">
                <div className="codex-form-field">
                  <Label className="codex-form-label" htmlFor="agent-dialog-name">{t('Name')}</Label>
                  <Input
                    id="agent-dialog-name"
                    onChange={(event) => {
                      setAgentDraft((current) => ({ ...current, displayName: event.target.value }));
                    }}
                    value={agentDraft.displayName}
                  />
                </div>
                <div className="codex-form-field">
                  <Label className="codex-form-label" htmlFor="agent-dialog-id">{t('Agent ID')}</Label>
                  <Input
                    disabled={agentDialogMode === 'edit'}
                    id="agent-dialog-id"
                    onChange={(event) => {
                      setAgentIdTouched(true);
                      setAgentDraft((current) => ({ ...current, agentId: event.target.value }));
                    }}
                    value={agentDraft.agentId}
                  />
                </div>
              </div>

              <div className="codex-form-field">
                <Label className="codex-form-label">{t('Provider')}</Label>
                <Select
                  onValueChange={(value: ProviderType) => {
                    setAgentDraft((current) => ({
                      ...current,
                      providerType: value,
                      model: '',
                      modelReasoningEffort: value === 'codex_app_server' || isNativeModelProvider(value)
                        ? current.modelReasoningEffort
                        : '',
                      modelServiceTier: value === 'gpt' ? current.modelServiceTier : '',
                      authSource: isNativeModelProvider(value) ? defaultAuthSource(value) : '',
                      apiKey: '',
                      baseUrl: '',
                    }));
                    void ensureProviderModels(value);
                  }}
                  value={agentDraft.providerType}
                >
                  <SelectTrigger>
                    <SelectValue placeholder={t('Select provider')} />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectGroup>
                      <SelectItem value="claude_code">Claude</SelectItem>
                      <SelectItem value="codex_app_server">Codex</SelectItem>
                      <SelectItem value="gemini_cli">Gemini</SelectItem>
                      <SelectItem value="gpt">GPT</SelectItem>
                      <SelectItem value="anthropic">Claude</SelectItem>
                      <SelectItem value="google">Gemini</SelectItem>
                    </SelectGroup>
                  </SelectContent>
                </Select>
                {agentModelStatus ? <span className="codex-form-hint">{agentModelStatus}</span> : null}
              </div>

              {isNativeModelProvider(agentDraft.providerType) ? (
                <>
                  {agentDraft.providerType === 'gpt' ? (
                    <div className="codex-form-field">
                      <Label className="codex-form-label">{t('GPT auth')}</Label>
                      <Select
                        onValueChange={(value) => {
                          setAgentDraft((current) => ({
                            ...current,
                            authSource: value,
                            apiKey: value === 'codex' ? '' : current.apiKey,
                          }));
                        }}
                        value={agentDraft.authSource || 'codex'}
                      >
                        <SelectTrigger className="agents-hub-model-select">
                          <SelectValue placeholder={t('Select auth')} />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectGroup>
                            <SelectItem value="codex">{t('Use GPT token')}</SelectItem>
                            <SelectItem value="api_key">{t('Use API key')}</SelectItem>
                          </SelectGroup>
                        </SelectContent>
                      </Select>
                    </div>
                  ) : null}

                  {agentDraft.providerType !== 'gpt' || agentDraft.authSource === 'api_key' ? (
                    <div className="codex-form-field">
                      <Label className="codex-form-label" htmlFor="agent-dialog-api-key">
                        {t('API Key')}
                      </Label>
                      <Input
                        autoCapitalize="off"
                        autoComplete="off"
                        id="agent-dialog-api-key"
                        onChange={(event) => {
                          setAgentDraft((current) => ({ ...current, apiKey: event.target.value }));
                        }}
                        placeholder={
                          agentDraft.providerType === 'anthropic' || agentDraft.providerType === 'claude_llm'
                            ? 'ANTHROPIC_API_KEY'
                            : agentDraft.providerType === 'google' || agentDraft.providerType === 'gemini_llm'
                              ? 'GEMINI_API_KEY'
                              : 'OPENAI_API_KEY'
                        }
                        spellCheck={false}
                        type="password"
                        value={agentDraft.apiKey}
                      />
                      <span className="codex-form-hint">
                        {t('Stored on this custom provider config.')}
                      </span>
                    </div>
                  ) : null}

                  <div className="codex-form-field">
                    <Label className="codex-form-label" htmlFor="agent-dialog-base-url">
                      {t('Base URL')}
                    </Label>
                    <Input
                      autoCapitalize="off"
                      autoComplete="off"
                      id="agent-dialog-base-url"
                      onChange={(event) => {
                        setAgentDraft((current) => ({ ...current, baseUrl: event.target.value }));
                      }}
                      placeholder={t('(provider default)')}
                      spellCheck={false}
                      value={agentDraft.baseUrl}
                    />
                  </div>
                </>
              ) : null}

              {agentSupportsModelSelection ? (
                <div className="codex-form-field">
                  <Label className="codex-form-label" htmlFor="agent-dialog-model">{t('Model')}</Label>
                  <Select
                    onValueChange={(value) => {
                      setAgentDraft((current) => ({
                        ...current,
                        model: value === PROVIDER_DEFAULT_MODEL_VALUE ? '' : value,
                        modelServiceTier: '',
                      }));
                    }}
                    value={agentDraft.model.trim() || PROVIDER_DEFAULT_MODEL_VALUE}
                  >
                    <SelectTrigger className="agents-hub-model-select" id="agent-dialog-model">
                      <SelectValue placeholder={t('Select model')} />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        <SelectItem value={PROVIDER_DEFAULT_MODEL_VALUE}>{t('Provider default')}</SelectItem>
                        {agentModelOptions.map((model) => (
                          <SelectItem key={model.id} value={model.id}>
                            {model.label}
                          </SelectItem>
                        ))}
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                  <span className="codex-form-hint">
                    {activeAgentProviderModels?.defaultModel
                      ? t('Gateway default: {model}', { model: activeAgentProviderModels.defaultModel })
                      : t('Optional. Leave empty to use the provider default.')}
                  </span>
                </div>
              ) : null}

              {agentSupportsReasoningEffortSelection ? (
                <div className="codex-form-field">
                  <Label className="codex-form-label" htmlFor="agent-dialog-reasoning-effort">{t('Reasoning effort')}</Label>
                  <Select
                    onValueChange={(value) => {
                      setAgentDraft((current) => ({
                        ...current,
                        modelReasoningEffort: value === PROVIDER_DEFAULT_REASONING_VALUE ? '' : value,
                      }));
                    }}
                    value={agentDraft.modelReasoningEffort.trim() || PROVIDER_DEFAULT_REASONING_VALUE}
                  >
                    <SelectTrigger className="agents-hub-model-select" id="agent-dialog-reasoning-effort">
                      <SelectValue placeholder={t('Select reasoning effort')} />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        <SelectItem value={PROVIDER_DEFAULT_REASONING_VALUE}>{t('Provider default')}</SelectItem>
                        {agentReasoningEffortOptions.map((effort) => (
                          <SelectItem key={effort.id} value={effort.id}>
                            {effort.label}
                          </SelectItem>
                        ))}
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                  <span className="codex-form-hint">
                    {t('Lower values are faster; higher values spend more reasoning.')}
                  </span>
                </div>
              ) : null}

              {agentSupportsServiceTierSelection ? (
                <div className="codex-form-field">
                  <Label className="codex-form-label" htmlFor="agent-dialog-service-tier">{t('Service tier')}</Label>
                  <Select
                    onValueChange={(value) => {
                      setAgentDraft((current) => ({
                        ...current,
                        modelServiceTier: value === PROVIDER_DEFAULT_SERVICE_TIER_VALUE ? '' : value,
                      }));
                    }}
                    value={agentDraft.modelServiceTier.trim() || PROVIDER_DEFAULT_SERVICE_TIER_VALUE}
                  >
                    <SelectTrigger className="agents-hub-model-select" id="agent-dialog-service-tier">
                      <SelectValue placeholder={t('Select service tier')} />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        <SelectItem value={PROVIDER_DEFAULT_SERVICE_TIER_VALUE}>{t('Provider default')}</SelectItem>
                        {agentServiceTierOptions.map((tier) => (
                          <SelectItem key={tier.id} value={tier.id}>
                            {tier.label}
                          </SelectItem>
                        ))}
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                  <span className="codex-form-hint">
                    {t('Fast service tiers trade higher usage for lower latency when the model supports them.')}
                  </span>
                </div>
              ) : null}

              <div className="codex-form-field">
                <Label className="codex-form-label" htmlFor="agent-dialog-default-workspace">
                  {t('Default workspace directory')}
                </Label>
                <WorkspacePathPicker
                  id="agent-dialog-default-workspace"
                  onChange={(value) => {
                    setAgentDraft((current) => ({ ...current, defaultWorkspaceDir: value }));
                  }}
                  onAddWorkspace={onAddWorkspace}
                  placeholder={t('/path/to/project')}
                  value={agentDraft.defaultWorkspaceDir}
                  workspaces={workspaces}
                />
                <span className="codex-form-hint">
                  {t('Used when a new bot or task thread has no explicit workspace.')}
                </span>
              </div>

              <div className="codex-form-field">
                <Label className="codex-form-label" htmlFor="agent-dialog-prompt">{t('System Prompt')}</Label>
                <Textarea
                  className="agents-hub-system-prompt"
                  id="agent-dialog-prompt"
                  onChange={(event) => {
                    setAgentDraft((current) => ({ ...current, systemPrompt: event.target.value }));
                  }}
                  value={agentDraft.systemPrompt}
                />
              </div>

              <DialogFooter className="agents-hub-dialog-footer">
                <div className="agents-hub-dialog-status">{agentValidationError}</div>
                <div className="agents-hub-dialog-actions">
                  <Button
                    disabled={saving}
                    onClick={closeAgentDialog}
                    type="button"
                    variant="outline"
                  >
                    {t('Cancel')}
                  </Button>
                  <Button disabled={Boolean(agentValidationError) || saving} type="submit">
                    {saving ? t('Saving...') : agentDialogMode === 'create' ? t('Create Agent') : t('Save Agent')}
                  </Button>
                </div>
              </DialogFooter>
            </form>
          ) : (
            <div className="agents-hub-dialog-stack">
              <div className="agents-hub-detail-header">
                <AgentAvatar
                  agentId={selectedAgent?.agentId}
                  avatarDataUrl={selectedAgent?.avatarDataUrl}
                  builtIn={selectedAgent?.builtIn}
                  className="agents-hub-avatar-centered large"
                  label={selectedAgent?.displayName || selectedAgent?.agentId || 'A'}
                  providerIcon={selectedAgent?.providerIcon}
                  providerType={selectedAgent?.providerType}
                />
                <div className="agents-hub-detail-copy">
                <div className="agents-hub-card-badges">
                  <Badge variant="outline">{selectedAgent?.builtIn ? t('Built-in') : t('Custom')}</Badge>
                  {selectedAgent ? <Badge variant="outline">{providerLabel(selectedAgent.providerType)}</Badge> : null}
                </div>
                <h3>{selectedAgent?.displayName || t('Agent')}</h3>
                <p>{selectedAgent?.agentId || ''}</p>
                {selectedAgent && (selectedAgent.providerType === 'gemini_cli' || isNativeModelProvider(selectedAgent.providerType as ProviderType) || selectedAgent.model.trim()) ? (
                  <p>{selectedAgent.model || t('(provider default)')}</p>
                ) : null}
                {selectedAgent && isNativeModelProvider(selectedAgent.providerType as ProviderType) ? (
                  <p>{t('Auth')}: {selectedAgent.authSource || defaultAuthSource(selectedAgent.providerType as ProviderType)}</p>
                ) : null}
                {selectedAgent?.modelReasoningEffort.trim() ? (
                  <p>{t('Reasoning effort')}: {selectedAgent.modelReasoningEffort}</p>
                ) : null}
                {selectedAgent?.modelServiceTier.trim() ? (
                  <p>{t('Service tier')}: {selectedAgent.modelServiceTier}</p>
                ) : null}
                {selectedAgent?.defaultWorkspaceDir.trim() ? (
                  <p>{selectedAgent.defaultWorkspaceDir.trim()}</p>
                ) : null}
              </div>
              </div>

              <div className="agents-hub-detail-block">
                <div className="agents-hub-detail-label">{t('Default workspace directory')}</div>
                <div className="agents-hub-detail-body mono">
                  {selectedAgent?.defaultWorkspaceDir.trim() || t('(not set)')}
                </div>
              </div>

              <div className="agents-hub-detail-block">
                <div className="agents-hub-detail-label">{t('System Prompt')}</div>
                <div className="agents-hub-detail-body mono">
                  {selectedAgent?.systemPrompt || t('(empty)')}
                </div>
              </div>

              <DialogFooter className="agents-hub-dialog-actions">
                {selectedAgent ? (
                  <Button
                    onClick={() => {
                      closeAgentDialog();
                      onStartThread?.(selectedAgent.agentId);
                    }}
                    type="button"
                    variant="outline"
                  >
                    {t('Chat')}
                  </Button>
                ) : null}
                {selectedAgent ? (
                  <Button
                    onClick={() => {
                      closeAgentDialog();
                      openCreateTeamDialog(selectedAgent.agentId);
                    }}
                    type="button"
                    variant="outline"
                  >
                    {t('Create Team')}
                  </Button>
                ) : null}
                {selectedAgent && !selectedAgent.builtIn ? (
                  <Button
                    onClick={() => {
                      closeAgentDialog();
                      onOpenMemory?.(selectedAgent);
                    }}
                    type="button"
                    variant="outline"
                  >
                    <IconDatabase aria-hidden size={15} stroke={1.8} />
                    {t('Memory')}
                  </Button>
                ) : null}
                {selectedAgent && !selectedAgent.builtIn ? (
                  <Button
                    onClick={() => {
                      openEditAgentDialog(selectedAgent);
                    }}
                    type="button"
                  >
                    {t('Edit Agent')}
                  </Button>
                ) : null}
              </DialogFooter>
            </div>
          )}
        </DialogContent>
      </Dialog>

      <Dialog open={avatarStyleDialogOpen} onOpenChange={setAvatarStyleDialogOpen}>
        <DialogContent className="agents-hub-avatar-style-dialog" size="compact">
          <DialogHeader>
            <DialogTitle>{t('Avatar style')}</DialogTitle>
          </DialogHeader>

          <div className="agents-hub-avatar-style-grid">
            {AVATAR_STYLE_OPTIONS.map((option) => (
              <button
                className={`agents-hub-avatar-style-option ${avatarStyleId === option.id ? 'active' : ''}`}
                key={option.id}
                onClick={() => {
                  setAvatarStyleId(option.id);
                }}
                type="button"
              >
                {t(option.label)}
              </button>
            ))}
            <button
              className={`agents-hub-avatar-style-option ${avatarStyleId === CUSTOM_AVATAR_STYLE_ID ? 'active' : ''}`}
              onClick={() => {
                setAvatarStyleId(CUSTOM_AVATAR_STYLE_ID);
              }}
              type="button"
            >
              {t('Custom style')}
            </button>
          </div>

          {avatarStyleId === CUSTOM_AVATAR_STYLE_ID ? (
            <div className="codex-form-field">
              <Label className="codex-form-label" htmlFor="agent-avatar-custom-style">
                {t('Custom style')}
              </Label>
              <Textarea
                className="agents-hub-avatar-style-textarea"
                id="agent-avatar-custom-style"
                onChange={(event) => {
                  setCustomAvatarStyle(event.target.value);
                }}
                placeholder={t('e.g. polished paper-cut icon with emerald accents')}
                value={customAvatarStyle}
              />
            </div>
          ) : null}

          <DialogFooter className="agents-hub-dialog-footer">
            <div className="agents-hub-dialog-status">{avatarStyleValidationError}</div>
            <div className="agents-hub-dialog-actions">
              <Button
                disabled={avatarGenerating}
                onClick={() => {
                  setAvatarStyleDialogOpen(false);
                }}
                type="button"
                variant="outline"
              >
                {t('Cancel')}
              </Button>
              <Button
                disabled={Boolean(avatarStyleValidationError) || avatarGenerating}
                onClick={() => {
                  void handleGenerateAvatar(activeAvatarStylePrompt);
                }}
                type="button"
              >
                {avatarGenerating ? t('Generating...') : t('Generate avatar')}
              </Button>
            </div>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={Boolean(teamDialogMode)}
        onOpenChange={(open) => {
          if (!open) {
            closeTeamDialog();
          }
        }}
      >
        <DialogContent className="sm:max-w-[720px] team-builder-dialog" size="wide">
          <DialogHeader>
            <DialogTitle>
              {teamDialogMode === 'create'
                ? t('Create team')
                : teamDialogMode === 'edit'
                  ? t('Edit team')
                  : selectedTeam?.displayName || t('Team')}
            </DialogTitle>
            <DialogDescription>
              {t('Set team name and add at least one agent. You can set one as TL (Team Lead).')}
            </DialogDescription>
          </DialogHeader>

          {teamDialogMode === 'create' || teamDialogMode === 'edit' ? (
            <form className="agents-hub-dialog-form" onSubmit={handleTeamSubmit}>
              <div className="agents-hub-avatar-editor team-builder-avatar-editor">
                <AgentAvatar
                  avatarDataUrl={teamDraft.avatarDataUrl}
                  className="agents-hub-avatar-centered large"
                  label={teamDraft.displayName || teamDraft.teamId || 'T'}
                  team
                />
                <div className="agents-hub-avatar-editor-actions">
                  <input
                    accept={AGENT_AVATAR_ACCEPT}
                    className="sr-only"
                    onChange={(event) => {
                      void handleAvatarFileChange(event, 'team');
                    }}
                    ref={teamAvatarFileInputRef}
                    type="file"
                  />
                  <Button
                    onClick={() => teamAvatarFileInputRef.current?.click()}
                    type="button"
                    variant="outline"
                  >
                    <IconUpload aria-hidden size={15} stroke={1.8} />
                    {t('Upload avatar')}
                  </Button>
                  <Button
                    disabled={avatarGenerating}
                    onClick={() => {
                      setAvatarStyleTarget('team');
                      setAvatarStyleDialogOpen(true);
                    }}
                    type="button"
                    variant="outline"
                  >
                    <IconSparkles aria-hidden size={15} stroke={1.8} />
                    {avatarGenerating ? t('Generating...') : t('Generate avatar')}
                  </Button>
                  {teamDraft.avatarDataUrl ? (
                    <Button
                      onClick={() => {
                        setTeamDraft((current) => ({ ...current, avatarDataUrl: '' }));
                      }}
                      type="button"
                      variant="ghost"
                    >
                      {t('Clear')}
                    </Button>
                  ) : null}
                </div>
              </div>
              <div className="team-builder-form-grid">
                <div className="codex-form-field">
                  <Label className="codex-form-label" htmlFor="team-dialog-name">{t('Team name')}</Label>
                  <Input
                    id="team-dialog-name"
                    onChange={(event) => {
                      setTeamDraft((current) => ({ ...current, displayName: event.target.value }));
                    }}
                    placeholder={t('e.g. dev-team')}
                    value={teamDraft.displayName}
                  />
                </div>
                <div className="codex-form-field">
                  <Label className="codex-form-label" htmlFor="team-dialog-workflow">{t('Workflow')}</Label>
                  <Textarea
                    id="team-dialog-workflow"
                    onChange={(event) => {
                      setTeamDraft((current) => ({ ...current, workflowText: event.target.value }));
                    }}
                    placeholder={t('How should the team coordinate? (optional)')}
                    rows={2}
                    value={teamDraft.workflowText}
                  />
                </div>
              </div>

              <div className="team-builder-body">
                <div className="team-builder-left">
                  <div className="team-builder-agent-count">{t('ALL AGENTS')} ({agents.length})</div>
                  <div className="team-builder-agent-list">
                    {agents.map((agent) => {
                      const selected = teamDraft.memberAgentIds.includes(agent.agentId);
                      return (
                        <button
                          className={`team-builder-agent-row ${selected ? 'selected' : ''}`}
                          key={agent.agentId}
                          onClick={() => {
                            toggleTeamMember(agent.agentId);
                          }}
                          type="button"
                        >
                          <AgentAvatar
                            agentId={agent.agentId}
                            avatarDataUrl={agent.avatarDataUrl}
                            builtIn={agent.builtIn}
                            className="agents-hub-avatar-centered small"
                            label={agent.displayName || agent.agentId}
                            providerIcon={agent.providerIcon}
                            providerType={agent.providerType}
                          />
                          <div className="team-builder-agent-info">
                            <span className="team-builder-agent-name">{agent.displayName}</span>
                            <span className="team-builder-agent-desc">
                              {previewText(agent.systemPrompt, agent.builtIn ? t('Built-in agent') : t('Custom agent'))}
                            </span>
                          </div>
                          <span className={`team-builder-toggle-btn ${selected ? 'checked' : ''}`}>
                            {selected
                              ? <IconCheck aria-hidden size={14} stroke={2.5} />
                              : <IconPlus aria-hidden size={14} stroke={2} />}
                          </span>
                        </button>
                      );
                    })}
                    {!agents.length ? (
                      <div className="codex-empty-state">{t('No agents available yet.')}</div>
                    ) : null}
                  </div>
                </div>

                <div className="team-builder-right">
                  <div className="team-builder-agent-count">{t('SELECTED MEMBERS')} ({teamDraft.memberAgentIds.length} / {agents.length})</div>
                  <div className="team-builder-selected-list">
                    {teamDraft.memberAgentIds.map((agentId) => {
                      const agent = agentMap.get(agentId);
                      const isLeader = teamDraft.leaderAgentId === agentId;
                      return (
                        <div className="team-builder-selected-row" key={agentId}>
                          <AgentAvatar
                            agentId={agent?.agentId || agentId}
                            avatarDataUrl={agent?.avatarDataUrl}
                            builtIn={agent?.builtIn}
                            className="agents-hub-avatar-centered small"
                            label={agent?.displayName || agentId}
                            providerIcon={agent?.providerIcon}
                            providerType={agent?.providerType}
                          />
                          <div className="team-builder-agent-info">
                            <span className="team-builder-agent-name">{agent?.displayName || agentId}</span>
                            <span className="team-builder-agent-desc">
                              {previewText(agent?.systemPrompt || '', t('Agent'))}
                            </span>
                          </div>
                          <div className="team-builder-selected-actions">
                            {isLeader ? (
                              <span className="team-builder-tl-badge active">TL</span>
                            ) : (
                              <button
                                className="team-builder-tl-badge"
                                onClick={() => {
                                  selectTeamLeader(agentId);
                                }}
                                title={t('Set as Team Lead')}
                                type="button"
                              >
                                TL
                              </button>
                            )}
                            <button
                              className="team-builder-remove-btn"
                              onClick={() => {
                                toggleTeamMember(agentId);
                              }}
                              title={t('Remove')}
                              type="button"
                            >
                              <IconX aria-hidden size={14} stroke={2} />
                            </button>
                          </div>
                        </div>
                      );
                    })}
                    {!teamDraft.memberAgentIds.length ? (
                      <div style={{ padding: '32px 16px', color: 'var(--color-token-description-foreground)', fontSize: 'var(--text-sm)', textAlign: 'center' }}>
                        {t('Select agents from the left to add them.')}
                      </div>
                    ) : null}
                  </div>
                </div>
              </div>

              <DialogFooter className="team-builder-footer">
                {teamValidationError ? (
                  <span className="agents-hub-dialog-status" style={{ marginRight: 'auto' }}>{teamValidationError}</span>
                ) : null}
                <Button
                  disabled={saving}
                  onClick={closeTeamDialog}
                  type="button"
                  variant="outline"
                >
                  {t('Cancel')}
                </Button>
                <Button className="team-builder-create-team-btn" disabled={Boolean(teamValidationError) || saving} type="submit">
                  {saving ? t('Saving...') : teamDialogMode === 'create' ? t('Create team') : t('Save team')}
                </Button>
              </DialogFooter>
            </form>
          ) : (
            <div className="agents-hub-dialog-stack">
              <div className="agents-hub-detail-header">
                <AgentAvatar
                  avatarDataUrl={selectedTeam?.avatarDataUrl}
                  className="agents-hub-avatar-centered large"
                  label={selectedTeam?.displayName || selectedTeam?.teamId || 'T'}
                  team
                />
                <div className="agents-hub-detail-copy">
                  <div className="agents-hub-card-badges">
                    <Badge variant="outline">
                      {t('Lead')}: {agentMap.get(selectedTeam?.leaderAgentId || '')?.displayName || selectedTeam?.leaderAgentId}
                    </Badge>
                    <Badge variant="outline">{t('{count} members', { count: selectedTeam?.memberAgentIds.length || 0 })}</Badge>
                  </div>
                  <h3>{selectedTeam?.displayName || t('Team')}</h3>
                  <p>{selectedTeam?.teamId || ''}</p>
                </div>
              </div>

              <div className="agents-hub-detail-block">
                <div className="agents-hub-detail-label">{t('Members')}</div>
                <div className="agents-hub-chip-list">
                  {(selectedTeam?.memberAgentIds || []).map((agentId) => (
                    <Badge key={agentId} variant="outline">
                      {agentMap.get(agentId)?.displayName || agentId}
                    </Badge>
                  ))}
                </div>
              </div>

              <div className="agents-hub-detail-block">
                <div className="agents-hub-detail-label">{t('Workflow')}</div>
                <div className="agents-hub-detail-body">{selectedTeam?.workflowText || t('(empty)')}</div>
              </div>

              <DialogFooter className="agents-hub-dialog-actions">
                {selectedTeam ? (
                  <Button
                    onClick={() => {
                      onStartThread?.(selectedTeam.teamId);
                    }}
                    type="button"
                    variant="outline"
                  >
                    {t('Chat')}
                  </Button>
                ) : null}
                {selectedTeam ? (
                  <Button
                    onClick={() => {
                      openEditTeamDialog(selectedTeam);
                    }}
                    type="button"
                  >
                    {t('Edit Team')}
                  </Button>
                ) : null}
              </DialogFooter>
            </div>
          )}
        </DialogContent>
      </Dialog>

      <Dialog
        open={Boolean(workflowDialogMode)}
        onOpenChange={(open) => {
          if (!open) {
            closeWorkflowDialog();
          }
        }}
      >
        <DialogContent className="agents-hub-agent-dialog agents-hub-workflow-dialog" size="viewer">
          <DialogHeader className="agents-hub-dialog-header">
            <DialogDescription className="agents-hub-dialog-kicker">
              {t('Workflow')}
            </DialogDescription>
            <DialogTitle className="agents-hub-dialog-title">
              {selectedWorkflow?.name || t('Workflow definition')}
            </DialogTitle>
            <DialogDescription className="agents-hub-dialog-description">
              {selectedWorkflow?.description || t('File-backed workflow definition installed for task execution.')}
            </DialogDescription>
          </DialogHeader>

          <div className="agents-hub-dialog-stack">
            <div className="agents-hub-workflow-meta-strip">
              <Badge variant="outline">v{selectedWorkflow?.version || 1}</Badge>
              <span>{selectedWorkflow?.workflowId || ''}</span>
            </div>

            <div className="agents-hub-workflow-source">
              <div className="agents-hub-workflow-source-header">
                <div>
                  <div className="agents-hub-detail-label">{t('Source')}</div>
                  <div className="agents-hub-workflow-source-path">
                    {workflowSource?.path || './workflow.ts'}
                  </div>
                </div>
                <div className="agents-hub-card-badges">
                  {workflowSource?.language ? <Badge variant="outline">{workflowSource.language}</Badge> : null}
                  {workflowSource?.mediaType ? <Badge variant="outline">{workflowSource.mediaType}</Badge> : null}
                </div>
              </div>
              <pre className={`agents-hub-workflow-code ${workflowSourceError ? 'error' : ''}`}>
                <code>
                  {workflowSourceLoading
                    ? t('Loading source...')
                    : workflowSourceError
                      ? workflowSourceError
                      : workflowSource?.content || t('(empty)')}
                </code>
              </pre>
            </div>

            <div className="agents-hub-workflow-footnote">
              <span>
                {t('Input')}: {selectedWorkflow
                  ? workflowInputPlaceholder(selectedWorkflow) || t('Plain text input')
                  : t('Plain text input')}
              </span>
              <Button
                disabled={workflowSourceLoading}
                onClick={() => {
                  if (selectedWorkflow) {
                    void loadWorkflowSource(selectedWorkflow.workflowId);
                  }
                }}
                size="sm"
                type="button"
                variant="ghost"
              >
                {t('Refresh')}
              </Button>
              <Button
                onClick={closeWorkflowDialog}
                size="sm"
                type="button"
                variant="outline"
              >
                {t('Close')}
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}
