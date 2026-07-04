import { AppShell } from './app-shell/AppShell';
import { ToastProvider } from './toast-provider';

export function App() {
  return (
    <ToastProvider>
      <AppShell />
    </ToastProvider>
  );
}
