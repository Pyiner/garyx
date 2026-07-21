import type {
  DesktopGatewayMutationResult,
  DesktopState,
  GaryxDesktopApi,
} from '@shared/contracts';

import {
  requestDesktopState,
  requestDesktopStateResult,
} from '../pinned-order-ingress';

const stateMethodNames = [
  'getState',
  'getStateFast',
  'saveSettings',
  'rememberGatewayProfile',
  'addGatewayProfile',
  'updateGatewayProfile',
  'deleteGatewayProfile',
  'selectWorkspace',
  'removeWorkspace',
  'pinWorkspace',
  'renameWorkspace',
  'selectAutomation',
  'markAutomationSeen',
  'deleteAutomation',
  'addChannelAccount',
  'setBotBinding',
  'bindChannelEndpoint',
  'detachChannelEndpoint',
  'renameThread',
  'setThreadPinned',
  'setThreadPinOrder',
] as const satisfies readonly (keyof GaryxDesktopApi)[];
const stateMethods: ReadonlySet<PropertyKey> = new Set(stateMethodNames);

const stateResultMethodNames = [
  'addWorkspaceByPath',
  'createAutomation',
  'updateAutomation',
  'runAutomationNow',
  'createThread',
] as const satisfies readonly (keyof GaryxDesktopApi)[];
const stateResultMethods: ReadonlySet<PropertyKey> = new Set(
  stateResultMethodNames,
);

// Lifecycle mutations return DesktopGatewayMutationResult<DesktopState>:
// the authoritative state is NESTED in the applied variant's `value`, so
// they need their own stamping lane — treating the wrapper as a state (the
// old stateMethods lane) stamps garbage and crashes identity reads.
const mutationResultMethodNames = [
  'archiveThread',
  'deleteThread',
] as const satisfies readonly (keyof GaryxDesktopApi)[];
const mutationResultMethods: ReadonlySet<PropertyKey> = new Set(
  mutationResultMethodNames,
);

type DesktopApiMethod = (...args: unknown[]) => unknown;

type DesktopApiStateIngress = Readonly<{
  requestState: (
    request: () => Promise<DesktopState>,
  ) => Promise<DesktopState>;
  requestStateResult: <Result>(
    request: () => Promise<Result>,
    selectState: (result: Result) => DesktopState | null,
  ) => Promise<Result>;
}>;

function createFacadeMethod(
  rawApi: GaryxDesktopApi,
  property: PropertyKey,
  method: DesktopApiMethod,
  ingress: DesktopApiStateIngress,
): (...args: unknown[]) => unknown {
  const invoke = (args: unknown[]) => Reflect.apply(method, rawApi, args);
  if (stateMethods.has(property)) {
    return (...args) => ingress.requestState(
      () => invoke(args) as Promise<DesktopState>,
    );
  }
  if (stateResultMethods.has(property)) {
    return (...args) => ingress.requestStateResult(
      () => invoke(args) as Promise<{ state: DesktopState }>,
      (result) => result.state,
    );
  }
  if (mutationResultMethods.has(property)) {
    // The facade sees the RAW transport result (shared/contracts/thread.ts):
    // "ok" carries the authoritative state; "definitiveEndpointResponse"
    // may carry one too; "ambiguous"/"notSent" carry nothing to stamp.
    return (...args) => ingress.requestStateResult(
      () => invoke(args) as Promise<DesktopGatewayMutationResult<DesktopState>>,
      (result): DesktopState | null =>
        result.kind === 'ok' || result.kind === 'definitiveEndpointResponse'
          ? result.value
          : null,
    );
  }
  return (...args) => invoke(args);
}

/**
 * Materializes an immutable renderer-side facade over Electron's bridge API.
 *
 * contextBridge exposes a frozen cross-context object. A Proxy cannot replace
 * values for that object's non-configurable, non-writable method properties,
 * so interception has to live on a separate ordinary object. Materializing
 * once also gives every delegated method a stable identity.
 */
export function createDesktopApiFacade(
  rawApi: GaryxDesktopApi,
  ingress: DesktopApiStateIngress,
): GaryxDesktopApi {
  const facade = {};
  for (const property of Reflect.ownKeys(rawApi)) {
    const value = Reflect.get(rawApi, property, rawApi) as unknown;
    Object.defineProperty(facade, property, {
      enumerable: Object.prototype.propertyIsEnumerable.call(rawApi, property),
      value: typeof value === 'function'
        ? createFacadeMethod(
          rawApi,
          property,
          value as DesktopApiMethod,
          ingress,
        )
        : value,
    });
  }
  return Object.freeze(facade) as GaryxDesktopApi;
}

const desktopApiStateIngress: DesktopApiStateIngress = {
  requestState: requestDesktopState,
  requestStateResult: requestDesktopStateResult,
};

let cachedRawApi: GaryxDesktopApi | null = null;
let cachedFacadeApi: GaryxDesktopApi | null = null;

export function getDesktopApi(): GaryxDesktopApi {
  const rawApi = window.garyxDesktop;
  if (cachedRawApi === rawApi && cachedFacadeApi) {
    return cachedFacadeApi;
  }
  cachedRawApi = rawApi;
  cachedFacadeApi = createDesktopApiFacade(rawApi, desktopApiStateIngress);
  return cachedFacadeApi;
}
