import { AlertDialog } from '@base-ui/react/alert-dialog';
import type { ReactNode } from 'react';
import { DIALOG_BACKDROP_CLASS, DIALOG_POPUP_CLASS } from '@/lib/dialogStyles';
import { cn } from '@/lib/utils';

interface AlertDialogPopupProps {
  title: string;
  description: string;
  /** Buttons placed at the bottom right (AlertDialog.Close, etc.) */
  children: ReactNode;
}

/** Contents of a confirmation or notice dialog. Shown centered on the screen with the background dimmed */
export function AlertDialogPopup({
  title,
  description,
  children,
}: AlertDialogPopupProps) {
  return (
    <AlertDialog.Portal>
      <AlertDialog.Backdrop className={DIALOG_BACKDROP_CLASS} />
      <AlertDialog.Popup
        className={cn(DIALOG_POPUP_CLASS, 'w-[min(90vw,22rem)]')}
      >
        <AlertDialog.Title className="text-base font-medium">
          {title}
        </AlertDialog.Title>
        <AlertDialog.Description className="mt-2 text-sm text-muted-foreground">
          {description}
        </AlertDialog.Description>
        <div className="mt-5 flex justify-end gap-2">{children}</div>
      </AlertDialog.Popup>
    </AlertDialog.Portal>
  );
}
