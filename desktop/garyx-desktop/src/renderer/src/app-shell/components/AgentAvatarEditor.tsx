import type { DesktopProviderIconDescriptor } from '@shared/contracts';

import { Button } from '../../components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../components/ui/dialog';
import { Label } from '../../components/ui/label';
import { Textarea } from '../../components/ui/textarea';
import { useI18n } from '../../i18n';
import { ProviderAgentIcon, hasProviderAgentIcon } from './ProviderAgentIcon';
import { AVATAR_STYLE_OPTIONS, CUSTOM_AVATAR_STYLE_ID, avatarLabel } from './agents-hub-helpers';
import type { AvatarStyleId, ProviderType } from './agents-hub-helpers';

type AgentAvatarEditorProps = {
  agentId?: string | null;
  avatarDataUrl?: string | null;
  builtIn?: boolean;
  className: string;
  label: string;
  providerIcon?: DesktopProviderIconDescriptor | null;
  providerType?: ProviderType | null;
};

export function AgentAvatarEditor({
  agentId,
  avatarDataUrl,
  builtIn,
  className,
  label,
  providerIcon,
  providerType,
}: AgentAvatarEditorProps) {
  const showProviderIcon =
    Boolean(builtIn && !avatarDataUrl)
    && hasProviderAgentIcon(agentId, providerType, providerIcon);
  const classes = [
    className,
    builtIn ? 'builtin' : '',
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

type AvatarStyleDialogProps = {
  avatarGenerating: boolean;
  avatarStyleDialogOpen: boolean;
  avatarStyleId: AvatarStyleId;
  customAvatarStyle: string;
  handleGenerateAvatar: (stylePrompt: string) => Promise<void>;
  setAvatarStyleDialogOpen: React.Dispatch<React.SetStateAction<boolean>>;
  setAvatarStyleId: React.Dispatch<React.SetStateAction<AvatarStyleId>>;
  setCustomAvatarStyle: React.Dispatch<React.SetStateAction<string>>;
};

export function AvatarStyleDialog({
  avatarGenerating,
  avatarStyleDialogOpen,
  avatarStyleId,
  customAvatarStyle,
  handleGenerateAvatar,
  setAvatarStyleDialogOpen,
  setAvatarStyleId,
  setCustomAvatarStyle,
}: AvatarStyleDialogProps) {
  const { t } = useI18n();

  const activeAvatarStylePrompt = avatarStyleId === CUSTOM_AVATAR_STYLE_ID
    ? customAvatarStyle.trim()
    : AVATAR_STYLE_OPTIONS.find((option) => option.id === avatarStyleId)?.prompt || '';
  const avatarStyleValidationError =
    avatarStyleId === CUSTOM_AVATAR_STYLE_ID && !customAvatarStyle.trim()
      ? t('Custom style is required.')
      : null;

  return (
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
  );
}
