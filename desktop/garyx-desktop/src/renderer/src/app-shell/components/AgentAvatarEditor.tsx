import { useEffect, useRef, useState } from 'react';

import type { DesktopProviderIconDescriptor } from '@shared/contracts';

import { Button } from '../../components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../components/ui/dialog';
import { Label } from '../../components/ui/label';
import { Textarea } from '../../components/ui/textarea';
import { useI18n } from '../../i18n';
import { ProviderAgentIcon, hasProviderAgentIcon } from './ProviderAgentIcon';
import type { AvatarGenerationFlow } from './agent-avatar-flow';
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
  isLoading?: boolean;
};

export function AgentAvatarEditor({
  agentId,
  avatarDataUrl,
  builtIn,
  className,
  label,
  providerIcon,
  providerType,
  isLoading = false,
}: AgentAvatarEditorProps) {
  const { t } = useI18n();
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
      {isLoading ? (
        <span aria-hidden className="agents-hub-avatar-loading-overlay">
          <span className="agents-hub-avatar-loading-spinner" />
          <span className="agents-hub-avatar-loading-text">{t('Generating…')}</span>
          <span className="agents-hub-avatar-loading-sweep" />
        </span>
      ) : null}
    </span>
  );
}

type AvatarStyleDialogProps = {
  agentId: string;
  avatarStyleDialogOpen: boolean;
  avatarStyleId: AvatarStyleId;
  builtIn?: boolean;
  customAvatarStyle: string;
  displayName: string;
  flow: AvatarGenerationFlow;
  handleGenerateAvatar: (stylePrompt: string) => void;
  onCancel: () => void;
  onChangeStyle: () => void;
  onUseAvatar: () => void;
  providerType: ProviderType;
  setAvatarStyleId: React.Dispatch<React.SetStateAction<AvatarStyleId>>;
  setCustomAvatarStyle: React.Dispatch<React.SetStateAction<string>>;
};

