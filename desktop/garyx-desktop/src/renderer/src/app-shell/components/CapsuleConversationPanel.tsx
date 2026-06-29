import { Maximize2, Minimize2, X } from "lucide-react";

import { useI18n } from "../../i18n";
import { CapsuleLivePreviewFrame } from "./CapsuleLivePreviewFrame";

export function CapsuleConversationPanel({
  capsuleId,
  revision,
  title,
  isFullscreen,
  onToggleFullscreen,
  onClose,
}: {
  capsuleId: string;
  revision: number;
  title: string;
  isFullscreen: boolean;
  onToggleFullscreen: () => void;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const fullscreenLabel = isFullscreen
    ? t("Exit fullscreen")
    : t("View fullscreen");
  const FullscreenIcon = isFullscreen ? Minimize2 : Maximize2;

  return (
    <aside aria-label={title} className="capsule-conversation-panel">
      <header className="capsule-preview-toolbar">
        <span className="capsule-preview-title">{title}</span>
        <div className="capsule-preview-toolbar-actions">
          <button
            aria-label={fullscreenLabel}
            aria-pressed={isFullscreen}
            className="capsule-toolbar-button"
            onClick={onToggleFullscreen}
            title={fullscreenLabel}
            type="button"
          >
            <FullscreenIcon size={16} />
          </button>
          <button
            aria-label={t("Close")}
            className="capsule-toolbar-button"
            onClick={onClose}
            title={t("Close")}
            type="button"
          >
            <X size={16} />
          </button>
        </div>
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
