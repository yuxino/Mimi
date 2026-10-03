import { useState } from "react";
import { Switch } from "../../components/Switch";
import { I18N } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import { SettingsRow } from "./SettingsPrimitives";
import { useSettingsToast } from "./useSettingsToast";

/** The native command also rejects this preference on other platforms. */
export function DockPreference() {
  const show = useStore((state) => state.settings.showInDock);
  const save = useStore((state) => state.saveSettings);
  const [pending, setPending] = useState(false);
  const { runWithToast } = useSettingsToast();
  if (typeof navigator === "undefined" || !/Macintosh|MacIntel/i.test(navigator.userAgent + " " + navigator.platform)) {
    return null;
  }
  return (
    <>
      <SettingsRow label={I18N.settings.showInDock}>
        <Switch
          aria-label={I18N.settings.showInDock}
          checked={show}
          disabled={pending}
          onChange={(showInDock) => {
            setPending(true);
            void runWithToast(() => save({ showInDock }), I18N.settings.dockSaveFailed)
              .finally(() => setPending(false));
          }}
        />
      </SettingsRow>
      <div className="settings-divider" />
    </>
  );
}
