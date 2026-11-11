import {
  Eraser,
  Grid3x3,
  Palette,
  Pen,
  Pointer,
  Redo2,
  Trash2,
  Undo2,
} from 'lucide-react';
import { AlertDialog } from '@base-ui/react/alert-dialog';
import { Popover } from '@base-ui/react/popover';
import { cn } from 'cn';
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { Button, buttonVariants } from '@/components/ui/button';
import { Separator } from '@/components/ui/separator';
import { Slider } from '@/components/ui/slider';
import { Toggle } from '@/components/ui/toggle';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';
import { AlertDialogPopup } from './AlertDialogPopup';
import { PEN_COLORS } from './color';
import { ERASER_SIZE } from './erase';
import { PartialEraseIcon, StrokeEraseIcon } from './EraserModeIcons';
import type { EraserMode, History, Tool } from './types';

interface Choice<T extends string> {
  value: T;
  /** Button name (used by screen readers) */
  label: string;
  content: ReactNode;
}

/** Tool choices. The pen icon is drawn in the selected color */
function toolChoices(penColor: string): Choice<Tool>[] {
  return [
    {
      value: 'pen',
      label: 'Pen',
      // lucide icons use currentColor for lines, so color changes their color
      content: <Pen style={{ color: penColor }} />,
    },
    { value: 'eraser', label: 'Eraser', content: <Eraser /> },
  ];
}

const ERASER_MODES: Choice<EraserMode>[] = [
  { value: 'stroke', label: 'Object Eraser', content: <StrokeEraseIcon /> },
  { value: 'partial', label: 'Pixel Eraser', content: <PartialEraseIcon /> },
];

/** Step of the size slider (CSS pixels). Fine enough to move almost continuously */
const ERASER_SIZE_STEP = 0.1;

interface ChoiceGroupProps<T extends string> {
  label: string;
  choices: readonly Choice<T>[];
  value: T;
  onChange: (value: T) => void;
  /** Called when a button is pressed (including pressing the selected button again) */
  onPress?: (value: T) => void;
  /** Direction the buttons are laid out. Horizontal by default */
  orientation?: 'horizontal' | 'vertical';
}

/** A group of buttons with exactly one always selected */
function ChoiceGroup<T extends string>({
  label,
  choices,
  value,
  onChange,
  onPress,
  orientation,
}: ChoiceGroupProps<T>) {
  return (
    <ToggleGroup
      aria-label={label}
      size="lg"
      orientation={orientation}
      value={[value]}
      // Pressing the selected button again gives an empty array; keep the selection in that case
      onValueChange={([next]) => {
        const selected = choices.find((choice) => choice.value === next);
        if (selected) onChange(selected.value);
      }}
    >
      {choices.map((choice) => (
        <ToggleGroupItem
          key={choice.value}
          value={choice.value}
          // Marker used to find the button the popover is anchored to
          data-value={choice.value}
          aria-label={choice.label}
          onClick={() => onPress?.(choice.value)}
        >
          {choice.content}
        </ToggleGroupItem>
      ))}
    </ToggleGroup>
  );
}

interface EraserSettingsProps {
  eraserMode: EraserMode;
  onEraserModeChange: (mode: EraserMode) => void;
  /** Diameter of the tracing eraser (CSS pixels) */
  eraserSize: number;
  onEraserSizeChange: (size: number) => void;
}

/** Passed to popover contents. Called when the final setting (nothing left to choose) is chosen, closing the popover */
interface Closable {
  onDone: () => void;
}

/**
 * Contents of the eraser popover: erase mode and size in one row.
 * Size is used only when erasing by tracing, so it is disabled (not hidden) when erasing whole strokes.
 * Closes when "erase whole strokes" is chosen. When "erase by tracing" is chosen or the size changes,
 * it stays open so the size can be adjusted repeatedly (it closes on an outside press or pressing the eraser again)
 */
function EraserSettings({
  eraserMode,
  onEraserModeChange,
  eraserSize,
  onEraserSizeChange,
  onDone,
}: EraserSettingsProps & Closable) {
  const sizeDisabled = eraserMode !== 'partial';
  return (
    <div className="flex items-center gap-1">
      <ChoiceGroup
        label="Eraser Type"
        choices={ERASER_MODES}
        value={eraserMode}
        onChange={onEraserModeChange}
        onPress={(mode) => {
          if (mode === 'stroke') onDone();
        }}
      />
      <Separator orientation="vertical" className="mx-1 my-1.5" />
      <Slider
        thumbLabel="Eraser Size"
        className="w-28 px-2 data-disabled:opacity-50"
        disabled={sizeDisabled}
        min={ERASER_SIZE.min}
        max={ERASER_SIZE.max}
        step={ERASER_SIZE_STEP}
        value={eraserSize}
        onValueChange={onEraserSizeChange}
      />
      <span
        className={cn(
          'w-11 text-right text-sm text-muted-foreground tabular-nums',
          sizeDisabled && 'opacity-50',
        )}
      >
        {Math.round(eraserSize)}px
      </span>
    </div>
  );
}

