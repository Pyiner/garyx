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
}

const electronWorkspaceDataAdapter: WorkspaceDataAdapter = {
  listDirectories(input) {
    return window.garyxDesktop.listWorkspaceDirectories(input);
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
