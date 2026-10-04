import { useEffect, useRef, useState } from "react";
import { profileErrorMessage } from "../../lib/connectionDiagnostics";
import { I18N } from "../../lib/i18n";
import { ConfigInput } from "./ConfigInput";
import { InlineFeedback } from "./SettingsPrimitives";

/** Keep a native editing draft while committing only trimmed display metadata. */
export function AutoSaveNameField({ id, label, value, placeholder, disabled = false, readOnly = false, allowEmpty = true, className = "", onSave }: {
  id: string;
  label: string;
  value: string;
  placeholder: string;
  disabled?: boolean;
  readOnly?: boolean;
  allowEmpty?: boolean;
  className?: string;
  onSave: (name: string) => Promise<unknown>;
}) {
  const [draft, setDraft] = useState(value);
  const [error, setError] = useState<{ message: string; retry: boolean } | null>(null);
  const current = useRef({ draft: value, confirmed: value, source: value, revision: 0, failedRevision: -1, composing: false, focused: false, inFlight: false, flushRequested: false, ownValues: new Set<string>() });
  const config = useRef({ disabled, readOnly, allowEmpty, onSave });
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const mounted = useRef(false);
  const flushRef = useRef<() => void>(() => {});

  const clearTimer = () => {
    if (timer.current !== null) clearTimeout(timer.current);
    timer.current = null;
  };
  const flush = async () => {
    clearTimer();
    const state = current.current;
    const options = config.current;
    // Disabling the UI blocks new edits; it must not discard a draft already
    // owned by this field. The store/native guard validates its final write.
    if (options.readOnly || state.composing) return;
    if (state.inFlight) { state.flushRequested = true; return; }
    const name = state.draft.trim();
    if (!options.allowEmpty && !name) {
      if (mounted.current) setError({ message: I18N.settings.nameRequired, retry: false });
      return;
    }
    if (name === state.confirmed || state.failedRevision === state.revision) return;
    const revision = state.revision;
    state.inFlight = true;
    state.flushRequested = false;
    state.ownValues.add(name);
    if (state.ownValues.size > 32) state.ownValues.delete(state.ownValues.values().next().value!);
    try {
      const result = await options.onSave(name);
      if (result === null || result === false) throw new Error("name_save_rejected");
      state.confirmed = name;
      if (mounted.current && revision === state.revision) setError(null);
    } catch (cause) {
      state.ownValues.delete(name);
      state.failedRevision = revision;
      if (mounted.current && revision === state.revision) setError({ message: profileErrorMessage(cause), retry: true });
    } finally {
      state.inFlight = false;
      if (state.flushRequested) {
        state.flushRequested = false;
        flushRef.current();
      }
    }
  };
  const schedule = () => {
    clearTimer();
    if (!current.current.composing) timer.current = setTimeout(() => { void flush(); }, 350);
  };
  const change = (next: string) => {
    const singleLine = next.replace(/[\r\n]/g, "");
    current.current.draft = singleLine;
    current.current.revision += 1;
    setDraft(singleLine);
    setError(null);
    schedule();
  };

  useEffect(() => {
    config.current = { disabled, readOnly, allowEmpty, onSave };
    flushRef.current = () => { void flush(); };
  });
  useEffect(() => {
    const state = current.current;
    if (state.source === value) return;
    state.source = value;
    const ownAcknowledgement = state.ownValues.delete(value);
    if (!state.inFlight && state.draft.trim() === value) {
      state.confirmed = value;
      setError(null);
      return;
    }
    // While editing, a late acknowledgement must not replace newer text or
    // move the caret. Once idle, accept external changes, including old names.
    if ((ownAcknowledgement && state.focused) || state.inFlight || state.draft.trim() !== state.confirmed) return;
    state.confirmed = value;
    if (state.draft.trim() !== value) {
      state.draft = value;
      setDraft(value);
      setError(null);
    }
  }, [value]);
  useEffect(() => {
    if (!disabled && !readOnly && !current.current.composing && current.current.draft.trim() !== current.current.confirmed) {
      clearTimer();
      timer.current = setTimeout(() => flushRef.current(), 350);
    }
  }, [disabled, readOnly]);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      clearTimer();
      // The keyed editor owns its final draft and callback after navigation.
      // Finish it without writing React state into its replacement editor.
      flushRef.current();
    };
  }, []);

  return <div className={`settings-field auto-save-name-field ${className}`}>
    <label htmlFor={id}>{label}</label>
    <span className="settings-field__inline">
      <ConfigInput expandable id={id} value={draft} maxLength={64} disabled={disabled} readOnly={readOnly}
        placeholder={placeholder} autoComplete="off"
        aria-invalid={error ? true : undefined} aria-describedby={error ? `${id}-error` : undefined}
        onValueChange={change}
        onFocus={() => { current.current.focused = true; }}
        onGroupBlur={event => {
          if (!event.currentTarget.contains(event.relatedTarget)) {
            current.current.focused = false;
            void flush();
          }
        }}
        onCompositionStart={() => { current.current.composing = true; clearTimer(); }}
        onCompositionEnd={event => { current.current.composing = false; change(event.currentTarget.value); }}
        onKeyDown={event => {
          if (event.key !== "Enter" || event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229 || current.current.composing) return;
          event.preventDefault();
          event.stopPropagation();
          void flush();
        }} />
    </span>
    {error && <div id={`${id}-error`} className="auto-save-name-field__error"><InlineFeedback tone="error">{error.message}{error.retry && <button type="button" className="settings-link" disabled={disabled || readOnly} onClick={() => {
      current.current.failedRevision = -1;
      setError(null);
      void flush();
    }}>{I18N.settings.retryUpdate}</button>}</InlineFeedback></div>}
  </div>;
}
