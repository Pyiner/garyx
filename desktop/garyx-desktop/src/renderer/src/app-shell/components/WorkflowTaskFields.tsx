import type { DesktopWorkflowDefinition } from '@shared/contracts';

import { Field, FieldLabel } from '../../components/ui/field';
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from '../../components/ui/select';
import type { Translate } from '../../i18n';

const CHOOSE_WORKFLOW_VALUE = '__choose_workflow__';

type WorkflowTaskFieldsProps = {
  definitions: DesktopWorkflowDefinition[];
  loading: boolean;
  error: string | null;
  selectedWorkflowId: string;
  onSelectWorkflow: (workflowId: string) => void;
  t: Translate;
};

export function WorkflowTaskFields({
  definitions,
  loading,
  error,
  selectedWorkflowId,
  onSelectWorkflow,
  t,
}: WorkflowTaskFieldsProps) {
  const selected = definitions.find(
    (definition) => definition.workflowId === selectedWorkflowId,
  );

  if (loading) {
    return (
      <div className="tasks-workflow-state">{t('Loading workflows…')}</div>
    );
  }

  if (error) {
    return (
      <div className="tasks-workflow-state tasks-workflow-state-error">
        {error}
      </div>
    );
  }

  if (!definitions.length) {
    return (
      <div className="tasks-workflow-empty">
        <p className="tasks-workflow-empty-title">
          {t('No workflow definitions installed')}
        </p>
        <p className="tasks-workflow-empty-hint">
          {t('Install one with')}{' '}
          <code>garyx workflow definition upsert --file &lt;path&gt;</code>
        </p>
      </div>
    );
  }

  return (
    <>
      <Field className="tasks-field tasks-field-full">
        <FieldLabel>{t('Workflow')}</FieldLabel>
        <Select
          value={selectedWorkflowId || CHOOSE_WORKFLOW_VALUE}
          onValueChange={(value) => {
            onSelectWorkflow(value === CHOOSE_WORKFLOW_VALUE ? '' : value);
          }}
        >
          <SelectTrigger>
            <SelectValue />
          </SelectTrigger>
          <SelectContent
            className="tasks-create-select-content"
            position="popper"
            sideOffset={4}
          >
            <SelectGroup>
              <SelectLabel>{t('Workflow')}</SelectLabel>
              <SelectItem value={CHOOSE_WORKFLOW_VALUE}>
                {t('Choose a workflow')}
              </SelectItem>
              {definitions.map((definition) => (
                <SelectItem
                  key={definition.workflowId}
                  textValue={definition.name}
                  value={definition.workflowId}
                >
                  {definition.name}
                </SelectItem>
              ))}
            </SelectGroup>
          </SelectContent>
        </Select>
        {selected?.description ? (
          <p className="tasks-workflow-description">{selected.description}</p>
        ) : null}
      </Field>
    </>
  );
}
