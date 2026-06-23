import {
  Button,
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from 'garyx-desktop';

export const Confirm = () => (
  <Dialog defaultOpen>
    <DialogContent>
      <DialogHeader>
        <DialogTitle>Remove bot?</DialogTitle>
        <DialogDescription>
          This disconnects “Telegram · main” and stops routing its threads. The
          conversation history is kept.
        </DialogDescription>
      </DialogHeader>
      <DialogFooter>
        <Button variant="outline">Cancel</Button>
        <Button variant="destructive">Remove bot</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
);

export const FormDialog = () => (
  <Dialog defaultOpen>
    <DialogContent>
      <DialogHeader>
        <DialogTitle>New workspace</DialogTitle>
        <DialogDescription>Point Garyx at a directory to use as a workspace.</DialogDescription>
      </DialogHeader>
      <DialogBody>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6, fontSize: 14 }}>
          <span style={{ color: '#555' }}>Path</span>
          <div
            style={{
              border: '1px solid #e1e1e1',
              borderRadius: 8,
              padding: '8px 10px',
              color: '#0d0d0d',
              fontFamily: 'SF Mono, Menlo, monospace',
            }}
          >
            /Users/test/repos/garyx
          </div>
        </div>
      </DialogBody>
      <DialogFooter>
        <Button variant="outline">Cancel</Button>
        <Button>Add workspace</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
);