interface PenSettingsProps {
  /** Pen color ("#rrggbb") */
  penColor: string;
  onPenColorChange: (color: string) => void;
  /** Color chosen in the color palette. null if none chosen yet */
  customColor: string | null;
  onCustomColorChange: (color: string) => void;
}

/** Color swatch (circle) */
function Swatch({ color }: { color: string }) {
  return (
    <span
      className="size-5 rounded-full border border-black/15"
      style={{ backgroundColor: color }}
    />
  );
}

/**
 * Contents of the pen popover: black, red, blue, green, and the color chosen in the color palette (custom).
 * Choosing a color in the palette keeps it as the custom color and makes it the pen color.
 * Closes when a swatch is pressed or a color is confirmed in the palette
 */
function PenSettings({
  penColor,
  onPenColorChange,
  customColor,
  onCustomColorChange,
  onDone,
}: PenSettingsProps & Closable) {
  // React's onChange fires even while the color is being moved in the palette (input event).
  // Closing then could close the palette too, so close when the color is confirmed (change event)
  const paletteRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const palette = paletteRef.current;
    palette?.addEventListener('change', onDone);
    return () => palette?.removeEventListener('change', onDone);
  }, [onDone]);

  const custom =
    customColor && !PEN_COLORS.some(({ value }) => value === customColor)
      ? [{ value: customColor, label: 'Custom Color' }]
      : [];
  const choices: Choice<string>[] = [...PEN_COLORS, ...custom].map(
    ({ value, label }) => ({ value, label, content: <Swatch color={value} /> }),
  );
  return (
    <div className="flex items-center gap-1">
      <ChoiceGroup
        label="Pen Color"
        choices={choices}
        value={penColor}
        onChange={onPenColorChange}
        onPress={onDone}
      />
      <Separator orientation="vertical" className="mx-1 my-1.5" />
      {/* Pressing the label makes the input (type=color) inside open the OS color palette */}
      <label className={buttonVariants({ variant: 'ghost', size: 'icon-lg' })}>
        <Palette />
        <span className="sr-only">Choose Custom Color</span>
        <input
          ref={paletteRef}
          type="color"
          className="sr-only"
          value={customColor ?? penColor}
          onChange={(event) => {
            onCustomColorChange(event.target.value);
            onPenColorChange(event.target.value);
          }}
        />
      </label>
    </div>
  );
}

/**
 * Button that clears the canvas. It cannot be undone, so it asks for confirmation when pressed.
 * It sits at the end (bottom) of the toolbar, away from frequently used buttons (to prevent mistaken presses).
 * The confirmation text and buttons follow Apple's Human Interface Guidelines (Alerts):
 * a title saying exactly what will happen, a short description, and buttons with a verb describing the result plus "Cancel".
 * It confirms an action the user chose, so the action button does not use a destructive (red) style
 */
function ClearButton({
  disabled,
  onClear,
}: {
  disabled: boolean;
  onClear: () => void;
}) {
  return (
    <AlertDialog.Root>
      <AlertDialog.Trigger
        render={
          <Button
            variant="ghost"
            size="icon-lg"
            aria-label="Clear"
            disabled={disabled}
          />
        }
      >
        <Trash2 />
      </AlertDialog.Trigger>
      <AlertDialogPopup
        title="Clear the canvas?"
        description="All strokes will be erased. You can't undo this action."
      >
        <AlertDialog.Close render={<Button variant="outline" />}>
          Cancel
        </AlertDialog.Close>
        <AlertDialog.Close render={<Button onClick={onClear} />}>
          Clear
        </AlertDialog.Close>
      </AlertDialogPopup>
    </AlertDialog.Root>
  );
}

type ToolbarProps = EraserSettingsProps &
  PenSettingsProps & {
    tool: Tool;
    onToolChange: (tool: Tool) => void;
    /** Whether one finger draws (erases) */
    fingerDrawing: boolean;
    onFingerDrawingChange: (enabled: boolean) => void;
    /** Whether to show the grid */
    showGrid: boolean;
    onShowGridChange: (show: boolean) => void;
    history: History;
    /** Zoom factor (1 is 100%). null when not zoomed or panned, in which case the zoom button is hidden */
    viewScale: number | null;
    /** Resets the view to 100% and the original position */
    onResetView: () => void;
    /** Whether there is at least one stroke (Clear is disabled otherwise) */
    canClear: boolean;
    /** Returns to a blank initial state */
    onClear: () => void;
  };

