import { useRef } from 'react';

import type { RenderCapsuleCard } from '@shared/contracts';

import { useI18n } from '../../i18n';
import { useInViewport } from '../use-in-viewport';
import { CapsuleLivePreviewFrame } from './CapsuleLivePreviewFrame';

function CapsuleChatCard({
  card,
  onOpenCapsule,
}: {
  card: RenderCapsuleCard;
  onOpenCapsule?: (card: RenderCapsuleCard) => void;
}) {
  const { t } = useI18n();
  const ref = useRef<HTMLButtonElement | null>(null);
  const visible = useInViewport(ref);
  const title = card.title?.trim() || t('Untitled Capsule');
  return (
    <button
      ref={ref}
      className="capsule-chat-card"
      onClick={() => onOpenCapsule?.(card)}
      title={title}
      type="button"
    >
      <span className="capsule-card-preview-shell">
        <CapsuleLivePreviewFrame
          active={visible}
          capsuleId={card.capsule_id}
          mode="card"
          revision={card.revision}
          title={title}
        />
      </span>
      <span className="capsule-chat-card-meta">
        <span className="capsule-chat-card-title">{title}</span>
      </span>
    </button>
  );
}

/**
 * Renders the server-derived Capsule cards for a turn (dumb-render: the cards,
 * their order, and their placement after the final answer are all decided by
 * `render_state`). Clicking opens the in-app preview.
 */
export function CapsuleChatCardList({
  cards,
  onOpenCapsule,
}: {
  cards: RenderCapsuleCard[];
  onOpenCapsule?: (card: RenderCapsuleCard) => void;
}) {
  if (!cards.length) {
    return null;
  }
  return (
    <div className="capsule-chat-cards">
      {cards.map((card) => (
        <CapsuleChatCard card={card} key={card.id} onOpenCapsule={onOpenCapsule} />
      ))}
    </div>
  );
}
