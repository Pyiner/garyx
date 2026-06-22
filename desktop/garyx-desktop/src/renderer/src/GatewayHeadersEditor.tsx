import { useEffect, useRef, useState } from 'react';
import { Plus, Trash2 } from 'lucide-react';

import {
  formatGatewayHeaderEntries,
  parseGatewayHeaderEntries,
  type GatewayHeaderEntry,
} from '@shared/gateway-headers';

import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { useI18n } from './i18n';

type GatewayHeaderRow = GatewayHeaderEntry & {
  id: string;
};

type GatewayHeadersEditorProps = {
  value: string;
  onChange: (value: string) => void;
  className?: string;
};

let nextHeaderRowId = 0;

function createHeaderRow(entry?: Partial<GatewayHeaderEntry>): GatewayHeaderRow {
  nextHeaderRowId += 1;
  return {
    id: `gateway-header-${nextHeaderRowId}`,
    name: entry?.name ?? '',
    value: entry?.value ?? '',
  };
}

function rowsFromValue(value: string): GatewayHeaderRow[] {
  const rows = parseGatewayHeaderEntries(value).map((entry) => createHeaderRow(entry));
  return rows.length > 0 ? rows : [createHeaderRow()];
}

function rowsToValue(rows: readonly GatewayHeaderRow[]): string {
  return formatGatewayHeaderEntries(rows);
}

export function GatewayHeadersEditor({
  value,
  onChange,
  className,
}: GatewayHeadersEditorProps) {
  const { t } = useI18n();
  const lastValueRef = useRef(value);
  const [rows, setRows] = useState<GatewayHeaderRow[]>(() => rowsFromValue(value));

  useEffect(() => {
    if (value === lastValueRef.current) {
      return;
    }
    lastValueRef.current = value;
    setRows(rowsFromValue(value));
  }, [value]);

  function emit(nextRows: GatewayHeaderRow[]) {
    setRows(nextRows);
    const nextValue = rowsToValue(nextRows);
    lastValueRef.current = nextValue;
    onChange(nextValue);
  }

  function updateRow(rowId: string, field: 'name' | 'value', nextValue: string) {
    emit(rows.map((row) => (row.id === rowId ? { ...row, [field]: nextValue } : row)));
  }

  function removeRow(rowId: string) {
    const nextRows = rows.filter((row) => row.id !== rowId);
    emit(nextRows.length > 0 ? nextRows : [createHeaderRow()]);
  }

  function addRow() {
    setRows((current) => [...current, createHeaderRow()]);
  }

  return (
    <div className={['gateway-headers-editor', className].filter(Boolean).join(' ')}>
      <div className="gateway-headers-editor-list">
        {rows.map((row) => (
          <div className="gateway-headers-editor-row" key={row.id}>
            <Input
              autoCapitalize="off"
              autoComplete="off"
              aria-label={t('Header name')}
              className="gateway-headers-editor-input"
              placeholder={t('Header name')}
              spellCheck={false}
              type="text"
              value={row.name}
              onChange={(event) => updateRow(row.id, 'name', event.target.value)}
            />
            <Input
              autoCapitalize="off"
              autoComplete="off"
              aria-label={t('Header value')}
              className="gateway-headers-editor-input"
              placeholder={t('Header value')}
              spellCheck={false}
              type="text"
              value={row.value}
              onChange={(event) => updateRow(row.id, 'value', event.target.value)}
            />
            <Button
              aria-label={t('Remove header')}
              className="gateway-headers-editor-remove"
              disabled={rows.length === 1 && !row.name.trim() && !row.value.trim()}
              onClick={() => removeRow(row.id)}
              size="icon-sm"
              type="button"
              variant="ghost"
            >
              <Trash2 aria-hidden />
            </Button>
          </div>
        ))}
      </div>
      <Button
        className="gateway-headers-editor-add"
        onClick={addRow}
        size="sm"
        type="button"
        variant="outline"
      >
        <Plus aria-hidden />
        {t('Add header')}
      </Button>
    </div>
  );
}