export function AvatarStyleDialog({
  agentId,
  avatarStyleDialogOpen,
  avatarStyleId,
  builtIn,
  customAvatarStyle,
  displayName,
  flow,
  handleGenerateAvatar,
  onCancel,
  onChangeStyle,
  onUseAvatar,
  providerType,
  setAvatarStyleId,
  setCustomAvatarStyle,
}: AvatarStyleDialogProps) {
  const { t } = useI18n();
  const failureRef = useRef<HTMLDivElement | null>(null);
  const useAvatarRef = useRef<HTMLButtonElement | null>(null);
  const [showsLongWaitMessage, setShowsLongWaitMessage] = useState(false);
  const avatarGenerating = flow.phase === 'generating';

  const activeAvatarStylePrompt = avatarStyleId === CUSTOM_AVATAR_STYLE_ID
    ? customAvatarStyle.trim()
    : AVATAR_STYLE_OPTIONS.find((option) => option.id === avatarStyleId)?.prompt || '';
  const avatarStyleValidationError =
    avatarStyleId === CUSTOM_AVATAR_STYLE_ID && !customAvatarStyle.trim()
      ? t('Custom style is required.')
      : null;

  useEffect(() => {
    if (flow.phase === 'failed') {
      failureRef.current?.focus();
    } else if (flow.phase === 'candidate') {
      useAvatarRef.current?.focus();
    }
  }, [flow.phase]);

  useEffect(() => {
    if (!avatarGenerating) {
      setShowsLongWaitMessage(false);
      return undefined;
    }
    const timeout = window.setTimeout(() => {
      setShowsLongWaitMessage(true);
    }, 8_000);
    return () => window.clearTimeout(timeout);
  }, [avatarGenerating, flow.requestId]);

  return (
    <Dialog
      open={avatarStyleDialogOpen}
      onOpenChange={(open) => {
        if (!open) {
          onCancel();
        }
      }}
    >
      <DialogContent className="agents-hub-avatar-style-dialog" size="compact">
        <DialogHeader>
          <DialogTitle>{t('Avatar style')}</DialogTitle>
          <DialogDescription className="sr-only">
            {t('Choose a style, then generate a preview. Your current avatar will not change until you use the result.')}
          </DialogDescription>
        </DialogHeader>

        <div className={`agents-hub-avatar-comparison ${flow.candidateAvatarDataUrl ? 'has-candidate' : ''}`}>
          <AvatarComparisonItem
            agentId={agentId}
            avatarDataUrl={flow.currentAvatarDataUrl}
            builtIn={builtIn}
            displayName={displayName}
            isLoading={avatarGenerating && !flow.candidateAvatarDataUrl}
            providerType={providerType}
            title={t('Current')}
          />
          {flow.candidateAvatarDataUrl ? (
            <AvatarComparisonItem
              agentId={agentId}
              avatarDataUrl={flow.candidateAvatarDataUrl}
              builtIn={builtIn}
              displayName={displayName}
              isLoading={avatarGenerating}
              providerType={providerType}
              title={t('New avatar')}
            />
          ) : null}
        </div>

        <div aria-live="polite" className="agents-hub-avatar-flow-status">
          {flow.phase === 'choosing' ? (
            <span>{t('Choose a style, then generate a preview. Your current avatar will not change until you use the result.')}</span>
          ) : flow.phase === 'generating' ? (
            <span>
              {t('Generating avatar…')}
              {showsLongWaitMessage ? ` ${t('This can take a little while.')}` : ''}
            </span>
          ) : flow.phase === 'candidate' ? (
            <span>{t('Avatar ready. Use avatar updates the form draft; Save persists it.')}</span>
          ) : (
            <div
              className="agents-hub-avatar-flow-error"
              ref={failureRef}
              role="status"
              tabIndex={-1}
            >
              {t(flow.failure?.message || 'Couldn’t generate an avatar.')}
            </div>
          )}
        </div>

        {flow.phase === 'choosing' || flow.phase === 'generating' ? (
          <div aria-disabled={avatarGenerating} className="agents-hub-avatar-style-controls">
            <div className="agents-hub-avatar-style-grid">
              {AVATAR_STYLE_OPTIONS.map((option) => (
                <button
                  className={`agents-hub-avatar-style-option ${avatarStyleId === option.id ? 'active' : ''}`}
                  disabled={avatarGenerating}
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
                disabled={avatarGenerating}
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
                  disabled={avatarGenerating}
                  id="agent-avatar-custom-style"
                  onChange={(event) => {
                    setCustomAvatarStyle(event.target.value);
                  }}
                  placeholder={t('e.g. polished paper-cut icon with emerald accents')}
                  value={customAvatarStyle}
                />
              </div>
            ) : null}
          </div>
        ) : null}

        <DialogFooter className="agents-hub-dialog-footer">
          <div className="agents-hub-dialog-status">{avatarStyleValidationError}</div>
          <div className="agents-hub-dialog-actions">
            <Button
              onClick={onCancel}
              type="button"
              variant="outline"
            >
              {avatarGenerating ? t('Cancel generation') : t('Cancel')}
            </Button>
            {flow.phase === 'failed' ? (
              <Button onClick={onChangeStyle} type="button" variant="outline">
                {t('Change style')}
              </Button>
            ) : null}
            {flow.phase === 'candidate' ? (
              <Button
                onClick={() => handleGenerateAvatar(activeAvatarStylePrompt)}
                type="button"
                variant="outline"
              >
                {t('Generate again')}
              </Button>
            ) : null}
            {flow.phase === 'choosing' ? (
              <Button
                disabled={Boolean(avatarStyleValidationError)}
                onClick={() => handleGenerateAvatar(activeAvatarStylePrompt)}
                type="button"
              >
                {t('Generate')}
              </Button>
            ) : flow.phase === 'generating' ? (
              <Button disabled type="button">
                <span aria-hidden className="agents-hub-button-spinner" />
                {t('Generating…')}
              </Button>
            ) : flow.phase === 'candidate' ? (
              <Button onClick={onUseAvatar} ref={useAvatarRef} type="button">
                {t('Use avatar')}
              </Button>
            ) : (
              <Button onClick={() => handleGenerateAvatar(activeAvatarStylePrompt)} type="button">
                {t('Retry')}
              </Button>
            )}
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function AvatarComparisonItem({
  agentId,
  avatarDataUrl,
  builtIn,
  displayName,
  isLoading,
  providerType,
  title,
}: {
  agentId: string;
  avatarDataUrl: string | null;
  builtIn?: boolean;
  displayName: string;
  isLoading: boolean;
  providerType: ProviderType;
  title: string;
}) {
  return (
    <div className="agents-hub-avatar-comparison-item">
      <span className="agents-hub-avatar-comparison-label">{title}</span>
      <AgentAvatarEditor
        agentId={agentId}
        avatarDataUrl={avatarDataUrl}
        builtIn={builtIn}
        className="agents-hub-avatar-centered avatar-flow-preview"
        isLoading={isLoading}
        label={displayName || agentId || 'A'}
        providerType={providerType}
      />
    </div>
  );
}
