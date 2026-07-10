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
  ...props
}: React.ComponentProps<typeof DropdownMenuContent>) {
  return (
    <DropdownMenuContent
      className={cn("floating-action-menu", className)}
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
  variant,
  ...props
}: React.ComponentProps<typeof DropdownMenuItem> & {
  variant?: "default" | "destructive";
}) {
  return (
    <DropdownMenuItem
      className={cn(
        "floating-action-menu-row",
        variant === "destructive" && "floating-action-menu-row--destructive",
        className,
      )}
      variant={variant}
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
