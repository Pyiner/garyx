/// <reference types="vite/client" />

import type { GaryxDesktopApi } from '@shared/contracts';
import type { RendererPerformanceSnapshot } from './perf-metrics';

declare global {
  interface Window {
    __garyxPerformanceMonitor?: {
      getSnapshot: () => RendererPerformanceSnapshot;
      isDesktopApiMonitorInstalled: () => boolean;
    };
    garyxDesktop: GaryxDesktopApi;
  }
}

export {};
