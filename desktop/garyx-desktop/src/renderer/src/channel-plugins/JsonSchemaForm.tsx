/**
 * Minimal JSON Schema → React form renderer for channel plugin
 * account configuration.
 *
 * Handles the subset of JSON Schema (2020-12) that plugins
 * actually use: object-rooted schemas with `properties`,
 * `required`, and `enum` / `default` on leaf fields. Covers:
 *
 *   - `type: string` → `<input type="text">`, `<input type="password">`
 *     when `x-garyx.secret === true`, `<select>` when `enum` is set.
 *   - `type: boolean` → `<input type="checkbox">`.
 *   - `type: integer` / `number` → `<input type="number">`.
 *   - `type: array` with `items.type === "string"` → simple comma-
 *     or newline-separated textarea (the common shape for
 *     `allow_from` lists).
 *   - Nested `type: object` → recursive sub-form under a labelled
 *     fieldset.
 *
 * Unknown shapes (e.g. `oneOf`, `$ref`) fall back to a raw JSON
 * textarea so the user can still edit the value; a banner warns the
 * operator that the UI couldn't synthesize a proper control.
 *
 * This renderer is intentionally style-light: it emits semantic
 * HTML with labelled stable class names. The settings panel's CSS
 * picks them up to match the existing Notion / Linear-influenced
 * aesthetic.
 */
import { useCallback, useMemo, type ReactElement, type ReactNode } from "react";

import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { useI18n, type Translate } from "@/i18n";

type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [key: string]: JsonValue };

interface JsonSchemaNode {
  type?: string;
  description?: string;
  default?: JsonValue;
  enum?: JsonValue[];
  required?: string[];
  properties?: Record<string, JsonSchemaNode>;
  items?: JsonSchemaNode;
  minimum?: number;
  maximum?: number;
  "x-garyx"?: { secret?: boolean };
}

export interface JsonSchemaFormProps {
  schema: Record<string, unknown>;
  value: Record<string, unknown>;
  onChange: (next: Record<string, unknown>) => void;
  /** How fields marked `x-garyx.secret` should render when their
   * actual value is available. Add flows keep password-style entry;
   * edit flows can opt into plain text so operators can inspect the
   * saved token directly. */
  secretInputType?: "password" | "text";
  /** Show a "Secrets redacted" placeholder instead of the real
   * value for fields marked `x-garyx.secret`. Used on the Edit
   * form when we don't want to round-trip the server's stored
   * token through the UI. Defaults to false so a user editing a
   * field keeps what they typed. */
  redactSecrets?: boolean;
  disabled?: boolean;
}

export function JsonSchemaForm({
  schema,
  value,
  onChange,
  secretInputType = "password",
  redactSecrets = false,
  disabled = false,
}: JsonSchemaFormProps): ReactElement {
  const { t } = useI18n();
  const node = schema as unknown as JsonSchemaNode;
  const required = useMemo(
    () => new Set(Array.isArray(node.required) ? node.required : []),
    [node.required],
  );
  const properties = node.properties ?? {};

  const updateField = useCallback(
    (key: string, next: JsonValue) => {
      onChange({ ...value, [key]: next });
    },
    [onChange, value],
  );

  // Stable ordering: `required` fields first, then the rest in the
  // schema's declaration order. Keeps the UI predictable even when
  // the gateway reorders its keys.
  const entries = useMemo(() => {
    const keys = Object.keys(properties);
    keys.sort((a, b) => {
      const ar = required.has(a) ? 0 : 1;
      const br = required.has(b) ? 0 : 1;
      if (ar !== br) return ar - br;
      return 0;
    });
    return keys.map((k) => [k, properties[k]] as const);
  }, [properties, required]);

  if (entries.length === 0) {
    return (
      <div className="json-schema-form-empty">
        {t("This plugin declares no configurable fields.")}
      </div>
    );
  }

  return (
    <div className="json-schema-form">
      {entries.map(([key, fieldSchema]) => (
        <SchemaField
          key={key}
          fieldKey={key}
          schema={fieldSchema}
      value={(value as Record<string, JsonValue>)[key]}
      required={required.has(key)}
      disabled={disabled}
      secretInputType={secretInputType}
      redactSecrets={redactSecrets}
      fieldId={`json-schema-field-${key}`}
      onChange={(next) => updateField(key, next)}
    />
  ))}
    </div>
  );
}

