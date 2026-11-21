import { Popover } from '@base-ui/react/popover';
import { cn } from '@/lib/utils';
import { Trash2 } from 'lucide-react';
import { useState, type RefObject } from 'react';
import { Button } from '@/components/ui/button';
import { Separator } from '@/components/ui/separator';
import { Slider } from '@/components/ui/slider';

interface ImageSettingsProps {
  /** Ratio (%) to the original image size. null until decoding finishes (the slider is disabled) */
  imageScale: number | null;
  /** While the slider is moving (changes only the display and drawing) */
  onImageScaleChange: (percent: number) => void;
  /** When the slider is released (committed as an undoable operation) */
  onImageScaleCommit: (percent: number) => void;
  /** Deletes the image */
  onImageDelete: () => void;
}

/** Ratio (%) to the original size allowed by the image size slider */
const IMAGE_SCALE_PERCENT = { min: 5, max: 400 };

/**
 * Popover contents: the size slider (ratio to the original image) and delete.
 * It does not close while the slider is moving, so the size can be adjusted any number of times
 */
function ImageSettings({
  imageScale,
  onImageScaleChange,
  onImageScaleCommit,
  onImageDelete,
}: ImageSettingsProps) {
  // While moving, the display follows the dragged value instead of the committed one (the parent commits outside the slider)
  const [draft, setDraft] = useState<number | null>(null);
  const shown = draft ?? imageScale ?? 100;
  return (
    <div className="flex items-center gap-1">
      <Slider
        thumbLabel="Image Size"
        className="w-28 px-2 data-disabled:opacity-25"
        disabled={imageScale === null}
        min={IMAGE_SCALE_PERCENT.min}
        max={IMAGE_SCALE_PERCENT.max}
        step={1}
        value={shown}
        onValueChange={(value) => {
          setDraft(value);
          onImageScaleChange(value);
        }}
        onValueCommitted={(value) => {
          setDraft(null);
          onImageScaleCommit(value);
        }}
      />
      <span
        className={cn(
          'w-11 text-right text-sm text-muted-foreground tabular-nums',
          imageScale === null && 'opacity-25',
        )}
      >
        {Math.round(shown)}%
      </span>
      <Separator orientation="vertical" className="mx-1 my-1.5" />
      <Button
        variant="ghost"
        size="icon-lg"
        aria-label="Delete Image"
        onClick={onImageDelete}
      >
        <Trash2 />
      </Button>
    </div>
  );
}

type ImageSettingsPopoverProps = ImageSettingsProps & {
  open: boolean;
  /** Closes (on delete, on a press outside the popover and canvas, or Esc) */
  onClose: () => void;
  /** Anchor of the popover: the "Move Image" button or the clicked point (VirtualElement) */
  anchor: Popover.Positioner.Props['anchor'];
  /** Which side of the anchor to show it on (left for the button, below for a clicked point; the opposite side if it does not fit) */
  side: 'bottom' | 'left';
  /**
   * Pressing these doesn't close it. The parent decides what a press does
   * (canvas: reopens on the image, closes outside it or when dragging starts. Move Image: toggles it)
   */
  ignoreOutsidePressOn: readonly RefObject<HTMLElement | null>[];
};

/**
 * The popover for the image size and delete, with the move tool (Move Image).
 * Shown beside the button when Move Image is selected, or near the clicked point when the image is clicked
 * (so it stays on screen even when the image fills the sheet)
 */
export function ImageSettingsPopover({
  open,
  onClose,
  anchor,
  side,
  ignoreOutsidePressOn,
  onImageDelete,
  ...settings
}: ImageSettingsPopoverProps) {
  const onOpenChange = (
    next: boolean,
    { reason, event }: Popover.Root.ChangeEventDetails,
  ) => {
    if (next) {
      return;
    }
    const target = event.target;
    if (
      reason === 'outside-press' &&
      target instanceof Node &&
      ignoreOutsidePressOn.some((ref) => ref.current?.contains(target))
    ) {
      return;
    }
    onClose();
  };
  return (
    <Popover.Root open={open} onOpenChange={onOpenChange}>
      <Popover.Portal>
        <Popover.Positioner
          anchor={anchor}
          side={side}
          align="center"
          sideOffset={8}
          collisionPadding={8}
        >
          <Popover.Popup
            aria-label="Image Settings"
            initialFocus={false}
            className="rounded-xl border bg-background p-1 shadow-md outline-none"
          >
            <ImageSettings
              {...settings}
              onImageDelete={() => {
                onImageDelete();
                onClose();
              }}
            />
          </Popover.Popup>
        </Popover.Positioner>
      </Popover.Portal>
    </Popover.Root>
  );
}
