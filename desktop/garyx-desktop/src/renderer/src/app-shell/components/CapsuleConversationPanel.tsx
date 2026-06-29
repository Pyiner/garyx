import { X } from "lucide-react";

import { useI18n } from "../../i18n";
import { CapsuleLivePreviewFrame } from "./CapsuleLivePreviewFrame";

export function CapsuleConversationPanel({
  capsuleId,
  revision,
  title,
  onClose,
}: {
  capsuleId: string;
  revision: number;
  title: string;
  onClose: () => void;
}) {
  const { t } = useI18n();

  return (
    <aside aria-label={title} className="capsule-conversation-panel">
      <header className="capsule-preview-toolbar">
        <span className="capsule-preview-title">{title}</span>
        <button
          aria-label={t("Close")}
          className="capsule-toolbar-button"
          onClick={onClose}
          title={t("Close")}
          type="button"
        >
          <X size={16} />
        </button>
      </header>
      <div className="capsule-preview-body">
        <CapsuleLivePreviewFrame
          active
          capsuleId={capsuleId}
          mode="preview"
          revision={revision}
          title={title}
        />
      </div>
    </aside>
  );
}
