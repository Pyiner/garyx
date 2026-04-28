/// <reference types="vite/client" />

import type { GaryxDesktopApi } from '@shared/contracts';

declare global {
  interface Window {
    garyxDesktop: GaryxDesktopApi;
  }
}

export {};
