import type {
  DesktopWorkflowDefinition,
  DesktopWorkflowSourceDocument,
} from '@shared/contracts';

import { Badge } from '../../components/ui/badge';
import { Button } from '../../components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '../../components/ui/dialog';
import { useI18n } from '../../i18n';
import { workflowInputPlaceholder } from './agents-hub-helpers';
import type { WorkflowDialogMode } from './agents-hub-helpers';

type WorkflowViewDialogProps = {
  closeWorkflowDialog: () => void;
  loadWorkflowSource: (workflowId: string) => Promise<void>;
  selectedWorkflow: DesktopWorkflowDefinition | null;
  workflowDialogMode: WorkflowDialogMode;
  workflowSource: DesktopWorkflowSourceDocument | null;
  workflowSourceError: string | null;
  workflowSourceLoading: boolean;
};

export function WorkflowViewDialog({
  closeWorkflowDialog,
  loadWorkflowSource,
  selectedWorkflow,
  workflowDialogMode,
  workflowSource,
  workflowSourceError,
  workflowSourceLoading,
}: WorkflowViewDialogProps) {
  const { t } = useI18n();

  return (
    <Dialog
      open={Boolean(workflowDialogMode)}
      onOpenChange={(open) => {
        if (!open) {
          closeWorkflowDialog();
        }
      }}
    >
      <DialogContent className="agents-hub-agent-dialog agents-hub-workflow-dialog" size="viewer">
        <DialogHeader className="agents-hub-dialog-header">
          <DialogDescription className="agents-hub-dialog-kicker">
            {t('Workflow')}
          </DialogDescription>
          <DialogTitle className="agents-hub-dialog-title">
            {selectedWorkflow?.name || t('Workflow definition')}
          </DialogTitle>
          <DialogDescription className="agents-hub-dialog-description">
            {selectedWorkflow?.description || t('File-backed workflow definition installed for task execution.')}
          </DialogDescription>
        </DialogHeader>

        <div className="agents-hub-dialog-stack">
          <div className="agents-hub-workflow-meta-strip">
            <Badge variant="outline">v{selectedWorkflow?.version || 1}</Badge>
            <span>{selectedWorkflow?.workflowId || ''}</span>
          </div>

          <div className="agents-hub-workflow-source">
            <div className="agents-hub-workflow-source-header">
              <div>
                <div className="agents-hub-detail-label">{t('Source')}</div>
                <div className="agents-hub-workflow-source-path">
                  {workflowSource?.path || './workflow.ts'}
                </div>
              </div>
              <div className="agents-hub-card-badges">
                {workflowSource?.language ? <Badge variant="outline">{workflowSource.language}</Badge> : null}
                {workflowSource?.mediaType ? <Badge variant="outline">{workflowSource.mediaType}</Badge> : null}
              </div>
            </div>
            <pre className={`agents-hub-workflow-code ${workflowSourceError ? 'error' : ''}`}>
              <code>
                {workflowSourceLoading
                  ? t('Loading source...')
                  : workflowSourceError
                    ? workflowSourceError
                    : workflowSource?.content || t('(empty)')}
              </code>
            </pre>
          </div>

          <div className="agents-hub-workflow-footnote">
            <span>
              {t('Input')}: {selectedWorkflow
                ? workflowInputPlaceholder(selectedWorkflow) || t('Plain text input')
                : t('Plain text input')}
            </span>
            <Button
              disabled={workflowSourceLoading}
              onClick={() => {
                if (selectedWorkflow) {
                  void loadWorkflowSource(selectedWorkflow.workflowId);
                }
              }}
              size="sm"
              type="button"
              variant="ghost"
            >
              {t('Refresh')}
            </Button>
            <Button
              onClick={closeWorkflowDialog}
              size="sm"
              type="button"
              variant="outline"
            >
              {t('Close')}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