interface SchemaFieldProps {
  fieldKey: string;
  fieldId: string;
  schema: JsonSchemaNode;
  value: JsonValue | undefined;
  required: boolean;
  disabled: boolean;
  secretInputType: "password" | "text";
  redactSecrets: boolean;
  onChange: (next: JsonValue) => void;
}

function SchemaField({
  fieldKey,
  fieldId,
  schema,
  value,
  required,
  disabled,
  secretInputType,
  redactSecrets,
  onChange,
}: SchemaFieldProps): ReactElement {
  const { t } = useI18n();
  const label = prettifyKey(fieldKey);
  const description = schema.description;
  const isSecret = schema["x-garyx"]?.secret === true;

  // Scalar with an enum collapses to a <select> regardless of type.
  if (Array.isArray(schema.enum) && schema.enum.length > 0) {
    const current =
      value !== undefined && value !== null
        ? String(value)
        : schema.default !== undefined
          ? String(schema.default)
          : "";
    return (
      <LabelledField label={label} description={description} required={required}>
        <Select
          disabled={disabled}
          value={current || undefined}
          onValueChange={(next) => onChange(next)}
        >
          <SelectTrigger id={fieldId} className="w-full bg-white">
            <SelectValue placeholder={t("Choose...")} />
          </SelectTrigger>
          <SelectContent>
            {schema.enum.map((option) => {
              const text = String(option);
              return (
                <SelectItem key={text} value={text}>
                  {text}
                </SelectItem>
              );
            })}
          </SelectContent>
        </Select>
      </LabelledField>
    );
  }

  switch (schema.type) {
    case "boolean": {
      const current =
        typeof value === "boolean"
          ? value
          : typeof schema.default === "boolean"
            ? schema.default
            : false;
      return (
        <LabelledField
          label={label}
          description={description}
          required={required}
          inline
        >
          <Checkbox
            id={fieldId}
            disabled={disabled}
            checked={current}
            onCheckedChange={(next) => onChange(next === true)}
          />
        </LabelledField>
      );
    }
    case "integer":
    case "number": {
      const current =
        typeof value === "number"
          ? String(value)
          : typeof schema.default === "number"
            ? String(schema.default)
            : "";
      return (
        <LabelledField label={label} description={description} required={required}>
          <Input
            id={fieldId}
            disabled={disabled}
            type="number"
            min={schema.minimum}
            max={schema.maximum}
            value={current}
            onChange={(e) => {
              const raw = e.target.value;
              if (raw === "") {
                onChange("");
                return;
              }
              const parsed =
                schema.type === "integer" ? parseInt(raw, 10) : parseFloat(raw);
              onChange(Number.isFinite(parsed) ? parsed : raw);
            }}
          />
        </LabelledField>
      );
    }
    case "array": {
      const isStringArray = schema.items?.type === "string";
      if (!isStringArray) {
        return unsupported(label, schema, value, onChange, t);
      }
      const list = Array.isArray(value) ? (value as string[]) : [];
      const joined = list.join("\n");
      return (
        <LabelledField label={label} description={description} required={required}>
          <Textarea
            id={fieldId}
            disabled={disabled}
            value={joined}
            placeholder={t("One per line")}
            rows={Math.min(8, Math.max(2, list.length + 1))}
            onChange={(e) => {
              const next = e.target.value
                .split(/[\r\n]+/)
                .map((s) => s.trim())
                .filter(Boolean);
              onChange(next);
            }}
          />
        </LabelledField>
      );
    }
    case "object": {
      const child = (value && typeof value === "object" && !Array.isArray(value)
        ? (value as Record<string, JsonValue>)
        : {});
      return (
        <fieldset className="rounded-2xl border border-[#ecece8] bg-[#fafaf8] p-4">
          <legend className="px-1 text-sm font-semibold text-neutral-900">
            {label}
            {description ? (
              <span className="ml-2 text-xs font-normal text-neutral-500">
                {description}
              </span>
            ) : null}
          </legend>
          <JsonSchemaForm
            schema={schema as unknown as Record<string, unknown>}
            value={child}
            disabled={disabled}
            secretInputType={secretInputType}
            redactSecrets={redactSecrets}
            onChange={(next) => onChange(next as unknown as JsonValue)}
          />
        </fieldset>
      );
    }
    case "string":
    default: {
      const current =
        typeof value === "string"
          ? value
          : typeof schema.default === "string"
            ? schema.default
            : "";
      if (isSecret && redactSecrets && current) {
        return (
          <LabelledField
            label={label}
            description={description}
            required={required}
          >
            <Input
              id={fieldId}
              disabled={disabled}
              type="password"
              value="••••••••"
              placeholder={t("Click to replace")}
              onFocus={(e) => {
                e.target.value = "";
                onChange("");
              }}
              onChange={(e) => onChange(e.target.value)}
            />
          </LabelledField>
        );
      }
      return (
        <LabelledField label={label} description={description} required={required}>
          <Input
            id={fieldId}
            disabled={disabled}
            type={isSecret ? secretInputType : "text"}
            value={current}
            onChange={(e) => onChange(e.target.value)}
          />
        </LabelledField>
      );
    }
  }
}

