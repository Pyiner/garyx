import { createContext, useContext } from 'react';

import type {
  DesktopLocalDirectoryListing,
  DesktopWorkspace,
} from '@shared/contracts';

import { requestDesktopStateResult } from '../pinned-order-ingress';

/**
 * Platform boundary for the shared workspace picker surfaces. The picker
 * presentation is platform-free; data access goes through this adapter —
 * Electron surfaces default to the IPC bridge, the Web entry injects an
 * HTTP adapter over web-api.
 */
export interface WorkspaceDataAdapter {
  listDirectories(input?: {
    path?: string | null;
  }): Promise<DesktopLocalDirectoryListing>;
  addWorkspace(
    path: string,
    name?: string | null,
  ): Promise<DesktopWorkspace | null>;
  /** The gateway workspace catalog (server order, gateway home) for picker
   *  hosts whose surface does not already hold it. */
  listCatalog(): Promise<{
    workspaces: DesktopWorkspace[];
    gatewayHome: string | null;
  }>;
}

const electronWorkspaceDataAdapter: WorkspaceDataAdapter = {
  listDirectories(input) {
    return window.garyxDesktop.listWorkspaceDirectories(input);
  },
  async listCatalog() {
    const state = await window.garyxDesktop.getState();
    return {
      workspaces: state.workspaces,
      gatewayHome: state.gatewayHome ?? null,
    };
  },
  async addWorkspace(path, name) {
    const result = await requestDesktopStateResult(
      () => window.garyxDesktop.addWorkspaceByPath({ path, name }),
      (response) => response.state,
    );
    return result.workspace || null;
  },
};

export const WorkspaceDataAdapterContext =
  createContext<WorkspaceDataAdapter>(electronWorkspaceDataAdapter);

export function useWorkspaceDataAdapter(): WorkspaceDataAdapter {
  return useContext(WorkspaceDataAdapterContext);
}

/**
 * Gateway connection epoch for workspace surfaces. A change means a new
 * workspace universe: open pickers/browsers close their transient state and
 * drop cached catalogs (consumers reset in an effect keyed on this value).
 */
export const WorkspaceEpochContext = createContext<string>("");

export function useWorkspaceEpoch(): string {
  return useContext(WorkspaceEpochContext);
}