/** Line separating groups of vertically stacked buttons */
function GroupSeparator() {
  return <Separator className="my-1" />;
}

/**
 * Single-column vertical toolbar placed in the sidebar on the right of the screen.
 * A tool's settings popover appears right beside that tool's button, on its left (the canvas side)
 */
export function Toolbar({
  tool,
  onToolChange,
  fingerDrawing,
  onFingerDrawingChange,
  showGrid,
  onShowGridChange,
  history,
  viewScale,
  onResetView,
  canClear,
  onClear,
  penColor,
  onPenColorChange,
  customColor,
  onCustomColorChange,
  ...eraserSettings
}: ToolbarProps) {
  const [settingsOpen, setSettingsOpen] = useState(false);
  /** The pen and eraser group. Used to tell outside presses and to find the popover's anchor button */
  const toolsRef = useRef<HTMLSpanElement>(null);
  /** Popover anchor: the selected tool's button. The popover appears right beside it (vertically centered) */
  const settingsAnchor = useCallback(
    () => toolsRef.current?.querySelector(`[data-value="${tool}"]`) ?? null,
    [tool],
  );

  // Keep using the same function so the palette's change listener need not be re-attached when contents re-render
  const closeSettings = useCallback(() => setSettingsOpen(false), []);

  // Pressing a tool opens its settings popover. Pressing the selected tool toggles it
  const onToolPress = (pressed: Tool) => {
    setSettingsOpen((open) => (pressed === tool ? !open : true));
  };

  const onSettingsOpenChange = (
    open: boolean,
    { reason, event }: Popover.Root.ChangeEventDetails,
  ) => {
    // Presses on tool buttons are toggled by onToolPress, so do not treat them as outside presses
    const onTools =
      event.target instanceof Node && toolsRef.current?.contains(event.target);
    if (!open && reason === 'outside-press' && onTools) return;
    setSettingsOpen(open);
  };

  return (
    <div
      role="toolbar"
      aria-label="Drawing Tools"
      aria-orientation="vertical"
      className="flex flex-col items-center gap-1"
    >
      <span ref={toolsRef} className="inline-flex">
        <ChoiceGroup
          label="Tools"
          orientation="vertical"
          choices={toolChoices(penColor)}
          value={tool}
          onChange={onToolChange}
          onPress={onToolPress}
        />
      </span>

      <Popover.Root open={settingsOpen} onOpenChange={onSettingsOpenChange}>
        <Popover.Portal>
          <Popover.Positioner
            anchor={settingsAnchor}
            side="left"
            align="center"
            sideOffset={8}
            collisionPadding={8}
          >
            <Popover.Popup
              aria-label={tool === 'pen' ? 'Pen Settings' : 'Eraser Settings'}
              initialFocus={false}
              className="rounded-xl border bg-background p-1 shadow-md outline-none"
            >
              {tool === 'pen' ? (
                <PenSettings
                  penColor={penColor}
                  onPenColorChange={onPenColorChange}
                  customColor={customColor}
                  onCustomColorChange={onCustomColorChange}
                  onDone={closeSettings}
                />
              ) : (
                <EraserSettings {...eraserSettings} onDone={closeSettings} />
              )}
            </Popover.Popup>
          </Popover.Positioner>
        </Popover.Portal>
      </Popover.Root>

      {/* For devices without a pen. On at first, and turned off automatically when a pen is used (useFingerDrawing) */}
      <Toggle
        size="lg"
        aria-label="Draw with Finger"
        pressed={fingerDrawing}
        onPressedChange={(pressed) => onFingerDrawingChange(pressed)}
      >
        <Pointer />
      </Toggle>
      <Toggle
        size="lg"
        aria-label="Show Grid"
        pressed={showGrid}
        onPressedChange={(pressed) => onShowGridChange(pressed)}
      >
        <Grid3x3 />
      </Toggle>

      <GroupSeparator />

      <Button
        variant="ghost"
        size="icon-lg"
        aria-label="Undo"
        disabled={!history.canUndo}
        onClick={history.undo}
      >
        <Undo2 />
      </Button>
      <Button
        variant="ghost"
        size="icon-lg"
        aria-label="Redo"
        disabled={!history.canRedo}
        onClick={history.redo}
      >
        <Redo2 />
      </Button>

      {viewScale !== null && (
        <Button
          variant="ghost"
          size="lg"
          aria-label="Reset Zoom"
          // Narrow the horizontal padding so "800%" fits in the sidebar width
          className="px-1 tabular-nums"
          onClick={onResetView}
        >
          {Math.round(viewScale * 100)}%
        </Button>
      )}

      <GroupSeparator />
      <ClearButton disabled={!canClear} onClear={onClear} />
    </div>
  );
}
