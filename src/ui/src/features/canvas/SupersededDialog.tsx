import { AlertDialog } from '@base-ui/react/alert-dialog';
import { Button } from '@/components/ui/button';
import { AlertDialogPopup } from './AlertDialogPopup';

interface SupersededDialogProps {
  open: boolean;
  onClose: () => void;
}

/**
 * Tells the user that another page took over this page's connection (reconnected to the PC with the resume key given to this page).
 * Unless the user reconnected themselves, the key may have been stolen, so a dialog makes it noticeable
 */
export function SupersededDialog({ open, onClose }: SupersededDialogProps) {
  return (
    <AlertDialog.Root
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      <AlertDialogPopup
        title="Disconnected"
        description="Another canvas page reconnected to the PC in place of this one. If you didn't do this, someone else may be using your connection."
      >
        <AlertDialog.Close render={<Button />}>OK</AlertDialog.Close>
      </AlertDialogPopup>
    </AlertDialog.Root>
  );
}
