import { useEffect, useState } from "react";
import { RotateCw, Volume1 } from "lucide-react";
import { useApplicationAudioPicker } from "../../lib/audio/useApplicationAudioPicker";
import { useSettingsToast } from "../../windows/settings/feedback/useSettingsToast";
import { Select } from "../inputs/Select";
import { Icon } from "../Icon";
import "./application-audio-picker.css";

function ApplicationIcon({ dataUrl }: { dataUrl?: string | null }) {
  const [failedUrl, setFailedUrl] = useState<string | null>(null);
  // Native enumeration supplies local PNGs. Never fetch an app-provided URL.
  if (!dataUrl?.startsWith("data:image/png;base64,") || dataUrl === failedUrl) return <span className="application-audio-picker__symbol" aria-hidden="true"><Icon name="app-window" /></span>;
  return <img src={dataUrl} alt="" draggable={false} onError={() => setFailedUrl(dataUrl)} />;
}

/** The same application choice is available in Settings and the floating panel. */
export function ApplicationAudioPicker({ disabled = false, busy = false, onBusyChange }: {
  disabled?: boolean;
  busy?: boolean;
  onBusyChange?: (busy: boolean) => void;
}) {
  const feedback = useSettingsToast();
  const picker = useApplicationAudioPicker(disabled, busy, feedback);
  useEffect(() => {
    onBusyChange?.(picker.pending);
    return () => onBusyChange?.(false);
  }, [onBusyChange, picker.pending]);
  const options = picker.options.map(option => ({
    ...option,
    // Volume1's artwork spans x=2..17: center the drawing, not its 24px canvas.
    icon: option.value === "" ? <span className="application-audio-picker__symbol" aria-hidden="true"><Volume1 viewBox="-2.5 0 24 24" /></span> : <ApplicationIcon dataUrl={option.iconDataUrl} />,
  }));
  const refreshUnavailable = picker.unavailable || !picker.supported;
  const refreshBusy = !refreshUnavailable && (picker.busy || picker.loading);
  return <span className="application-audio-picker" aria-busy={picker.busy || picker.loading}
    title={!picker.supported ? picker.text.unsupported : undefined}>
    <span className="application-audio-picker__controls">
      <Select label={picker.text.title} value={picker.selected} valueLabel={picker.valueLabel} valueIcon={<ApplicationIcon dataUrl={picker.selectedIconDataUrl} />}
        searchLabel={picker.text.search} emptyMessage={picker.text.noMatch} options={options}
        disabled={picker.unavailable || (!picker.supported && picker.selected === "")} busy={picker.busy}
        onOpen={() => { void picker.refresh("open"); }} onChange={id => { void picker.choose(id); }} />
      <button type="button" className="application-audio-picker__refresh" aria-label={picker.text.refresh}
        title={picker.loading ? picker.text.loading : picker.empty ? picker.text.empty : picker.text.refresh} disabled={refreshUnavailable || refreshBusy}
        aria-busy={refreshBusy || undefined} data-refreshing={picker.loading || undefined} onClick={() => { void picker.refresh("refresh"); }}>
        <RotateCw size={14} aria-hidden="true" />
      </button>
    </span>
    {picker.missing && <span className="application-audio-picker__feedback" data-tone="error" role="alert">{picker.text.unavailable}</span>}
  </span>;
}
