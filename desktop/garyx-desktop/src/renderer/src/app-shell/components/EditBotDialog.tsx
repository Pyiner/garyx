/**
 * Schema-driven "Edit bot" dialog.
 *
 * Rewritten (April 2026) from a 424-line per-channel branching
 * monster into a catalog-driven wrapper around `PluginConfigSections`.
 * All channel-specific fields (token, app_id, domain, etc.) are now rendered by the
 * plugin's JSON Schema through the generic sections component.
 *
 * The dialog still owns the **account-metadata** fields that live
 * outside any plugin's schema:
 *   - `name` display name
 *   - `agent_id` binding
 *   - `workspace_dir` override
 *
 * Save path: combines metadata fields + plugin-config values and
 * emits an `EditBotPatch` the existing gateway expects. The patch
 * shape still names channel-specific fields (token / appId /
 * appSecret / domain / ...); the translator `configToPatchFields`
 * adapts the plugin-config dict back to that shape. When the
 * underlying `updateChannelAccount` IPC is generalised to accept
 * `config: Record<string, unknown>`, the translator disappears.
 */
import { useEffect, useMemo, useState } from "react";

import type {
  ChannelPluginCatalogEntry,
} from "@shared/contracts";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

import { DirectoryInput } from "../../components/DirectoryInput";
import { PluginConfigSections } from "../../channel-plugins/PluginConfigPanel";
import { useChannelPluginCatalog } from "../../channel-plugins/useChannelPluginCatalog";

type AgentTargetOption = { value: string; label: string };

export type EditBotDialogContext = {
  kind: string;
  accountId: string;
  account: any;
  resolvedAgentId: string;
};

export type EditBotPatch = {
  name?: string | null;
  agentId?: string;
  workspaceDir?: string | null;
  config?: Record<string, unknown>;
};

type EditBotDialogProps = {
  open: boolean;
  context: EditBotDialogContext | null;
  agentTargets: AgentTargetOption[];
  saving?: boolean;
  onClose: () => void;
  onSave: (input: {
    kind: string;
    accountId: string;
    patch: EditBotPatch;
  }) => Promise<void> | void;
  onRemove: (input: {
    kind: string;
    accountId: string;
  }) => Promise<void> | void;
};

/**
 * Seed the plugin-config form state from the account object the
 * gateway returned. Account keys are snake_case (server shape); the
 * plugin's JSON Schema uses the same keys — no transformation
 * needed, just a shallow copy of the string / array / boolean
 * fields. Non-plugin-config keys (enabled, name, workspace_dir,
 * etc.) are stripped so the JsonSchemaForm doesn't try to render
 * them as "extra" fields.
 */
function accountToConfig(account: Record<string, unknown>): Record<string, unknown> {
  const nested = account.config;
  if (nested && typeof nested === "object" && !Array.isArray(nested)) {
    return { ...(nested as Record<string, unknown>) };
  }
  const STRIP = new Set([
    "enabled",
    "name",
    "agent_id",
    "workspace_dir",
    "owner_target",
    "groups",
  ]);
  const out: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(account)) {
    if (STRIP.has(key)) continue;
    out[key] = value;
  }
  return out;
}

