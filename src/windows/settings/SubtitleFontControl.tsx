import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { Select } from "../../components/Select";
import { installedFontFamilies } from "../../lib/ipc";
import { I18N } from "../../lib/i18n";
import { SettingsRow } from "./SettingsPrimitives";
import { useSettingsToast } from "./useSettingsToast";
import { subtitleFontChoiceOwner } from "./subtitleFontChoiceOwner";

/** Enumerate only when opened; reopening also picks up newly installed fonts. */
export function SubtitleFontControl({ value, onChange, onPreview, getCurrentValue }: {
  value: string;
  onChange: (family: string) => Promise<unknown>;
  onPreview?: (family: string | undefined) => void;
  /** Read the live source even when this view has been replaced or unmounted. */
  getCurrentValue: () => string;
}) {
  const [families, setFamilies] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const pendingFamily = useSyncExternalStore(subtitleFontChoiceOwner.subscribe, subtitleFontChoiceOwner.getSnapshot);
  const saving = pendingFamily !== null;
  const mounted = useRef(false);
  const inFlight = useRef(false);
  const preview = useRef(onPreview);
  const previewedFamily = useRef<string | null>(null);
  const { beginToast } = useSettingsToast();
  useEffect(() => { preview.current = onPreview; }, [onPreview]);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; previewedFamily.current = null; preview.current?.(undefined); };
  }, []);
  useEffect(() => {
    if (previewedFamily.current !== pendingFamily) {
      previewedFamily.current = pendingFamily;
      preview.current?.(pendingFamily ?? undefined);
    }
  }, [pendingFamily]);

  async function refresh() {
    if (inFlight.current) return;
    inFlight.current = true;
    setLoading(true);
    const notify = beginToast();
    try {
      const next = await installedFontFamilies();
      if (mounted.current) { setFamilies(next); setLoaded(true); }
    } catch {
      notify(I18N.settings.subtitleFontsLoadFailed, true);
    } finally {
      inFlight.current = false;
      if (mounted.current) setLoading(false);
    }
  }

  function choose(family: string) {
    const notify = beginToast();
    previewedFamily.current = family;
    preview.current?.(family);
    subtitleFontChoiceOwner.choose(family, onChange,
      () => notify(I18N.settings.settingSaveFailed(I18N.settings.subtitleFont), true),
      () => getCurrentValue() === family);
  }

  return <SettingsRow label={I18N.settings.subtitleFont} description={I18N.settings.subtitleFontHelp}>
    <span className="settings-select-wrap" aria-busy={loading || saving}>
      <Select label={I18N.settings.subtitleFont} value={pendingFamily ?? value} valueLabel={pendingFamily ?? value}
        options={[
          { value: "", label: I18N.settings.subtitleFontDefault },
          ...families.map(family => ({ value: family, label: family })),
        ]}
        searchLabel={I18N.settings.subtitleFontSearch}
        emptyMessage={I18N.settings.subtitleFontsNoMatch}
        loadingMessage={loading ? I18N.settings.subtitleFontsLoading : undefined}
        closedArrowSelection={loaded}
        onOpen={() => { void refresh(); }} onChange={choose} />
    </span>
  </SettingsRow>;
}
