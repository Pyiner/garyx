import type { DesktopWorkspace } from '@shared/contracts';

import { WorkspacePathPicker } from './WorkspacePathPicker';

type DirectoryInputProps = {
  value: string;
  onChange: (next: string) => void;
  workspaces?: DesktopWorkspace[];
  id?: string;
  placeholder?: string;
};

export function DirectoryInput({ value, onChange, workspaces, id, placeholder }: DirectoryInputProps) {
  return (
    <WorkspacePathPicker
      id={id}
      onChange={onChange}
      placeholder={placeholder}
      value={value}
      workspaces={workspaces}
    />
  );
}
