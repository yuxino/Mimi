import { useCallback, useEffect, useLayoutEffect, useRef, useState, type FocusEventHandler, type InputHTMLAttributes, type ReactNode, type Ref } from "react";
import { readClipboardText } from "../../lib/clipboard";
import { Icon } from "../../components/Icon";
import { I18N } from "../../lib/i18n";
import { useSettingsToast } from "./useSettingsToast";

/** Clipboard access happens only on the user's paste action; the owning field decides when to commit. */
export type ConfigInputElement = HTMLInputElement | HTMLTextAreaElement;

export function ConfigInput({ onValueChange, onPasteValue, onGroupBlur, action, expandable = false, ref, type, onKeyDown, ...props }: Omit<InputHTMLAttributes<ConfigInputElement>, "onChange"> & {
  onValueChange: (value: string) => void;
  onPasteValue?: (value: string) => void;
  onGroupBlur?: FocusEventHandler<HTMLSpanElement>;
  action?: ReactNode;
  expandable?: boolean;
  ref?: Ref<ConfigInputElement>;
}) {
  const [busy, setBusy] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const { beginToast, clearToast } = useSettingsToast();
  const pending = useRef(false);
  const mounted = useRef(false);
  const disabled = useRef(!!props.disabled || !!props.readOnly);
  const field = useRef<ConfigInputElement | null>(null);
  const switchingField = useRef(false);
  const selection = useRef<{ start: number; end: number; direction: "forward" | "backward" | "none" } | null>(null);
  const canExpand = expandable && (type === undefined || type === "text" || type === "url" || type === "email" || type === "tel" || type === "search");
  const isExpanded = canExpand && expanded;
  const assignField = useCallback((element: ConfigInputElement | null) => {
    field.current = element;
    if (typeof ref === "function") return ref(element);
    if (ref) ref.current = element;
  }, [ref]);
  useEffect(() => { disabled.current = !!props.disabled || !!props.readOnly; }, [props.disabled, props.readOnly]);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useLayoutEffect(() => {
    if (!selection.current || !field.current) { switchingField.current = false; return; }
    const cursor = selection.current;
    selection.current = null;
    field.current.focus({ preventScroll: true });
    try { field.current.setSelectionRange(cursor.start, cursor.end, cursor.direction); } catch { /* Some native input types do not expose selection. */ }
    switchingField.current = false;
  }, [isExpanded]);
  useLayoutEffect(() => {
    const textarea = field.current;
    if (!isExpanded || !(textarea instanceof HTMLTextAreaElement)) return;
    const resize = () => {
      textarea.style.height = "auto";
      textarea.style.height = `${Math.min(240, Math.max(76, textarea.scrollHeight))}px`;
    };
    resize();
    if (typeof ResizeObserver === "undefined") return;
    let width = textarea.getBoundingClientRect().width;
    const observer = new ResizeObserver(() => {
      const nextWidth = textarea.getBoundingClientRect().width;
      if (nextWidth !== width) { width = nextWidth; resize(); }
    });
    observer.observe(textarea);
    return () => observer.disconnect();
  }, [isExpanded, props.value, props.placeholder]);
  const normalize = (value: string) => canExpand ? value.replace(/[\r\n]/g, "") : value;
  const change = (value: string) => { clearToast(); onValueChange(normalize(value)); };
  const toggle = () => {
    if (props.disabled) return;
    // WebKit may report relatedTarget=null while replacing a focused native
    // textbox. This internal focus transfer must not commit its owner's draft.
    switchingField.current = true;
    selection.current = { start: field.current?.selectionStart ?? 0, end: field.current?.selectionEnd ?? 0, direction: field.current?.selectionDirection ?? "none" };
    setExpanded(!isExpanded);
  };
  const paste = async () => {
    if (props.disabled || props.readOnly || pending.current) return;
    pending.current = true;
    setBusy(true);
    const notify = beginToast();
    try {
      const value = await readClipboardText();
      if (mounted.current && !disabled.current) {
        if (value.length > (props.maxLength ?? 2_048)) throw new Error("clipboard_value_too_long");
        const next = normalize(value);
        onValueChange(next);
        onPasteValue?.(next);
      }
    } catch {
      if (mounted.current) notify(I18N.settings.pasteFailed, true);
    } finally {
      pending.current = false;
      if (mounted.current) setBusy(false);
    }
  };
  return <span className="config-input-group" onBlur={event => { if (!switchingField.current) onGroupBlur?.(event); }}>
    <span className={`config-input${action ? " config-input--with-action" : ""}${canExpand ? " config-input--expandable" : ""}${isExpanded ? " config-input--expanded" : ""}`}>
      {isExpanded ? <textarea {...props} ref={assignField} rows={3} wrap="soft" onChange={event => change(event.target.value)} onKeyDown={event => {
        const composing = event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229;
        if (!composing && event.key === "Escape") { event.preventDefault(); event.stopPropagation(); toggle(); }
        if (!composing && event.key === "Enter") event.preventDefault();
        onKeyDown?.(event);
      }} /> : <input {...props} type={type} ref={assignField} onKeyDown={onKeyDown} onChange={event => change(event.target.value)} />}
      {action}
      {canExpand && <button type="button" className="config-input__expand" aria-label={isExpanded ? I18N.settings.collapseField : I18N.settings.expandField} title={isExpanded ? I18N.settings.collapseField : I18N.settings.expandField}
        aria-expanded={isExpanded} aria-controls={props.id} disabled={props.disabled} onMouseDown={event => event.preventDefault()} onClick={toggle}><Icon name={isExpanded ? "chevron-up" : "chevron-down"} /></button>}
      <button type="button" className="config-input__paste" aria-label={I18N.settings.pasteFromClipboard} title={I18N.settings.pasteFromClipboard}
        disabled={props.disabled || props.readOnly || busy} aria-busy={busy || undefined} onClick={() => { void paste(); }}><Icon name="clipboard" /><span className="settings-sr-only">{I18N.settings.pasteFromClipboard}</span></button>
    </span>
  </span>;
}
