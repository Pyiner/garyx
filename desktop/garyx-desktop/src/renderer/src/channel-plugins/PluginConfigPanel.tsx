/**
 * Schema-driven channel-plugin configuration panel.
 *
 * Two exports:
 *
 * - [`PluginConfigSections`] — headless: renders the form + auto-
 *   login sections for a given catalog entry. Caller owns the
 *   form state + save flow. Used by `AddBotDialog` / `EditBotDialog`
 *   which want to compose plugin-config with dialog-owned generic
 *   fields (accountId, name, workspaceDir).
 *
 * - [`PluginConfigPanel`] — standalone: `PluginConfigSections`
 *   wrapped with its own state + "保存" button. Useful as a
 *   drop-in in settings panels where the plugin's config is the
 *   only thing being edited.
 *
 * Both walk `config_methods[]` in order and render one block per
 * entry:
 *
 *   - `{kind:"form"}`       → JsonSchemaForm
 *   - `{kind:"auto_login"}` → button → AuthFlowDriver on click
 *   - unknown               → skipped (forward-compat)
 *
 * The panel is channel-blind — no `if (pluginId === "feishu") …`
 * branches anywhere. Adding a fourth channel (or a subprocess
 * plugin with the same contract) requires zero UI code.
 */
import { useCallback, useMemo, useState, type ReactElement } from "react";

import type {
  ChannelPluginCatalogEntry,
  ChannelPluginConfigMethod,
} from "@shared/contracts";

import { AuthFlowDriver } from "./AuthFlowDriver";
import { JsonSchemaForm } from "./JsonSchemaForm";

/** Discriminator over `config_methods[].kind`. */
function methodKind(method: ChannelPluginConfigMethod): string {
  return typeof method?.kind === "string" ? method.kind : "";
}

/**
 * Resolve the `config_methods[]` field into a renderable shape.
 *
 *   1. undefined (older gateway predating §11) → `[{kind:"form"}]`.
 *      Forward-compat — the universal baseline is a form.
 *   2. explicit empty array (gateway / plugin bug) → `"empty"`.
 *      The caller renders an error banner; silently rendering a
 *      form would mask the bug.
 *   3. non-empty → the array verbatim.
 */
function resolveConfigMethods(
  entry: ChannelPluginCatalogEntry,
): ChannelPluginConfigMethod[] | "empty" {
  const raw = entry.config_methods;
  if (raw === undefined) return [{ kind: "form" }];
  if (raw.length === 0) return "empty";
  return raw;
}

/**
 * Headless renderer. Caller controls `value` / `onChange`. No save
 * button, no dialog wrapping — pure sections.
 */
export interface PluginConfigSectionsProps {
  entry: ChannelPluginCatalogEntry;
  value: Record<string, unknown>;
  onChange: (next: Record<string, unknown>) => void;
  secretInputType?: "password" | "text";
  showAutoLoginMethod?: boolean;
}