export function EditBotDialog(props: EditBotDialogProps) {
  const { open, context, agentTargets, saving, onClose, onSave, onRemove } =
    props;
  const { entries } = useChannelPluginCatalog();

  const [name, setName] = useState("");
  const [agentId, setAgentId] = useState("");
  const [workspaceDir, setWorkspaceDir] = useState("");
  const [pluginConfig, setPluginConfig] = useState<Record<string, unknown>>({});
  const [error, setError] = useState<string | null>(null);
  const [removing, setRemoving] = useState(false);

  // Seed state whenever the dialog opens on a fresh context. Resets
  // the plugin-config form to the account's current values so the
  // user sees what's in effect, not stale input from the previous
  // open.
  useEffect(() => {
    if (!open || !context) {
      setError(null);
      setRemoving(false);
      return;
    }
    const account = (context.account || {}) as Record<string, unknown>;
    setName(String(account.name || ""));
    setAgentId(context.resolvedAgentId || "");
    setWorkspaceDir(String(account.workspace_dir || ""));
    setPluginConfig(accountToConfig(account));
    setError(null);
    setRemoving(false);
  }, [open, context]);

  const selectedEntry: ChannelPluginCatalogEntry | null = useMemo(() => {
    if (!entries || !context) return null;
    return (
      entries.find(
        (e) =>
          e.id === context.kind ||
          e.id.toLowerCase() === String(context.kind).toLowerCase(),
      ) ?? null
    );
  }, [entries, context]);

  if (!context) {
    return (
      <Dialog
        open={open}
        onOpenChange={(next) => {
          if (!next) onClose();
        }}
      >
        <DialogContent className="sm:max-w-[640px]" />
      </Dialog>
    );
  }

  const { kind, accountId } = context;
  const selectedAgentMissing = Boolean(
    agentId && !agentTargets.find((t) => t.value === agentId),
  );

  async function handleSave() {
    if (!context) return;
    setError(null);
    try {
      const patch: EditBotPatch = {
        name: name.trim() || null,
        workspaceDir: workspaceDir.trim() || null,
        config: pluginConfig,
      };
      if (agentId) patch.agentId = agentId;
      await onSave({ kind, accountId, patch });
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : "保存失败");
    }
  }

  async function handleRemove() {
    if (!context) return;
    if (!window.confirm(`确认删除 ${kind} 账号 "${name || accountId}"？`)) return;
    setRemoving(true);
    setError(null);
    try {
      await onRemove({ kind, accountId });
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : "删除失败");
    } finally {
      setRemoving(false);
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      <DialogContent className="sm:max-w-[640px]">
        <DialogHeader>
          <DialogTitle>编辑账号</DialogTitle>
          <DialogDescription>
            {`${selectedEntry?.display_name || kind} · ${accountId}`}
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-4">
          <div className="grid grid-cols-2 gap-3">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="edit-bot-name">显示名</Label>
              <Input
                id="edit-bot-name"
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="edit-bot-agent">Agent</Label>
              <Select value={agentId} onValueChange={setAgentId}>
                <SelectTrigger id="edit-bot-agent">
                  <SelectValue placeholder="选择 agent" />
                </SelectTrigger>
                <SelectContent>
                  {agentTargets.map((target) => (
                    <SelectItem key={target.value} value={target.value}>
                      {target.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {selectedAgentMissing && (
                <span className="text-xs text-amber-700">
                  当前 agent `{agentId}` 已不存在，请重新选择
                </span>
              )}
            </div>
            <div className="col-span-2 flex flex-col gap-1.5">
              <Label htmlFor="edit-bot-workspace">工作目录（可选）</Label>
              <DirectoryInput
                id="edit-bot-workspace"
                value={workspaceDir}
                onChange={setWorkspaceDir}
                placeholder="默认沿用主工作区"
              />
            </div>
          </div>

          {selectedEntry ? (
            <PluginConfigSections
              entry={selectedEntry}
              value={pluginConfig}
              onChange={setPluginConfig}
              secretInputType="text"
              showAutoLoginMethod={false}
            />
          ) : (
            <div className="rounded-md border border-[#eeeeee] bg-[#fafaf9] p-3 text-sm text-neutral-500">
              加载渠道目录…
            </div>
          )}
        </div>

        <DialogFooter className="gap-2">
          {error ? (
            <span className="mr-auto text-sm text-red-700">{error}</span>
          ) : null}
          <Button
            variant="outline"
            onClick={() => void handleRemove()}
            disabled={removing || saving}
          >
            {removing ? "删除中…" : "删除账号"}
          </Button>
          <Button variant="outline" onClick={onClose} disabled={saving || removing}>
            取消
          </Button>
          <Button onClick={() => void handleSave()} disabled={saving || removing}>
            {saving ? "保存中…" : "保存"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
