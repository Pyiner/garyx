import type { DesktopState, GaryxDesktopApi } from '@shared/contracts';

import {
  requestDesktopState,
  requestDesktopStateResult,
} from '../pinned-order-ingress';

const stateMethods = new Set<keyof GaryxDesktopApi>([
  'getState',
  'getStateFast',
  'saveSettings',
  'rememberGatewayProfile',
  'addGatewayProfile',
  'updateGatewayProfile',
  'deleteGatewayProfile',
  'selectWorkspace',
  'removeWorkspace',
  'selectAutomation',
  'markAutomationSeen',
  'deleteAutomation',
  'addChannelAccount',
  'setBotBinding',
  'bindChannelEndpoint',
  'detachChannelEndpoint',
  'renameThread',
  'archiveThread',
  'deleteThread',
  'setThreadPinned',
  'setThreadPinOrder',
]);

const stateResultMethods = new Set<keyof GaryxDesktopApi>([
  'addWorkspaceByPath',
  'createAutomation',
  'updateAutomation',
  'runAutomationNow',
  'createThread',
]);

let cachedRawApi: GaryxDesktopApi | null = null;
let cachedGuardedApi: GaryxDesktopApi | null = null;

export function getDesktopApi(): GaryxDesktopApi {
  const rawApi = window.garyxDesktop;
  if (cachedRawApi === rawApi && cachedGuardedApi) {
    return cachedGuardedApi;
  }
  cachedRawApi = rawApi;
  cachedGuardedApi = new Proxy(rawApi, {
    get(target, property, receiver) {
      const value = Reflect.get(target, property, receiver) as unknown;
      if (typeof value !== 'function') {
        return value;
      }
      if (stateMethods.has(property as keyof GaryxDesktopApi)) {
        return (...args: unknown[]) => requestDesktopState(
          () => Reflect.apply(value, target, args) as Promise<DesktopState>,
        );
      }
      if (stateResultMethods.has(property as keyof GaryxDesktopApi)) {
        return (...args: unknown[]) => requestDesktopStateResult(
          () => Reflect.apply(value, target, args) as Promise<{ state: DesktopState }>,
          (result) => result.state,
        );
      }
      return value.bind(target);
    },
  });
  return cachedGuardedApi;
}
