import { I18N } from "../../lib/i18n";
import { Select } from "./Select";

/** Shared language-menu behavior for settings, subtitle controls and the tray.
 * Callers supply the complete route-resolved catalog; menu size never limits it. */
export function LanguageSelect(props: {
  label: string;
  value: string;
  valueLabel?: string;
  options: readonly { value: string; label: string }[];
  disabled?: boolean;
  busy?: boolean;
  onChange: (value: string) => void;
}) {
  return <Select {...props}
    searchLabel={props.options.length > 6 ? I18N.settings.searchLanguages : undefined}
    emptyMessage={I18N.settings.noMatchingLanguages} />;
}
