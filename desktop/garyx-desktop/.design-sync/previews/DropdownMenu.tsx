import {
  Button,
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from 'garyx-desktop';
import { Pin, Archive, Copy, Trash2 } from 'lucide-react';

export const ThreadActions = () => (
  <DropdownMenu defaultOpen>
    <DropdownMenuTrigger asChild>
      <Button variant="outline">Actions</Button>
    </DropdownMenuTrigger>
    <DropdownMenuContent>
      <DropdownMenuLabel>Thread</DropdownMenuLabel>
      <DropdownMenuItem><Pin /> Pin thread</DropdownMenuItem>
      <DropdownMenuItem><Copy /> Duplicate</DropdownMenuItem>
      <DropdownMenuItem><Archive /> Archive</DropdownMenuItem>
      <DropdownMenuSeparator />
      <DropdownMenuItem variant="destructive"><Trash2 /> Delete</DropdownMenuItem>
    </DropdownMenuContent>
  </DropdownMenu>
);

export const WithCheckboxes = () => (
  <DropdownMenu defaultOpen>
    <DropdownMenuTrigger asChild>
      <Button variant="outline">Columns</Button>
    </DropdownMenuTrigger>
    <DropdownMenuContent>
      <DropdownMenuLabel>Show columns</DropdownMenuLabel>
      <DropdownMenuCheckboxItem checked>Agent</DropdownMenuCheckboxItem>
      <DropdownMenuCheckboxItem checked>Status</DropdownMenuCheckboxItem>
      <DropdownMenuCheckboxItem>Last active</DropdownMenuCheckboxItem>
    </DropdownMenuContent>
  </DropdownMenu>
);
