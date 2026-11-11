import { Slider as SliderPrimitive } from '@base-ui/react/slider';
import { cn } from 'cn';

interface SliderProps extends SliderPrimitive.Root.Props<number> {
  /** Name of the thumb (the input used by screen readers) */
  thumbLabel: string;
}

/** A slider that picks one value */
function Slider({ className, thumbLabel, ...props }: SliderProps) {
  return (
    <SliderPrimitive.Root
      data-slot="slider"
      className={cn('w-full', className)}
      {...props}
    >
      {/* touch-none: keep the page from scrolling when the thumb is dragged with a finger */}
      <SliderPrimitive.Control className="flex h-9 w-full touch-none items-center select-none">
        <SliderPrimitive.Track className="relative h-1.5 w-full rounded-full bg-muted">
          <SliderPrimitive.Indicator className="rounded-full bg-primary" />
          <SliderPrimitive.Thumb
            getAriaLabel={() => thumbLabel}
            className="size-4 rounded-full border border-primary bg-background shadow-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
          />
        </SliderPrimitive.Track>
      </SliderPrimitive.Control>
    </SliderPrimitive.Root>
  );
}

export { Slider };
