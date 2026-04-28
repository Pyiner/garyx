import type { GaryxDesktopApi } from '@shared/contracts';

export function getDesktopApi(): GaryxDesktopApi {
  return window.garyxDesktop;
}
