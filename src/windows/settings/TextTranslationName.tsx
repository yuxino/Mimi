import { useRef, useState } from "react";
import { profileErrorMessage } from "../../lib/connectionDiagnostics";
import { I18N } from "../../lib/i18n";
import { defaultTextTranslationName } from "../../lib/textTranslationName";
import type { ServiceProfile, TextTranslationNameDraft } from "../../lib/types";
import { ConfigInput } from "./ConfigInput";
import { InlineFeedback } from "./SettingsPrimitives";

/** Display metadata is saved independently of endpoint, model and credential drafts. */
export function TextTranslationName({ profile, route, inputId, disabled, onSave }: {
  profile: ServiceProfile;
  route: TextTranslationNameDraft["route"];
  inputId: string;
  disabled: boolean;
  onSave: (route: TextTranslationNameDraft["route"], name: string) => Promise<unknown>;
}) {
  const saved = profile.textTranslationNames?.[route] ?? "";
  const [renderedSaved, setRenderedSaved] = useState(saved);
  const [baseline, setBaseline] = useState(saved);
  const [draft, setDraft] = useState(saved);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const inFlight = useRef(false);
  if (renderedSaved !== saved) {
    setRenderedSaved(saved);
    setBaseline(saved);
    setDraft(saved);
    setError(null);
  }
  const changed = draft.trim() !== baseline;
  const save = async () => {
    if (disabled || inFlight.current || !changed) return;
    inFlight.current = true;
    setSaving(true);
    setError(null);
    const name = draft.trim();
    try {
      if (await onSave(route, name)) {
        setBaseline(name);
        setDraft(name);
      }
    } catch (cause) {
      setError(profileErrorMessage(cause));
    } finally {
      inFlight.current = false;
      setSaving(false);
    }
  };
  return <div className="settings-field translation-name-field">
    <label htmlFor={inputId}>{I18N.settings.textTranslationName}</label>
    <span className="settings-field__inline">
      <ConfigInput expandable id={inputId} value={draft} maxLength={64} disabled={disabled || saving}
        placeholder={defaultTextTranslationName(route)} autoComplete="off"
        aria-invalid={error ? true : undefined} aria-describedby={error ? `${inputId}-error` : undefined}
        onValueChange={value => { setDraft(value); setError(null); }}
        onKeyDown={event => {
          if (event.key === "Enter" && !event.nativeEvent.isComposing) {
            event.preventDefault();
            event.stopPropagation();
            void save();
          }
        }} />
      {changed && <button type="button" className="settings-button settings-button--quiet settings-button--compact"
        disabled={disabled || saving} onClick={() => void save()}>{I18N.settings.saveName}</button>}
    </span>
    {error && <div id={`${inputId}-error`} className="translation-name-field__error"><InlineFeedback tone="error">{error}</InlineFeedback></div>}
  </div>;
}
