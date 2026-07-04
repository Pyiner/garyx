// Workspace file tree (endgame architecture batch 5b, "Local state
// colocation list": WorkspaceFilesPanel owns directory expansion,
// preview, upload pending — this cut takes the filter + tree rendering;
// the directory/preview/upload state itself stays with
// useWorkspaceController until its own cut).
//
// Owns the file-filter text (per-keystroke filtering re-renders only
// this subtree) and the recursive node rendering + the hidden upload
// input, all verbatim from AppShell. The filter resets when the active
// workspace changes — the legacy AppShell effect cleared it alongside
// the expansion seeding, which stays with the controller.

import { useEffect, useState, type ReactNode, type RefObject } from "react";

import type { DesktopWorkspaceFileEntry } from "@shared/contracts";

import type { WorkspaceDirectoryState } from "../types";
import { workspaceDirectoryKey } from "../workspace-helpers";
import { WorkspaceFileIcon } from "../icons";

type WorkspaceFileTreeProps = {
  activeWorkspacePath: string | null;
  expandedWorkspaceDirectories: Record<string, boolean>;
  onActivateEntry: (entry: DesktopWorkspaceFileEntry) => void;
  onUploadFiles: (files: File[]) => void;
  selectedWorkspaceFile: { workspacePath: string; path: string } | null;
  workspaceDirectories: Record<string, WorkspaceDirectoryState>;
  workspaceUploadInputRef: RefObject<HTMLInputElement | null>;
  /**
   * Filter text lives here; the side-tools panel's filter input reads and
   * writes it through these render props to keep one owner.
   */
  children: (tree: ReactNode, filter: {
    value: string;
    onChange: (value: string) => void;
  }) => ReactNode;
};

export function WorkspaceFileTree({
  activeWorkspacePath,
  expandedWorkspaceDirectories,
  onActivateEntry,
  onUploadFiles,
  selectedWorkspaceFile,
  workspaceDirectories,
  workspaceUploadInputRef,
  children,
}: WorkspaceFileTreeProps) {
  const [workspaceFileFilter, setWorkspaceFileFilter] = useState("");

  // Legacy AppShell effect: switching (or clearing) the active workspace
  // resets the filter.
  useEffect(() => {
    setWorkspaceFileFilter("");
  }, [activeWorkspacePath]);

  const workspaceFileFilterQuery = workspaceFileFilter.trim().toLowerCase();

  function workspaceEntryMatchesFilter(
    workspacePath: string,
    entry: DesktopWorkspaceFileEntry,
  ): boolean {
    if (!workspaceFileFilterQuery) {
      return true;
    }
    const haystack = `${entry.name}\n${entry.path}`.toLowerCase();
    if (haystack.includes(workspaceFileFilterQuery)) {
      return true;
    }
    if (entry.entryType !== "directory") {
      return false;
    }
    const childKey = workspaceDirectoryKey(workspacePath, entry.path);
    const childEntries = workspaceDirectories[childKey]?.entries || [];
    return childEntries.some((child) =>
      workspaceEntryMatchesFilter(workspacePath, child),
    );
  }

  function renderWorkspaceFileNodes(
    workspacePath: string,
    directoryPath = "",
    depth = 0,
  ): ReactNode {
    const key = workspaceDirectoryKey(workspacePath, directoryPath);
    const state = workspaceDirectories[key];
    const entries = state?.entries || [];

    if (state?.loading && !entries.length) {
      return (
        <div
          className="workspace-file-empty"
          style={{ paddingLeft: `${depth * 14}px` }}
        >
          Loading…
        </div>
      );
    }

    if (state?.error && !entries.length) {
      return (
        <div
          className="workspace-file-empty workspace-file-error"
          style={{ paddingLeft: `${depth * 14}px` }}
        >
          {state.error}
        </div>
      );
    }

    if (!entries.length) {
      return null;
    }

    const nodes: ReactNode[] = [];

    nodes.push(
      ...entries.map((entry) => {
        if (!workspaceEntryMatchesFilter(workspacePath, entry)) {
          return null;
        }
        const childKey = workspaceDirectoryKey(workspacePath, entry.path);
        const isExpanded = expandedWorkspaceDirectories[childKey] === true;
        const shouldShowChildren =
          entry.entryType === "directory" &&
          (isExpanded || Boolean(workspaceFileFilterQuery));
        const isSelected =
          selectedWorkspaceFile?.workspacePath === workspacePath &&
          selectedWorkspaceFile?.path === entry.path;

        return (
          <div
            className="workspace-file-node-shell"
            key={`${workspacePath}:${entry.path}`}
          >
            <button
              className={`workspace-file-node ${isSelected ? "active" : ""}`}
              onClick={() => {
                onActivateEntry(entry);
              }}
              style={{ paddingLeft: `${10 + depth * 16}px` }}
              title={entry.path || entry.name}
              type="button"
            >
              <WorkspaceFileIcon entry={entry} open={isExpanded} />
              <span className="workspace-file-node-copy">
                <span className="workspace-file-node-name">{entry.name}</span>
              </span>
            </button>
            {shouldShowChildren ? (
              <div className="workspace-file-children">
                {renderWorkspaceFileNodes(workspacePath, entry.path, depth + 1)}
              </div>
            ) : null}
          </div>
        );
      }),
    );

    return nodes;
  }

  const tree = activeWorkspacePath ? (
    <>
      <input
        className="workspace-upload-input"
        multiple
        onChange={(event) => {
          const files = Array.from(event.target.files || []);
          if (!files.length) {
            return;
          }
          onUploadFiles(files);
          event.target.value = "";
        }}
        ref={workspaceUploadInputRef}
        tabIndex={-1}
        type="file"
      />
      <div
        className="workspace-directory-tree"
        onDragOver={(event) => {
          if (event.dataTransfer.types.includes("Files")) {
            event.preventDefault();
            event.dataTransfer.dropEffect = "copy";
          }
        }}
        onDrop={(event) => {
          const files = Array.from(event.dataTransfer.files || []);
          if (!files.length) {
            return;
          }
          event.preventDefault();
          event.stopPropagation();
          onUploadFiles(files);
        }}
      >
        {renderWorkspaceFileNodes(activeWorkspacePath)}
      </div>
    </>
  ) : null;

  return children(tree, {
    value: workspaceFileFilter,
    onChange: setWorkspaceFileFilter,
  });
}
