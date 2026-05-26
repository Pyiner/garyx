import type { DesktopWorkspace } from '@shared/contracts';

import { WorkspacePathPicker } from './WorkspacePathPicker';

type DirectoryInputProps = {
  value: string;
  onChange: (next: string) => void;
  workspaces?: DesktopWorkspace[];
  id?: string;
  placeholder?: string;
  onAddWorkspace?: (path: string) => Promise<DesktopWorkspace | null>;
};

export function DirectoryInput({ value, onChange, workspaces, id, placeholder, onAddWorkspace }: DirectoryInputProps) {
  return (
    <WorkspacePathPicker
      id={id}
      onAddWorkspace={onAddWorkspace}
      onChange={onChange}
      placeholder={placeholder}
      value={value}
      workspaces={workspaces}
    />
  );
}
