import { useCallback, useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent } from "react";
import { I18N } from "../../../lib/i18n";

interface OverlayOpacitySliderProps {
  /** Background opacity in percent (0 = transparent, 100 = solid). */
  opacity: number;
  disabled?: boolean;
  onChange: (opacity: number) => void;
}

function clampOpacity(value: number): number {
  return Math.min(100, Math.max(0, Math.round(value)));
}

/** Compact left-edge vertical background-opacity slider revealed on overlay hover. */
export function OverlayOpacitySlider({ opacity, disabled = false, onChange }: OverlayOpacitySliderProps) {
  const controlRef = useRef<HTMLDivElement>(null);
  const visualTrackRef = useRef<HTMLSpanElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const draggingPointerRef = useRef<number | null>(null);
  const [dragging, setDragging] = useState(false);
  const clamped = clampOpacity(opacity);

  const opacityFromClientY = useCallback((clientY: number) => {
    const visualRect = visualTrackRef.current?.getBoundingClientRect();
    const rect = visualRect && visualRect.height > 0 ? visualRect : controlRef.current?.getBoundingClientRect();
    if (!rect || rect.height <= 0) return clamped;
    const ratio = 1 - (clientY - rect.top) / rect.height;
    return clampOpacity(ratio * 100);
  }, [clamped]);

  const finishDrag = useCallback((pointerId?: number) => {
    if (draggingPointerRef.current === null) return;
    if (pointerId !== undefined && draggingPointerRef.current !== pointerId) return;
    const activeId = draggingPointerRef.current;
    draggingPointerRef.current = null;
    setDragging(false);
    try {
      controlRef.current?.releasePointerCapture(activeId);
    } catch {
      // Capture may already have been released or unavailable in tests.
    }
  }, []);

  useEffect(() => {
    if (disabled) finishDrag();
  }, [disabled, finishDrag]);

  useEffect(() => {
    const release = (event: PointerEvent) => finishDrag(event.pointerId);
    const blur = () => finishDrag();
    window.addEventListener("pointerup", release);
    window.addEventListener("pointercancel", release);
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("pointerup", release);
      window.removeEventListener("pointercancel", release);
      window.removeEventListener("blur", blur);
      finishDrag();
    };
  }, [finishDrag]);

  const handlePointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (disabled || event.button !== 0 || event.isPrimary === false) return;
    event.preventDefault();
    event.stopPropagation();
    draggingPointerRef.current = event.pointerId;
    setDragging(true);
    try {
      event.currentTarget.setPointerCapture(event.pointerId);
    } catch {
      // Window listeners finish the gesture when pointer capture is unavailable.
    }
    const next = opacityFromClientY(event.clientY);
    if (next !== clamped) onChange(next);
  };

  const handlePointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (disabled || draggingPointerRef.current !== event.pointerId) return;
    if (event.pointerType === "mouse" && (event.buttons & 1) === 0) {
      finishDrag(event.pointerId);
      return;
    }
    event.preventDefault();
    const next = opacityFromClientY(event.clientY);
    if (next !== clamped) onChange(next);
  };

  const handleKeyDown = (event: ReactKeyboardEvent<HTMLInputElement>) => {
    if (disabled) return;
    let next: number;
    switch (event.key) {
      case "ArrowUp":
      case "ArrowRight":
        next = clampOpacity(clamped + 5);
        break;
      case "ArrowDown":
      case "ArrowLeft":
        next = clampOpacity(clamped - 5);
        break;
      case "PageUp":
        next = clampOpacity(clamped + 10);
        break;
      case "PageDown":
        next = clampOpacity(clamped - 10);
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = 100;
        break;
      default:
        return;
    }
    event.preventDefault();
    event.stopPropagation();
    if (next !== clamped) onChange(next);
  };

  useEffect(() => {
    const control = controlRef.current;
    if (!control) return;
    const handleWheel = (event: WheelEvent) => {
      if (disabled || event.deltaY === 0) return;
      event.preventDefault();
      event.stopPropagation();
      const step = event.deltaY < 0 ? 5 : -5;
      const next = clampOpacity(clamped + step);
      if (next !== clamped) onChange(next);
    };
    control.addEventListener("wheel", handleWheel, { passive: false });
    return () => control.removeEventListener("wheel", handleWheel);
  }, [clamped, disabled, onChange]);

  const label = I18N.overlay.backgroundOpacity;
  const valueText = `${clamped}%`;

  return (
    <div
      className="overlay-hover-control overlay-opacity-slider"
      data-dragging={dragging ? "" : undefined}
      data-testid="overlay-opacity-slider"
    >
      <div
        ref={controlRef}
        className="overlay-opacity-slider__control"
        title={`${label}: ${valueText}`}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={(event) => finishDrag(event.pointerId)}
        onPointerCancel={(event) => finishDrag(event.pointerId)}
        onLostPointerCapture={(event) => finishDrag(event.pointerId)}
      >
        <input
          ref={inputRef}
          type="range"
          className="overlay-opacity-slider__input"
          min={0}
          max={100}
          step={1}
          value={clamped}
          disabled={disabled}
          aria-label={label}
          aria-orientation="vertical"
          aria-valuetext={valueText}
          onChange={(event) => {
            const next = clampOpacity(Number(event.currentTarget.value));
            if (next !== clamped) onChange(next);
          }}
          onKeyDown={handleKeyDown}
        />
        <span ref={visualTrackRef} className="overlay-opacity-slider__track" aria-hidden="true">
          <span className="overlay-opacity-slider__fill" style={{ height: `${clamped}%` }} />
          <span className="overlay-opacity-slider__thumb" style={{ bottom: `${clamped}%` }} />
        </span>
      </div>
      <span className="overlay-opacity-slider__readout" aria-hidden="true">
        {valueText}
      </span>
    </div>
  );
}
