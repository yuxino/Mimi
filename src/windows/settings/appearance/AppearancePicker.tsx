import { Icon } from "../../../components/Icon";
import { I18N } from "../../../lib/i18n";
import type { SettingsTheme } from "./useSettingsTheme";

export function AppearancePicker({
  value,
  onChange,
}: {
  value: SettingsTheme;
  onChange: (theme: SettingsTheme) => void;
}) {
  const options: { value: SettingsTheme; label: string }[] = [
    { value: "light", label: I18N.settings.themeLight },
    { value: "dark", label: I18N.settings.themeDark },
    { value: "system", label: I18N.settings.themeSystem },
  ];
  return (
    <div
      className="appearance-picker"
      role="group"
      aria-label={I18N.settings.appearance}
    >
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          className={`appearance-option${value === option.value ? " is-selected" : ""}`}
          aria-pressed={value === option.value}
          onClick={() => onChange(option.value)}
        >
          <span
            className={`appearance-preview appearance-preview--${option.value}`}
            aria-hidden="true"
          >
            <span className="appearance-preview__sidebar">
              <i />
              <i />
              <i />
            </span>
            <span className="appearance-preview__content">
              <i />
              <i />
              <i />
            </span>
          </span>
          <span className="appearance-option__label">
            {option.label}
            <span className="appearance-option__check" aria-hidden="true">
              {value === option.value && <Icon name="checkmark" />}
            </span>
          </span>
        </button>
      ))}
    </div>
  );
}