function unsupported(
  label: string,
  schema: JsonSchemaNode,
  value: JsonValue | undefined,
  onChange: (next: JsonValue) => void,
  t: Translate,
): ReactElement {
  const asText =
    value === undefined ? "" : JSON.stringify(value, null, 2);
  return (
    <LabelledField label={label} description={schema.description}>
      <div className="rounded-xl border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-900">
        {t("This field's schema is not rendered natively. Edit the raw JSON below.")}
      </div>
      <Textarea
        value={asText}
        rows={4}
        onChange={(e) => {
          try {
            onChange(JSON.parse(e.target.value));
          } catch {
            /* let the user keep typing; parse errors are normal mid-edit */
            onChange(e.target.value as unknown as JsonValue);
          }
        }}
      />
    </LabelledField>
  );
}

interface LabelledFieldProps {
  label: string;
  description?: string;
  required?: boolean;
  inline?: boolean;
  children: ReactNode;
}

function LabelledField({
  label,
  description,
  required,
  inline,
  children,
}: LabelledFieldProps): ReactElement {
  if (inline) {
    return (
      <div className="flex flex-col gap-2 rounded-xl border border-[#ededeb] bg-[#fcfcfb] px-3 py-3">
        <div className="flex items-start gap-3">
          {children}
          <div className="flex min-w-0 flex-col gap-1">
            <Label className="text-sm font-medium text-neutral-900">
              {label}
              {required ? (
                <span className="text-[13px] text-red-600">*</span>
              ) : null}
            </Label>
            {description ? (
              <p className="text-sm leading-6 text-neutral-700">{description}</p>
            ) : null}
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-2">
      <Label className="text-sm font-medium text-neutral-900">
        {label}
        {required ? <span className="text-[13px] text-red-600">*</span> : null}
      </Label>
      {children}
      {description ? (
        <p className="text-sm leading-6 text-neutral-700">{description}</p>
      ) : null}
    </div>
  );
}

/**
 * Turn a JSON Schema property key into a Humane label.
 * `base_url` → `Base url`, `app_id` → `App id`, etc.
 * Deliberately simple; future work could take hints from a
 * `title` field on the schema.
 */
function prettifyKey(key: string): string {
  if (!key) return key;
  const spaced = key.replace(/[_-]+/g, " ").trim();
  return spaced.charAt(0).toUpperCase() + spaced.slice(1);
}
