import {
  Button,
  DropdownMenu,
  DropdownMenuTrigger,
  FloatingActionMenuContent,
  FloatingActionMenuItem,
} from 'garyx-desktop';
import { Pin, PencilLine, Trash2 } from 'lucide-react';

export const Menu = () => (
  <DropdownMenu defaultOpen>
    <DropdownMenuTrigger asChild>
      <Button size="icon" variant="ghost">⋯</Button>
    </DropdownMenuTrigger>
    <FloatingActionMenuContent>
      <FloatingActionMenuItem><Pin /> Pin thread</FloatingActionMenuItem>
      <FloatingActionMenuItem><PencilLine /> Rename</FloatingActionMenuItem>
      <FloatingActionMenuItem variant="destructive"><Trash2 /> Delete</FloatingActionMenuItem>
    </FloatingActionMenuContent>
  </DropdownMenu>
);
