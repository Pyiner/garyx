// Add-bot dialog feature root (endgame architecture batch 5b, "Local
// state colocation list": bot management owns the add-bot dialog state).
//
// Owns the open flag; the shell keeps an imperative handle (the left-rail
// Add Bot action opens it and kicks the agent-target refresh, which stays
// with the shell because it writes catalog state). The legacy
// addBotInitialValues state was write-only-null dead state — no caller
// ever opened the dialog with prefilled values — so it is dropped and the
// dialog's optional initialValues prop is simply not passed.

import {
  forwardRef,
  useImperativeHandle,
  useState,
  type ComponentProps,
} from "react";

import { AddBotDialog } from "./AddBotDialog";

export interface AddBotDialogHandle {
  open(): void;
}

type AddBotDialogRootProps = Omit<
  ComponentProps<typeof AddBotDialog>,
  "open" | "onClose" | "initialValues"
>;

export const AddBotDialogRoot = forwardRef<
  AddBotDialogHandle,
  AddBotDialogRootProps
>(function AddBotDialogRoot(props, ref) {
  const [open, setOpen] = useState(false);

  useImperativeHandle(
    ref,
    () => ({
      open: () => {
        setOpen(true);
      },
    }),
    [],
  );

  return (
    <AddBotDialog
      {...props}
      onClose={() => {
        setOpen(false);
      }}
      open={open}
    />
  );
});
