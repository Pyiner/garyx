import { Check, CircleAlert, Info, X } from 'lucide-react';
import { useI18n } from './i18n';

export type ToastTone = 'success' | 'error' | 'info';

export type ToastItem = {
  id: number;
  message: string;
  tone: ToastTone;
};

type ToastViewportProps = {
  onDismiss: (id: number) => void;
  toasts: ToastItem[];
};

function ToastIcon({ tone }: { tone: ToastTone }) {
  switch (tone) {
    case 'success':
      return <Check size={16} strokeWidth={2} />;
    case 'error':
      return <CircleAlert size={16} strokeWidth={2} />;
    default:
      return <Info size={16} strokeWidth={2} />;
  }
}

export function ToastViewport({ onDismiss, toasts }: ToastViewportProps) {
  const { t } = useI18n();
  if (!toasts.length) {
    return null;
  }

  return (
    <div aria-live="polite" className="toast-viewport">
      {toasts.map((toast) => (
        <div
          className={`toast-card tone-${toast.tone}`}
          key={toast.id}
          role={toast.tone === 'error' ? 'alert' : 'status'}
        >
          <div className="toast-icon">
            <ToastIcon tone={toast.tone} />
          </div>
          <div className="toast-copy">
            <p>{toast.message}</p>
          </div>
          <button
            aria-label={t('Dismiss notification')}
            className="toast-dismiss"
            onClick={() => {
              onDismiss(toast.id);
            }}
            type="button"
          >
            <X size={14} strokeWidth={2} />
          </button>
        </div>
      ))}
    </div>
  );
}
