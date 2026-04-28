import { useEffect, useState } from 'react';

import type { DesktopMemoryDocument, DesktopMemoryDocumentScope } from '@shared/contracts';

import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Textarea } from '@/components/ui/textarea';

type ThreadMemoryPanelProps = {
  workspacePath?: string | null;
};

type MemoryTarget =
  | { scope: 'global'; title: string }
  | { scope: 'workspace'; title: string; workspacePath: string };

function formatTimestamp(value?: string | null): string {
  if (!value) {
    return '';
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return '';
  }
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  }).format(date);
}

function buildMemoryTarget(
  scope: DesktopMemoryDocumentScope,
  workspacePath?: string | null,
): MemoryTarget {
  if (scope === 'workspace' && workspacePath?.trim()) {
    return {
      scope: 'workspace',
      title: 'Workspace Memory',
      workspacePath: workspacePath.trim(),
    };
  }
  return {
    scope: 'global',
    title: 'Global Memory',
  };
}

function buildReadInput(target: MemoryTarget) {
  if (target.scope === 'workspace') {
    return {
      scope: 'workspace' as const,
      workspacePath: target.workspacePath,
    };
  }
  return {
    scope: 'global' as const,
  };
}

export function ThreadMemoryPanel({ workspacePath }: ThreadMemoryPanelProps) {
  const [activeScope, setActiveScope] = useState<DesktopMemoryDocumentScope>(
    workspacePath?.trim() ? 'workspace' : 'global',
  );
  const [document, setDocument] = useState<DesktopMemoryDocument | null>(null);
  const [draft, setDraft] = useState('');
  const [savedContent, setSavedContent] = useState('');
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  useEffect(() => {
    if (!workspacePath?.trim() && activeScope === 'workspace') {
      setActiveScope('global');
    }
  }, [activeScope, workspacePath]);

  useEffect(() => {
    let cancelled = false;
    async function load() {
      const target = buildMemoryTarget(activeScope, workspacePath);
      setLoading(true);
      setSaving(false);
      setError(null);
      setStatus(null);
      try {
        const nextDocument = await window.garyxDesktop.readMemoryDocument(buildReadInput(target));
        if (cancelled) {
          return;
        }
        setDocument(nextDocument);
        setDraft(nextDocument.content);
        setSavedContent(nextDocument.content);
      } catch (loadError) {
        if (cancelled) {
          return;
        }
        setDocument(null);
        setDraft('');
        setSavedContent('');
        setError(loadError instanceof Error ? loadError.message : 'Failed to load memory.md.');
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    }
    void load();
    return () => {
      cancelled = true;
    };
  }, [activeScope, workspacePath]);

  const dirty = draft !== savedContent;
  const modifiedLabel = formatTimestamp(document?.modifiedAt);
  const canUseWorkspace = Boolean(workspacePath?.trim());

  async function handleSave() {
    const target = buildMemoryTarget(activeScope, workspacePath);
    setSaving(true);
    setError(null);
    setStatus(null);
    try {
      const nextDocument = await window.garyxDesktop.saveMemoryDocument({
        ...buildReadInput(target),
        content: draft,
      });
      setDocument(nextDocument);
      setDraft(nextDocument.content);
      setSavedContent(nextDocument.content);
      setStatus('Saved memory.md.');
    } catch (saveError) {
      setError(saveError instanceof Error ? saveError.message : 'Failed to save memory.md.');
    } finally {
      setSaving(false);
    }
  }

  return (
    <Card className="thread-memory-panel">
      <CardHeader className="space-y-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <CardTitle className="text-base">Memory</CardTitle>
            <CardDescription>
              Edit durable memory without leaving the thread.
            </CardDescription>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <Button
              size="sm"
              type="button"
              variant={activeScope === 'workspace' ? 'default' : 'outline'}
              disabled={!canUseWorkspace}
              onClick={() => {
                setActiveScope('workspace');
              }}
            >
              Workspace
            </Button>
            <Button
              size="sm"
              type="button"
              variant={activeScope === 'global' ? 'default' : 'outline'}
              onClick={() => {
                setActiveScope('global');
              }}
            >
              Global
            </Button>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <Badge variant="secondary">
            {activeScope === 'workspace' ? 'Workspace Memory' : 'Global Memory'}
          </Badge>
          <Badge variant={document?.exists ? 'outline' : 'secondary'}>
            {document?.exists ? 'Existing File' : 'Create On Save'}
          </Badge>
          {modifiedLabel ? <Badge variant="secondary">Updated {modifiedLabel}</Badge> : null}
          {dirty ? <Badge variant="secondary">Unsaved Changes</Badge> : null}
          {status ? <span className="text-[12px] font-medium text-emerald-700">{status}</span> : null}
        </div>

        <div className="break-all font-mono text-[11px] leading-5 text-muted-foreground">
          {document?.path || 'Resolving memory path…'}
        </div>
      </CardHeader>

      <CardContent className="grid gap-3">
        {!canUseWorkspace && activeScope === 'global' ? (
          <div className="rounded-2xl border border-[#ece5d9] bg-[#fffaf2] px-4 py-3 text-[13px] leading-6 text-[#6a5840]">
            This thread is not bound to a workspace, so only global memory is available.
          </div>
        ) : null}

        {error ? (
          <div className="rounded-2xl border border-rose-200 bg-rose-50 px-4 py-3 text-[13px] leading-6 text-rose-700">
            {error}
          </div>
        ) : null}

        {!document?.exists && !loading ? (
          <div className="rounded-2xl border border-[#ece5d9] bg-[#fffaf2] px-4 py-3 text-[13px] leading-6 text-[#6a5840]">
            This memory file does not exist yet. Save once to create it.
          </div>
        ) : null}

        <Textarea
          className="min-h-[420px] resize-y rounded-2xl border-[#e7e7e5] bg-white font-mono text-[13px] leading-6 shadow-none"
          disabled={loading || saving}
          placeholder={loading ? 'Loading memory…' : 'Write durable notes for future runs.'}
          spellCheck={false}
          value={draft}
          onChange={(event) => {
            setDraft(event.target.value);
          }}
        />

        <div className="flex justify-end">
          <Button disabled={loading || saving || !dirty} onClick={() => { void handleSave(); }} type="button">
            {saving ? 'Saving…' : 'Save Memory'}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
