import { useEffect, useRef, useState } from 'react';

import type {
  ThreadChannelBindingInfo,
  ThreadRuntimeInfo,
} from '@shared/contracts';

import { InfoIcon } from './app-shell/icons';
import { useI18n } from './i18n';

type ThreadInfoPopoverProps = {
  threadId: string | null;
  threadInfo: ThreadRuntimeInfo | null;
  threadInfoLoaded: boolean;
};

function shellEscape(value: string): string {
  if (!value) {
    return "''";
  }
  return `'${value.replace(/'/g, `'\\''`)}'`;
}

function buildResumeCommand(threadInfo: ThreadRuntimeInfo | null): string | null {
  const sessionId = threadInfo?.sdkSessionId?.trim() || '';
  if (!sessionId) {
    return null;
  }

  let command = '';
  switch (threadInfo?.providerType) {
    case 'claude_code':
      command = `claude --permission-mode bypassPermissions --resume ${shellEscape(sessionId)}`;
      break;
    case 'codex_app_server':
      command = `codex -a never -s danger-full-access resume ${shellEscape(sessionId)}`;
      break;
    case 'gemini_cli':
      command = `gemini --approval-mode yolo --resume ${shellEscape(sessionId)}`;
      break;
    default:
      return null;
  }

  const workspacePath = threadInfo.workspacePath?.trim() || '';
  return workspacePath
    ? `cd ${shellEscape(workspacePath)}; ${command}`
    : command;
}

function providerDisplayName(threadInfo: ThreadRuntimeInfo | null): string {
  return threadInfo?.providerLabel?.trim() || 'Unknown';
}

function describeBinding(binding: ThreadChannelBindingInfo): string {
  const parts = [
    binding.channel || 'channel',
    binding.accountId || '',
    binding.displayLabel || binding.deliveryTargetId || binding.chatId || binding.bindingKey || '',
  ].filter(Boolean);
  return parts.join(' · ');
}

type InfoRowProps = {
  label: string;
  value: string;
  mono?: boolean;
};

function InfoRow({ label, value, mono = false }: InfoRowProps) {
  return (
    <div className="thread-info-row">
      <span className="thread-info-row-label">{label}</span>
      <span className={`thread-info-row-value${mono ? ' is-mono' : ''}`} title={value}>
        {value}
      </span>
    </div>
  );
}

export function ThreadInfoPopover({
  threadId,
  threadInfo,
  threadInfoLoaded,
}: ThreadInfoPopoverProps) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const [copyState, setCopyState] = useState<'idle' | 'command' | 'session'>('idle');
  const shellRef = useRef<HTMLDivElement | null>(null);
  const sessionId = threadInfo?.sdkSessionId?.trim() || '';
  const resumeCommand = buildResumeCommand(threadInfo);
  const bindings = threadInfo?.channelBindings || [];

  useEffect(() => {
    setOpen(false);
    setCopyState('idle');
  }, [threadId]);

  useEffect(() => {
    if (!open) {
      return undefined;
    }

    function handlePointerDown(event: PointerEvent) {
      if (!shellRef.current?.contains(event.target as Node)) {
        setOpen(false);
      }
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape') {
        setOpen(false);
      }
    }

    window.addEventListener('pointerdown', handlePointerDown);
    window.addEventListener('keydown', handleKeyDown);
    return () => {
      window.removeEventListener('pointerdown', handlePointerDown);
      window.removeEventListener('keydown', handleKeyDown);
    };
  }, [open]);

  useEffect(() => {
    if (copyState === 'idle') {
      return undefined;
    }
    const timer = window.setTimeout(() => {
      setCopyState('idle');
    }, 1800);
    return () => {
      window.clearTimeout(timer);
    };
  }, [copyState]);

  async function copyValue(value: string, nextState: 'command' | 'session') {
    try {
      await navigator.clipboard.writeText(value);
      setCopyState(nextState);
    } catch {
      setCopyState('idle');
    }
  }

  return (
    <div className={`thread-info-shell${open ? ' is-open' : ''}`} ref={shellRef}>
      <button
        aria-expanded={open}
        aria-haspopup="dialog"
        className={`conversation-header-action-button conversation-header-action-icon${open ? ' is-open' : ''}`}
        disabled={!threadId}
        onClick={() => {
          setOpen((current) => !current);
        }}
        title={t('Thread information')}
        type="button"
      >
        <InfoIcon />
      </button>

      {open ? (
        <div
          aria-label={t('Thread information')}
          className="thread-info-panel"
          role="dialog"
        >
          <div className="thread-info-panel-header">
            <div className="thread-info-panel-kicker">{t('Thread Info')}</div>
            <div className="thread-info-panel-title">
              {providerDisplayName(threadInfo)}
            </div>
            <div className="thread-info-panel-subtitle">{threadId || t('No thread selected')}</div>
          </div>

          {threadInfo ? (
            <>
              <div className="thread-info-grid">
                {threadInfo.agentId ? (
                  <InfoRow label={t('Agent')} value={threadInfo.agentId} />
                ) : null}
                {threadInfo.workspacePath ? (
                  <InfoRow label={t('Workspace')} value={threadInfo.workspacePath} mono />
                ) : null}
                {sessionId ? (
                  <InfoRow label={t('Session ID')} value={sessionId} mono />
                ) : null}
              </div>

              {resumeCommand ? (
                <div className="thread-info-command-block">
                  <div className="thread-info-command-label">{t('Resume command')}</div>
                  <code className="thread-info-command-value">{resumeCommand}</code>
                </div>
              ) : null}

              {bindings.length ? (
                <div className="thread-info-bindings">
                  <div className="thread-info-section-label">{t('Bindings')}</div>
                  <div className="thread-info-binding-list">
                    {bindings.map((binding) => (
                      <div className="thread-info-binding-row" key={`${binding.channel}:${binding.accountId}:${binding.bindingKey}`}>
                        {describeBinding(binding)}
                      </div>
                    ))}
                  </div>
                </div>
              ) : null}

              <div className="thread-info-actions">
                {sessionId ? (
                  <button
                    className="thread-info-action"
                    onClick={() => {
                      void copyValue(sessionId, 'session');
                    }}
                    type="button"
                  >
                    {copyState === 'session' ? t('Session Copied') : t('Copy Session ID')}
                  </button>
                ) : null}
                {resumeCommand ? (
                  <button
                    className="thread-info-action"
                    onClick={() => {
                      void copyValue(resumeCommand, 'command');
                    }}
                    type="button"
                  >
                    {copyState === 'command' ? t('Command Copied') : t('Copy Resume Command')}
                  </button>
                ) : null}
              </div>
            </>
          ) : threadInfoLoaded ? (
            <div className="thread-info-empty">
              {t('No provider runtime details are stored for this thread yet.')}
            </div>
          ) : (
            <div className="thread-info-empty">
              {t('Loading thread info…')}
            </div>
          )}
        </div>
      ) : null}
    </div>
  );
}
