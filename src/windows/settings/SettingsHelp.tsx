import { useId } from "react";
import { Icon, type IconName } from "../../components/Icon";
import { Tooltip } from "../../components/Tooltip";
import "./settings-help.css";

interface SettingsHelpProps {
  text: string;
  label?: string;
  /** A persistent description that adjacent form controls may reference. */
  id?: string;
  icon?: IconName;
}

/** Compact help with a visible hover/focus tooltip and a persistent description. */
export function SettingsHelp({ text, label, id, icon = "help" }: SettingsHelpProps) {
  const generatedId = useId();
  const descriptionId = id ?? generatedId;

  return (
    <span className="settings-help-control">
      <Tooltip label={text} popupClassName="settings-help-tooltip">
        {() => (
          <button
            className="settings-help-control__button"
            type="button"
            aria-label={label ?? text}
            aria-describedby={descriptionId}
          >
            <Icon name={icon} />
          </button>
        )}
      </Tooltip>
      <span id={descriptionId} className="settings-help-control__description">{text}</span>
    </span>
  );
}
