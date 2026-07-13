export type AvatarStyleId =
  | 'clean_glyph'
  | 'soft_3d'
  | 'glass_icon'
  | 'pixel_badge'
  | 'ink_line'
  | 'paper_cut'
  | 'blueprint'
  | 'enamel_sticker'
  | 'custom';

export const CUSTOM_AVATAR_STYLE_ID: AvatarStyleId = 'custom';
export const DEFAULT_AVATAR_STYLE_ID: AvatarStyleId = 'clean_glyph';

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

export type AgentAvatarPromptInput = {
  agentId?: string | null;
  displayName: string;
  stylePrompt?: string | null;
};

export function buildAgentAvatarPrompt(input: AgentAvatarPromptInput): string {
  const avatarName = input.displayName.trim() || input.agentId?.trim() || 'Agent';
  const name = JSON.stringify(avatarName);
  const stylePrompt = input.stylePrompt?.trim()
    || 'minimal vector glyph, simple geometry, balanced negative space, one confident accent color';
  return [
    `Create a square app avatar for an AI agent named ${name}.`,
    `Visual style: ${stylePrompt}.`,
    'Composition: one centered abstract agent mark, clean silhouette, readable at 32px, restrained palette, polished macOS developer-tool finish.',
    'Do not include text, letters, watermarks, screenshots, people, or UI chrome.',
  ].join('\n');
}
