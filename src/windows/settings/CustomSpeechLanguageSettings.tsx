import { useEffect, useId, useRef, useState } from "react";
import { Icon } from "../../components/Icon";
import { I18N } from "../../lib/i18n";
import { SOURCE_LANGUAGE_CODES, SOURCE_LANGUAGE_DISPLAY_NAMES, type ServiceProfile, type SourceLanguage } from "../../lib/types";
import { SettingsHelp } from "./SettingsHelp";
import { InlineFeedback, SettingsRow, SettingsSelect } from "./SettingsPrimitives";

type Declaration = SourceLanguage[] | null;
const canonical = (value: Declaration): Declaration => value === null ? null : SOURCE_LANGUAGE_CODES.filter(code => value.includes(code));

/** A user declaration narrows selectors without probing an endpoint or changing its model. */
export function CustomSpeechLanguageSettings({ profile, disabled, onSave }: {
  profile: ServiceProfile;
  disabled: boolean;
  onSave: (languages: Declaration) => Promise<unknown>;
}) {
  const editorId = useId();
  const [editing, setEditing] = useState(false);
  const saved = canonical(profile.customSpeechSourceLanguages ?? null);
  const [draft, setDraft] = useState<{ value: Declaration } | null>(null);
  const [query, setQuery] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState(false);
  const pending = useRef(false);
  const mounted = useRef(false);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const value = draft ? draft.value : saved;
  const dirty = JSON.stringify(value) !== JSON.stringify(saved);
  const locked = disabled || saving;
  const search = query.trim().toLocaleLowerCase();
  const options = SOURCE_LANGUAGE_CODES.filter(code => !search || code.toLocaleLowerCase().includes(search) || SOURCE_LANGUAGE_DISPLAY_NAMES[code].toLocaleLowerCase().includes(search));
  const change = (next: Declaration) => { if (!locked) { setDraft({ value: canonical(next) }); setError(false); } };
  const save = async () => {
    if (locked || pending.current || !dirty) return;
    pending.current = true; setSaving(true); setError(false);
    try {
      const result = await onSave(value);
      if (result === false || result === null) throw new Error("language_declaration_not_saved");
      if (mounted.current) { setDraft(null); setEditing(false); setQuery(""); }
    } catch {
      if (mounted.current) setError(true);
    } finally {
      pending.current = false;
      if (mounted.current) setSaving(false);
    }
  };
  return <section className="profile-language-settings custom-speech-languages" aria-label={I18N.settings.customSpeechLanguagesTitle} aria-busy={saving}>
    <header className="profile-language-settings__heading"><h3>{I18N.settings.customSpeechLanguagesTitle}</h3><SettingsHelp label={I18N.settings.helpLabel} text={I18N.settings.customSpeechLanguagesHelp} /></header>
    {!editing && <SettingsRow label={I18N.settings.customSpeechLanguagesDeclaration}>
      <span className="custom-speech-languages__summary">{saved === null ? I18N.settings.customSpeechLanguagesUnknown : saved.length === 0 ? I18N.settings.customSpeechLanguagesDefaultOnly : I18N.settings.customSpeechLanguagesCount(saved.length)}</span>
      <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={locked} aria-expanded={false} aria-controls={editorId} onClick={() => setEditing(true)}>{I18N.settings.customSpeechLanguagesEdit}</button>
    </SettingsRow>}
    {editing && <div id={editorId} className="custom-speech-languages__expanded">
    <SettingsRow label={I18N.settings.customSpeechLanguagesDeclaration}>
      <SettingsSelect label={I18N.settings.customSpeechLanguagesDeclaration} value={value === null ? "unknown" : "declared"} disabled={locked}
        options={[{ value: "unknown", label: I18N.settings.customSpeechLanguagesUnknown }, { value: "declared", label: I18N.settings.customSpeechLanguagesManual }]}
        onChange={mode => { change(mode === "unknown" ? null : saved ?? []); setQuery(""); }} />
    </SettingsRow>
    {value !== null && <div className="custom-speech-languages__editor">
      <label className="settings-field"><span>{I18N.settings.searchLanguages}</span><input type="search" value={query} onChange={event => setQuery(event.target.value)} disabled={locked} autoComplete="off" spellCheck={false} /></label>
      <div className="custom-speech-languages__choices" role="group" aria-label={I18N.settings.customSpeechLanguagesManual}>
        {options.map(code => <button key={code} type="button" className={`profile-language-choice${value.includes(code) ? " is-selected" : ""}`} aria-pressed={value.includes(code)} disabled={locked}
          onClick={() => change(value.includes(code) ? value.filter(item => item !== code) : [...value, code])}>
          <span>{SOURCE_LANGUAGE_DISPLAY_NAMES[code]}</span><span className="custom-speech-languages__code">{code}</span>{value.includes(code) && <Icon name="checkmark" />}
        </button>)}
        {options.length === 0 && <span>{I18N.settings.noMatchingLanguages}</span>}
      </div>
      <span className="custom-speech-languages__summary">{value.length === 0 ? I18N.settings.customSpeechLanguagesDefaultOnly : I18N.settings.customSpeechLanguagesCount(value.length)}</span>
    </div>}
    {error && <InlineFeedback tone="error">{I18N.settings.customSpeechLanguagesSaveFailed}</InlineFeedback>}
    <div className="credential-form__actions">
      <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={locked} onClick={() => { setDraft(null); setQuery(""); setError(false); setEditing(false); }}>{I18N.settings.cancel}</button>
      <button type="button" className="settings-button settings-button--primary settings-button--compact" disabled={locked || !dirty} onClick={() => void save()}>{saving ? I18N.settings.customSpeechLanguagesSaving : I18N.settings.customSpeechLanguagesSave}</button>
    </div>
    </div>}
  </section>;
}
