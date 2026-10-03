import { useEffect, useRef, useState, type FocusEventHandler, type InputHTMLAttributes, type Ref } from "react";
import { readClipboardText } from "../../lib/clipboard";
import { Icon } from "../../components/Icon";
import { I18N } from "../../lib/i18n";

/** Clipboard access happens only on the user's paste action; the owning field decides when to commit. */
export function ConfigInput({ onValueChange, onPasteValue, onGroupBlur, ref, ...props }: Omit<InputHTMLAttributes<HTMLInputElement>, "onChange"> & {
  onValueChange: (value: string) => void;
  onPasteValue?: (value: string) => void;
  onGroupBlur?: FocusEventHandler<HTMLSpanElement>;
  ref?: Ref<HTMLInputElement>;
}) {
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const pending = useRef(false);
  const mounted = useRef(false);
  const disabled = useRef(!!props.disabled);
  useEffect(() => { disabled.current = !!props.disabled; }, [props.disabled]);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const paste = async () => {
    if (props.disabled || pending.current) return;
    pending.current = true;
    setBusy(true); setFailed(false);
    try {
      const value = await readClipboardText();
      if (mounted.current && !disabled.current) {
        if (value.length > (props.maxLength ?? 2_048)) throw new Error("clipboard_value_too_long");
        onValueChange(value);
        onPasteValue?.(value);
      }
    } catch {
      if (mounted.current) setFailed(true);
    } finally {
      pending.current = false;
      if (mounted.current) setBusy(false);
    }
  };
  return <span className="config-input-group" onBlur={onGroupBlur}>
    <span className="config-input">
      <input {...props} ref={ref} onChange={event => { setFailed(false); onValueChange(event.target.value); }} />
      <button type="button" className="config-input__paste" aria-label={I18N.settings.pasteFromClipboard} title={I18N.settings.pasteFromClipboard}
        disabled={props.disabled || busy} aria-busy={busy || undefined} onClick={() => { void paste(); }}><Icon name="clipboard" /><span className="settings-sr-only">{I18N.settings.pasteFromClipboard}</span></button>
    </span>
    {failed && <span className="credential-unavailable" role="status">{I18N.settings.pasteFailed}</span>}
  </span>;
}
