import type {
  DesktopCustomAgent,
  DesktopProviderModels,
} from '@shared/contracts';

export type ProviderType = 'claude_code' | 'codex_app_server' | 'antigravity' | 'traex';
export type AgentDialogMode = 'create' | 'edit' | 'view' | null;
export type AvatarStyleId = 'clean_glyph' | 'soft_3d' | 'glass_icon' | 'pixel_badge' | 'ink_line' | 'paper_cut' | 'blueprint' | 'enamel_sticker' | 'custom';

export type AgentDraft = {
  agentId: string;
  displayName: string;
  providerType: ProviderType;
  model: string;
  modelReasoningEffort: string;
  modelServiceTier: string;
  defaultWorkspaceDir: string;
  avatarDataUrl: string;
  env: Array<{ key: string; value: string }>;
  systemPrompt: string;
};

export const PROVIDER_DEFAULT_MODEL_VALUE = '__provider_default__';
export const PROVIDER_DEFAULT_REASONING_VALUE = '__provider_default_reasoning__';
export const PROVIDER_DEFAULT_SERVICE_TIER_VALUE = '__provider_default_service_tier__';
export const AGENT_AVATAR_MAX_BYTES = 3 * 1024 * 1024;
export const AGENT_AVATAR_SIZE = 256;
export const AGENT_AVATAR_DATA_URL_MAX_LENGTH = 700_000;
export const AGENT_AVATAR_ACCEPT = 'image/png,image/jpeg,image/webp,image/svg+xml';
export const CUSTOM_AVATAR_STYLE_ID: AvatarStyleId = 'custom';
export const DEFAULT_AVATAR_STYLE_ID: AvatarStyleId = 'clean_glyph';
export const FALLBACK_REASONING_EFFORTS = [
  { id: 'none', label: 'None', description: 'No reasoning', recommended: false },
  { id: 'minimal', label: 'Minimal', description: 'Minimal reasoning', recommended: false },
  { id: 'low', label: 'Low', description: 'Faster responses', recommended: false },
  { id: 'medium', label: 'Medium', description: 'Balanced speed and reasoning', recommended: true },
  { id: 'high', label: 'High', description: 'Deeper reasoning', recommended: false },
  { id: 'xhigh', label: 'Extra High', description: 'Maximum reasoning', recommended: false },
];

export const AVATAR_STYLE_OPTIONS: Array<{
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

export function emptyAgentDraft(): AgentDraft {
  return {
    agentId: '',
    displayName: '',
    providerType: 'claude_code',
    model: '',
    modelReasoningEffort: '',
    modelServiceTier: '',
    defaultWorkspaceDir: '',
    avatarDataUrl: '',
    env: [],
    systemPrompt: '',
  };
}

export function deriveId(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .replace(/-{2,}/g, '-');
}

export function providerLabel(value: ProviderType): string {
  if (value === 'codex_app_server') {
    return 'Codex';
  }
  if (value === 'antigravity') {
    return 'Antigravity';
  }
  if (value === 'traex') {
    return 'Traex';
  }
  return 'Claude';
}

export function previewText(value: string | null | undefined, fallback: string): string {
  const normalized = value?.replace(/\s+/g, ' ').trim() || '';
  if (!normalized) {
    return fallback;
  }
  return normalized.length > 140 ? `${normalized.slice(0, 137)}...` : normalized;
}

export function providerModelsWithCurrent(
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

export function reasoningEffortsWithCurrent(
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

export function serviceTiersWithCurrent(
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

export function avatarLabel(value: string): string {
  return value
    .split(/\s+/)
    .map((part) => part[0] || '')
    .join('')
    .slice(0, 2)
    .toUpperCase();
}

export function readFileAsDataUrl(file: File): Promise<string> {
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

export function loadImageFromDataUrl(dataUrl: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onerror = () => reject(new Error('Failed to read avatar image'));
    image.onload = () => resolve(image);
    image.src = dataUrl;
  });
}

export async function normalizeAvatarFile(file: File): Promise<string> {
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

export function sortedAgents(value: DesktopCustomAgent[]): DesktopCustomAgent[] {
  return [...value]
    .filter((agent) => agent.standalone)
    .sort((left, right) => {
      if (left.builtIn !== right.builtIn) {
        return left.builtIn ? -1 : 1;
      }
      return left.displayName.localeCompare(right.displayName) || left.agentId.localeCompare(right.agentId);
    });
}

export function prettyJson(value: unknown): string {
  if (!value || typeof value !== 'object') {
    return '';
  }
  return JSON.stringify(value, null, 2);
}

export function stopEvent(event: React.MouseEvent<HTMLElement>) {
  event.preventDefault();
  event.stopPropagation();
}
