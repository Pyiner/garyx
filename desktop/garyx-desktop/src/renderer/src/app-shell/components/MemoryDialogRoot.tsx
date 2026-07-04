// Memory dialog feature root (endgame architecture batch 5a, "Local state
// colocation list": MemoryDialogRoot owns dialog target/draft/dirty/
// loading/saving/error plus the overlay pause effect).
//
// The dialog state lives entirely here; the shell keeps a ref and calls
// `open(target)` — opening, editing, and saving re-render only this root.

import { Suspense, forwardRef, lazy, useImperativeHandle } from "react";

import {
  useMemoryDialogController,
  type MemoryDialogTarget,
} from "../useMemoryDialogController";

const MemoryDialog = lazy(() =>
  import("../../components/MemoryDialog").then((module) => ({
    default: module.MemoryDialog,
  })),
);

export interface MemoryDialogHandle {
  open(target: MemoryDialogTarget): void;
}

export const MemoryDialogRoot = forwardRef<MemoryDialogHandle>(
  function MemoryDialogRoot(_props, ref) {
    const {
      closeMemoryDialog,
      memoryDialogDirty,
      memoryDialogDocument,
      memoryDialogDraft,
      memoryDialogError,
      memoryDialogLoading,
      memoryDialogSaving,
      memoryDialogStatus,
      memoryDialogTarget,
      openMemoryDialog,
      saveMemoryDialog,
      setMemoryDialogDraft,
    } = useMemoryDialogController();

    useImperativeHandle(
      ref,
      () => ({
        open: (target: MemoryDialogTarget) => {
          void openMemoryDialog(target);
        },
      }),
      [openMemoryDialog],
    );

    if (!memoryDialogTarget) {
      return null;
    }

    return (
      <Suspense fallback={null}>
        <MemoryDialog
          dirty={memoryDialogDirty}
          draftContent={memoryDialogDraft}
          error={memoryDialogError}
          exists={memoryDialogDocument?.exists ?? false}
          loading={memoryDialogLoading}
          modifiedAt={memoryDialogDocument?.modifiedAt ?? null}
          onClose={closeMemoryDialog}
          onDraftChange={setMemoryDialogDraft}
          onSave={() => {
            void saveMemoryDialog();
          }}
          open={Boolean(memoryDialogTarget)}
          path={memoryDialogDocument?.path || null}
          saving={memoryDialogSaving}
          scope={memoryDialogTarget?.scope || "agent"}
          status={memoryDialogStatus}
          title={memoryDialogTarget?.title || "memory.md"}
        />
      </Suspense>
    );
  },
);
