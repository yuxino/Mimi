import { useContext, useEffect, useState, useSyncExternalStore } from "react";
import { I18N } from "../../lib/i18n";
import { ConfigInput } from "./ConfigInput";
import { InlineFeedback } from "./SettingsPrimitives";
import { NameDraft, NameDraftContext, type NameDraftKey } from "./nameDraftOwner";

/** Keep a native editing draft while committing only trimmed display metadata. */
export function AutoSaveNameField({ id, label, value, placeholder, draftKey, disabled = false, readOnly = false, allowEmpty = true, className = "", onSave }: {
  id: string;
  label: string;
  value: string;
  placeholder: string;
  draftKey?: NameDraftKey;
  disabled?: boolean;
  readOnly?: boolean;
  allowEmpty?: boolean;
  className?: string;
  onSave: (name: string) => Promise<unknown>;
}) {
  const scope = useContext(NameDraftContext);
  const [local] = useState(() => new NameDraft(value));
  const current = scope && draftKey ? scope.owner.get(scope.profileId, draftKey, value) : local;
  const { draft, error } = useSyncExternalStore(current.subscribe, current.getSnapshot);

  useEffect(() => {
    current.views += 1;
    return () => current.detach();
  }, [current]);
  useEffect(() => {
    current.options = { readOnly, allowEmpty, onSave };
  });
  useEffect(() => {
    current.receive(value);
  }, [current, value]);
  useEffect(() => {
    if (!disabled && !readOnly && current.draft.trim() !== current.confirmed) {
      current.schedule();
    }
  }, [current, disabled, readOnly]);

  return <div className={`settings-field auto-save-name-field ${className}`}>
    <label htmlFor={id}>{label}</label>
    <span className="settings-field__inline">
      <ConfigInput id={id} value={draft} maxLength={64} disabled={disabled} readOnly={readOnly}
        placeholder={placeholder} autoComplete="off"
        aria-invalid={error ? true : undefined} aria-describedby={error ? `${id}-error` : undefined}
        onValueChange={next => current.change(next)}
        onFocus={() => { current.focused = true; }}
        onGroupBlur={event => {
          if (!event.currentTarget.contains(event.relatedTarget)) {
            current.focused = false;
            void current.flush();
          }
        }}
        onCompositionStart={() => { current.composing = true; current.clearTimer(); }}
        onCompositionEnd={event => { current.composing = false; current.change(event.currentTarget.value); }}
        onKeyDown={event => {
          if (event.key !== "Enter" || event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229 || current.composing) return;
          event.preventDefault();
          event.stopPropagation();
          current.confirm();
        }} />
    </span>
    {error && <div id={`${id}-error`} className="auto-save-name-field__error"><InlineFeedback tone="error">{error.message}{error.retry && <button type="button" className="settings-link" disabled={disabled || readOnly} onClick={() => current.retry()}>{I18N.settings.retryUpdate}</button>}</InlineFeedback></div>}
  </div>;
}
