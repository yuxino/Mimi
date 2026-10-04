import { useEffect, useRef, useState } from "react";
import { Select } from "../../components/Select";
import { installedFontFamilies } from "../../lib/ipc";
import { I18N } from "../../lib/i18n";
import { SettingsRow } from "./SettingsPrimitives";
import { useSettingsToast } from "./useSettingsToast";

/** Enumerate only when opened; reopening also picks up newly installed fonts. */
export function SubtitleFontControl({ value, onChange, onPreview }: {
  value: string;
  onChange: (family: string) => Promise<unknown>;
  onPreview?: (family: string | undefined) => void;
}) {
  const [families, setFamilies] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState(false);
  const [pendingFamily, setPendingFamily] = useState<string | null>(null);
  const mounted = useRef(false);
  const inFlight = useRef(false);
  const savingInFlight = useRef(false);
  const queuedFamily = useRef<string | null>(null);
  const preview = useRef(onPreview);
  const { beginToast } = useSettingsToast();
  const saveNotice = useRef<ReturnType<typeof beginToast> | null>(null);
  useEffect(() => { preview.current = onPreview; }, [onPreview]);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; preview.current?.(undefined); };
  }, []);

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

  async function persistChoices() {
    savingInFlight.current = true;
    setSaving(true);
    try {
      while (queuedFamily.current !== null) {
        const family = queuedFamily.current;
        queuedFamily.current = null;
        const notify = saveNotice.current;
        try {
          const result = await onChange(family);
          // The caller owns rollback. A successful first save also satisfies
          // a rapid return to that choice before persistence completes.
          if (result !== false && queuedFamily.current === family) queuedFamily.current = null;
        } catch {
          notify?.(I18N.settings.settingSaveFailed(I18N.settings.subtitleFont), true);
        }
      }
    } finally {
      savingInFlight.current = false;
      if (mounted.current) { setSaving(false); setPendingFamily(null); preview.current?.(undefined); }
    }
  }

  function choose(family: string) {
    saveNotice.current = beginToast();
    setPendingFamily(family);
    preview.current?.(family);
    queuedFamily.current = family;
    if (!savingInFlight.current) void persistChoices();
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
