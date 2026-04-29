import React from 'react';

import type { DesktopMemoryDocumentScope } from '@shared/contracts';

import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Textarea } from '@/components/ui/textarea';
import { useI18n } from '../i18n';

type MemoryDialogProps = {
  open: boolean;
  scope: DesktopMemoryDocumentScope;
  title: string;
  path: string | null;
  draftContent: string;
  dirty: boolean;
  exists: boolean;
  loading: boolean;
  saving: boolean;
  error: string | null;
  status: string | null;
  modifiedAt?: string | null;
  onDraftChange: (value: string) => void;
  onSave: () => void;
  onClose: () => void;
};

function formatTimestamp(value?: string | null): string {
  if (!value) {
    return '';
  }
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) {
    return '';
  }
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  }).format(parsed);
}

export function MemoryDialog({
  open,
  scope,
  title,
  path,
  draftContent,
  dirty,
  exists,
  loading,
  saving,
  error,
  status,
  modifiedAt,
  onDraftChange,
  onSave,
  onClose,
}: MemoryDialogProps) {
  const { t } = useI18n();
  const modifiedLabel = formatTimestamp(modifiedAt);

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen) onClose();
      }}
    >
      <DialogContent className="sm:max-w-[860px]">
        <DialogHeader>
          <DialogTitle className="text-base font-semibold">{title}</DialogTitle>
          <DialogDescription className="break-all font-mono text-[11px] leading-5 text-muted-foreground">
            {path || t('Resolving memory path…')}
          </DialogDescription>
        </DialogHeader>

        <div className="grid gap-3">
          <div className="flex flex-wrap items-center gap-2">
            <Badge
              variant="secondary"
              className="rounded-full px-2.5 py-0.5 text-[10px] font-medium"
            >
              {scope === 'global' ? t('Global Memory') : t('Automation Memory')}
            </Badge>
            <Badge
              variant={exists ? 'outline' : 'secondary'}
              className="rounded-full px-2.5 py-0.5 text-[10px] font-medium"
            >
              {exists ? t('Existing File') : t('Create On Save')}
            </Badge>
            {modifiedLabel ? (
              <Badge
                variant="secondary"
                className="rounded-full px-2.5 py-0.5 text-[10px] font-medium"
              >
                {t('Updated {time}', { time: modifiedLabel })}
              </Badge>
            ) : null}
            {dirty ? (
              <Badge
                variant="secondary"
                className="rounded-full px-2.5 py-0.5 text-[10px] font-medium"
              >
                {t('Unsaved Changes')}
              </Badge>
            ) : null}
            {status ? (
              <span className="text-[12px] font-medium text-emerald-700">{status}</span>
            ) : null}
          </div>

          {error ? (
            <div className="rounded-2xl border border-rose-200 bg-rose-50 px-4 py-3 text-[13px] leading-6 text-rose-700">
              {error}
            </div>
          ) : null}

          {!exists && !loading ? (
            <div className="rounded-2xl border border-[#ece5d9] bg-[#fffaf2] px-4 py-3 text-[13px] leading-6 text-[#6a5840]">
              {t('This memory file does not exist yet. Save once to create it.')}
            </div>
          ) : null}

          <Textarea
            className="min-h-[420px] resize-y rounded-2xl border-[#e7e7e5] bg-white font-mono text-[13px] leading-6 shadow-none"
            disabled={loading || saving}
            placeholder={loading ? t('Loading memory…') : t('Write durable notes for future runs.')}
            spellCheck={false}
            value={draftContent}
            onChange={(event) => {
              onDraftChange(event.target.value);
            }}
          />
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onClose} type="button">
            {t('Close')}
          </Button>
          <Button disabled={loading || saving || !dirty} onClick={onSave} type="button">
            {saving ? t('Saving…') : t('Save Memory')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