export function PluginConfigSections(
  props: PluginConfigSectionsProps,
): ReactElement {
  const {
    entry,
    value,
    onChange,
    secretInputType = "password",
    showAutoLoginMethod = true,
  } = props;
  const [showAutoLogin, setShowAutoLogin] = useState(false);
  const methods = useMemo(() => resolveConfigMethods(entry), [entry]);

  const handleAutoLoginConfirmed = useCallback(
    (values: Record<string, unknown>) => {
      // Auto-login values take precedence over anything the user
      // typed — they're authoritative for canonical identifiers
      // (app_id / token / account_id).
      onChange({ ...value, ...values });
      setShowAutoLogin(false);
    },
    [onChange, value],
  );

  if (methods === "empty") {
    return (
      <div className="rounded-md border border-[#eeeeee] bg-[#fafaf9] p-4 text-sm text-red-700">
        该插件未声明任何配置方法（<code>config_methods</code> 为空）。
        这是插件或网关的配置错误，请检查 manifest 或联系插件作者。
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      {methods.map((method, idx) => {
        switch (methodKind(method)) {
          case "form":
            return (
              <section
                key={`form-${idx}`}
                className="flex flex-col gap-3 rounded-md border border-[#eeeeee] bg-white p-4"
              >
                <h4 className="text-sm font-medium text-neutral-900">
                  手动填写
                </h4>
                <JsonSchemaForm
                  schema={entry.schema as Record<string, unknown>}
                  value={value}
                  onChange={onChange}
                  secretInputType={secretInputType}
                />
              </section>
            );
          case "auto_login":
            if (!showAutoLoginMethod) {
              return null;
            }
            return (
              <section
                key={`auto-${idx}`}
                className="flex flex-col gap-3 rounded-md border border-[#eeeeee] bg-white p-4"
              >
                <div className="flex items-center justify-between">
                  <h4 className="text-sm font-medium text-neutral-900">
                    一键登录
                  </h4>
                  {!showAutoLogin && (
                    <button
                      type="button"
                      onClick={() => setShowAutoLogin(true)}
                      className="rounded-md bg-[#2e7d32] px-3 py-1.5 text-sm text-white"
                    >
                      开始登录
                    </button>
                  )}
                </div>
                {showAutoLogin && (
                  <AuthFlowDriver
                    pluginId={entry.id}
                    formState={value}
                    iconDataUrl={entry.icon_data_url}
                    onConfirmed={handleAutoLoginConfirmed}
                    onCancel={() => setShowAutoLogin(false)}
                  />
                )}
                <p className="text-xs text-neutral-500">
                  登录成功后，上方表单会自动填入获取到的账号信息，你可以再次检查后保存。
                </p>
              </section>
            );
          default:
            // Unknown method — forward-compat skip.
            return null;
        }
      })}
    </div>
  );
}

/**
 * Standalone panel — owns its form state and renders a save button.
 * Use this when the plugin's config is the only thing the user is
 * editing. For dialogs that mix plugin config with outer fields
 * (AddBotDialog / EditBotDialog), use `PluginConfigSections` and
 * let the dialog manage state + save flow.
 */
export interface PluginConfigPanelProps {
  entry: ChannelPluginCatalogEntry;
  initialValue?: Record<string, unknown>;
  onSave: (values: Record<string, unknown>) => Promise<void> | void;
  onCancel?: () => void;
}

export function PluginConfigPanel(props: PluginConfigPanelProps): ReactElement {
  const { entry, initialValue = {}, onSave, onCancel } = props;
  const [value, setValue] = useState<Record<string, unknown>>(initialValue);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const handleSave = useCallback(async () => {
    setSaving(true);
    setSaveError(null);
    try {
      await onSave(value);
    } catch (err) {
      setSaveError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  }, [value, onSave]);

  return (
    <div className="flex flex-col gap-4">
      <header className="flex items-center gap-3">
        {entry.icon_data_url ? (
          <img
            src={entry.icon_data_url}
            alt=""
            width={28}
            height={28}
            className="rounded-md border border-[#eeeeee] bg-white"
          />
        ) : null}
        <div className="flex flex-col">
          <div className="text-base font-semibold text-neutral-900">
            {entry.display_name || entry.id}
          </div>
          {entry.description ? (
            <div className="text-xs text-neutral-500">{entry.description}</div>
          ) : null}
        </div>
      </header>

      <PluginConfigSections entry={entry} value={value} onChange={setValue} />

      <footer className="flex items-center justify-end gap-2 pt-2">
        {saveError ? (
          <span className="text-sm text-red-700 mr-auto">{saveError}</span>
        ) : null}
        {onCancel && (
          <button
            type="button"
            onClick={onCancel}
            disabled={saving}
            className="rounded-md border border-[#eeeeee] px-3 py-1.5 text-sm text-neutral-700 disabled:opacity-50"
          >
            取消
          </button>
        )}
        <button
          type="button"
          onClick={() => void handleSave()}
          disabled={saving}
          className="rounded-md bg-[#2e7d32] px-3 py-1.5 text-sm text-white disabled:opacity-50"
        >
          {saving ? "保存中…" : "保存"}
        </button>
      </footer>
    </div>
  );
}
