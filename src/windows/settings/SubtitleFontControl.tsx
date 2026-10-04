import { useEffect, useRef, useState } from "react";
import { Select } from "../../components/Select";
import { installedFontFamilies } from "../../lib/ipc";
import { I18N } from "../../lib/i18n";
import { SettingsRow } from "./SettingsPrimitives";
import { useSettingsToast } from "./useSettingsToast";

/** Enumerate only when opened; reopening also picks up newly installed fonts. */
export function SubtitleFontControl({ value, onChange }: {
  value: string;
  onChange: (family: string) => Promise<unknown>;
}) {
  const [families, setFamilies] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const mounted = useRef(false);
  const inFlight = useRef(false);
  const { beginToast } = useSettingsToast();
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  async function refresh() {
    if (inFlight.current) return;
    inFlight.current = true;
    setLoading(true);
    const notify = beginToast();
    try {
      const next = await installedFontFamilies();
      if (mounted.current) setFamilies(next);
    } catch {
      notify(I18N.settings.subtitleFontsLoadFailed, true);
    } finally {
      inFlight.current = false;
      if (mounted.current) setLoading(false);
    }
  }

  async function choose(family: string) {
    setSaving(true);
    try { await onChange(family); }
    finally { if (mounted.current) setSaving(false); }
  }

  return <SettingsRow label={I18N.settings.subtitleFont} description={I18N.settings.subtitleFontHelp}>
    <span className="settings-select-wrap" aria-busy={loading || saving}>
      <Select label={I18N.settings.subtitleFont} value={value} valueLabel={value}
        options={[
          { value: "", label: I18N.settings.subtitleFontDefault },
          ...families.map(family => ({ value: family, label: family })),
        ]}
        searchLabel={I18N.settings.subtitleFontSearch}
        emptyMessage={I18N.settings.subtitleFontsNoMatch}
        loadingMessage={loading ? I18N.settings.subtitleFontsLoading : undefined}
        disabled={saving}
        onOpen={() => { void refresh(); }} onChange={family => { void choose(family); }} />
    </span>
  </SettingsRow>;
}
