import { Icon, type IconName } from "../../components/Icon";
import type { RefObject } from "react";
import { Tooltip } from "../../components/Tooltip";
import "./control-button.css";

interface ControlButtonProps {
  icon: IconName;
  label: string;
  onClick: () => void;
  disabled?: boolean;
  busy?: boolean;
  tooltipArea?: RefObject<HTMLElement | null>;
  shortcutHint?: string;
  "data-testid"?: string;
}

/** 24×24 rounded control with a 10pt icon (matches `OverlayControlButton`). */
export function ControlButton({
  icon,
  label,
  onClick,
  disabled = false,
  busy = false,
  tooltipArea,
  shortcutHint,
  "data-testid": dataTestId,
}: ControlButtonProps) {
  return (
    <Tooltip label={label} shortcut={shortcutHint} popupArea={tooltipArea}>
      {(descriptionId, hovered) => (
        <button
          type="button"
          onClick={onClick}
          aria-label={label}
          aria-describedby={descriptionId}
          aria-busy={busy || undefined}
          disabled={disabled || busy}
          data-testid={dataTestId}
          data-hovered={hovered || undefined}
          className="overlay-control-button flex items-center justify-center"
        >
          {busy ? <span className="overlay-control-button__busy" aria-hidden="true" /> : <Icon name={icon} />}
        </button>
      )}
    </Tooltip>
  );
}
