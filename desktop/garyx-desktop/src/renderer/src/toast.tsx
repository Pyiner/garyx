import { IconAlertCircle, IconCheck, IconInfoCircle, IconX } from '@tabler/icons-react';

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
      return <IconCheck size={16} stroke={2} />;
    case 'error':
      return <IconAlertCircle size={16} stroke={2} />;
    default:
      return <IconInfoCircle size={16} stroke={2} />;
  }
}

export function ToastViewport({ onDismiss, toasts }: ToastViewportProps) {
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
            aria-label="Dismiss notification"
            className="toast-dismiss"
            onClick={() => {
              onDismiss(toast.id);
            }}
            type="button"
          >
            <IconX size={14} stroke={2} />
          </button>
        </div>
      ))}
    </div>
  );
}
