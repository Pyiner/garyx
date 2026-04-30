import * as React from "react";

import {
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";

function FloatingActionMenuContent({
  className,
  sideOffset = 6,
  ...props
}: React.ComponentProps<typeof DropdownMenuContent>) {
  return (
    <DropdownMenuContent
      className={cn("floating-action-menu", className)}
      sideOffset={sideOffset}
      {...props}
    />
  );
}

function FloatingActionMenuSubContent({
  className,
  sideOffset = 6,
  ...props
}: React.ComponentProps<typeof DropdownMenuSubContent>) {
  return (
    <DropdownMenuSubContent
      className={cn("floating-action-menu", className)}
      sideOffset={sideOffset}
      {...props}
    />
  );
}

function FloatingActionMenuItem({
  className,
  ...props
}: React.ComponentProps<typeof DropdownMenuItem>) {
  return (
    <DropdownMenuItem
      className={cn("floating-action-menu-row", className)}
      {...props}
    />
  );
}

function FloatingActionMenuSubTrigger({
  className,
  ...props
}: React.ComponentProps<typeof DropdownMenuSubTrigger>) {
  return (
    <DropdownMenuSubTrigger
      className={cn(
        "floating-action-menu-row floating-action-menu-subtrigger",
        className,
      )}
      {...props}
    />
  );
}

export {
  FloatingActionMenuContent,
  FloatingActionMenuItem,
  FloatingActionMenuSubContent,
  FloatingActionMenuSubTrigger,
};
