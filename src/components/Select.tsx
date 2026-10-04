import { useEffect, useId, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Icon } from "./Icon";
import "./select.css";

type MenuStyle = CSSProperties & {
  "--select-hover": string;
  "--provider-light-display": string;
  "--provider-dark-display": string;
  "--provider-backing-background": string;
};

export interface SelectOption {
  value: string;
  label: string;
  /** Decorative mark; the label remains the option's searchable name. */
  icon?: ReactNode;
}

interface SelectProps {
  label: string;
  value: string;
  options: readonly SelectOption[];
  disabled?: boolean;
  /** Enables an explicit filter input; callers supply localized copy. */
  searchLabel?: string;
  emptyMessage?: string;
  /** Visible progress for choices loaded after opening a searchable picker. */
  loadingMessage?: string;
  /** Label for a saved selection that is not in the current results. */
  valueLabel?: string;
  /** Decorative fallback for a saved selection not in the current results. */
  valueIcon?: ReactNode;
  /** Load choices only after the user opens the picker. */
  onOpen?: () => void;
  onChange: (value: string) => void;
}

/** One app-styled picker for Settings, the subtitle controls, and the tray. */
export function Select({ label, value, options, disabled = false, searchLabel, emptyMessage, loadingMessage, valueLabel, valueIcon, onOpen, onChange }: SelectProps) {
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const revealActive = useRef(true);
  const optionNodes = useRef(new Map<string, HTMLDivElement>());
  const typeahead = useRef({ text: "", at: 0 });
  const [popup, setPopup] = useState<MenuStyle | null>(null);
  const [query, setQuery] = useState("");
  const searchable = searchLabel !== undefined;
  const visible = useMemo(() => {
    const text = searchable ? query.trim().toLocaleLowerCase() : "";
    return options.filter(option => !text || option.label.toLocaleLowerCase().includes(text) || option.value.toLocaleLowerCase().includes(text));
  }, [options, query, searchable]);
  const selected = options.findIndex((option) => option.value === value);
  const selectedIcon = selected >= 0 ? options[selected].icon : valueIcon;
  const selectedVisible = visible.findIndex(option => option.value === value);
  const [cursor, setCursor] = useState({ selection: value, query: "", index: 0 });
  // A shortcut/another window can change the value while the menu is open.
  const active = visible.length === 0 ? -1 : Math.min(visible.length - 1,
    cursor.selection === value && cursor.query === query && cursor.index >= 0 ? cursor.index : Math.max(0, selectedVisible));
  const open = popup !== null && !disabled;

  function setActive(index: number | ((previous: number) => number), reveal = true) {
    revealActive.current = reveal;
    setCursor(previous => ({
      selection: value,
      query,
      index: typeof index === "number" ? index : index(previous.selection === value && previous.query === query && previous.index >= 0 ? previous.index : Math.max(0, selectedVisible)),
    }));
  }

  function show() {
    const button = trigger.current;
    if (!button || disabled || options.length === 0) return;
    button.focus();
    const rect = button.getBoundingClientRect();
    const theme = getComputedStyle(button);
    const width = Math.min(Math.max(rect.width, searchable ? 280 : 200), window.innerWidth - 16);
    const below = window.innerHeight - rect.bottom - 12;
    const above = rect.top - 12;
    // Search results may arrive asynchronously. Reserve enough room for them
    // when opening; a short list still sizes naturally below this maximum.
    const desired = searchable ? 280 : Math.min(options.length * 38 + 10, 280);
    const upwards = below < desired && above > below;
    // In short windows, allow the searchable menu to overlap the trigger so
    // the input and at least one result remain usable within the viewport.
    const height = Math.min(desired, Math.max(searchable ? 90 : 40, upwards ? above : below), ...(searchable ? [Math.max(0, window.innerHeight - 16)] : []));
    setPopup({
      position: "fixed",
      width,
      maxHeight: height,
      left: Math.max(8, Math.min(searchable ? rect.right - width : rect.left, window.innerWidth - width - 8)),
      ...(searchable
        ? upwards
          ? { bottom: Math.max(8, Math.min(window.innerHeight - rect.top + 5, window.innerHeight - height - 8)) }
          : { top: Math.max(8, Math.min(rect.bottom + 5, window.innerHeight - height - 8)) }
        : upwards ? { bottom: window.innerHeight - rect.top + 5 } : { top: rect.bottom + 5 }),
      color: theme.color,
      background: theme.getPropertyValue("--select-menu-bg"),
      borderColor: theme.getPropertyValue("--select-menu-border"),
      fontFamily: theme.fontFamily,
      "--select-hover": theme.getPropertyValue("--select-hover"),
      "--provider-light-display": theme.getPropertyValue("--provider-light-display"),
      "--provider-dark-display": theme.getPropertyValue("--provider-dark-display"),
      "--provider-backing-background": theme.getPropertyValue("--provider-backing-background"),
    });
    setQuery("");
    revealActive.current = true;
    // Follow the saved choice while lazy options arrive, until the user navigates.
    setCursor({ selection: value, query: "", index: -1 });
    typeahead.current = { text: "", at: 0 };
    onOpen?.();
  }

  function choose(index: number) {
    const option = visible[index];
    setPopup(null);
    trigger.current?.focus();
    if (option && option.value !== value) onChange(option.value);
  }

  useEffect(() => {
    if (!open) return;
    const outside = (event: Event) => {
      const node = event.target as Node;
      if (!trigger.current?.contains(node) && !menu.current?.contains(node)) setPopup(null);
    };
    const dismiss = () => setPopup(null);
    const scroll = (event: Event) => {
      if (!menu.current?.contains(event.target as Node)) dismiss();
    };
    document.addEventListener("pointerdown", outside, true);
    document.addEventListener("focusin", outside);
    document.addEventListener("scroll", scroll, true);
    window.addEventListener("resize", dismiss);
    window.addEventListener("blur", dismiss);
    return () => {
      document.removeEventListener("pointerdown", outside, true);
      document.removeEventListener("focusin", outside);
      document.removeEventListener("scroll", scroll, true);
      window.removeEventListener("resize", dismiss);
      window.removeEventListener("blur", dismiss);
    };
  }, [open]);

  useEffect(() => {
    const option = visible[active];
    const node = option && optionNodes.current.get(option.value);
    const scroller = searchable ? list.current : menu.current;
    if (!open || !node || !scroller || !revealActive.current) return;
    // scrollIntoView also scrolls overflow:hidden ancestors in WebKit,
    // moving the search field and clipping the first row. Scroll only results.
    const row = node.getBoundingClientRect();
    const bounds = scroller.getBoundingClientRect();
    const top = bounds.top + scroller.clientTop;
    const bottom = top + scroller.clientHeight;
    if (row.top < top) scroller.scrollTop -= top - row.top;
    else if (row.bottom > bottom) scroller.scrollTop += row.bottom - bottom;
  }, [active, cursor, open, searchable, visible]);

  useEffect(() => {
    if (open && searchable) input.current?.focus({ preventScroll: true });
  }, [open, searchable]);

  function onKeyDown(event: KeyboardEvent<HTMLButtonElement | HTMLInputElement>) {
    const editing = event.currentTarget === input.current;
    if (editing && event.nativeEvent.isComposing) return;
    if (event.key === "Tab" || event.key === "Escape") {
      if (open && event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
      }
      setPopup(null);
      // The input lives in a body portal. Restore the form's tab starting
      // point before the browser performs its normal Tab/Shift+Tab action.
      if (editing) trigger.current?.focus({ preventScroll: true });
      return;
    }
    if (["ArrowDown", "ArrowUp", "Home", "End", "Enter"].includes(event.key) || (!editing && event.key === " ")) {
      event.preventDefault();
      if (!open) { show(); return; }
      if (visible.length === 0) return;
      if (event.key === "Enter" || event.key === " ") choose(active);
      else if (event.key === "Home") setActive(0);
      else if (event.key === "End") setActive(visible.length - 1);
      else setActive((index) => (index + (event.key === "ArrowDown" ? 1 : -1) + visible.length) % visible.length);
      return;
    }
    if (event.key.length === 1 && !event.metaKey && !event.ctrlKey && !event.altKey) {
      if (editing) return; // Let native text editing and IME produce the query.
      if (searchable) {
        event.preventDefault();
        if (!open) show();
        updateQuery(open ? query + event.key : event.key);
        return;
      }
      if (!open) show();
      const now = event.timeStamp;
      const text = (now - typeahead.current.at < 700 ? typeahead.current.text : "") + event.key.toLocaleLowerCase();
      typeahead.current = { text, at: now };
      const index = options.findIndex((option) => option.label.toLocaleLowerCase().startsWith(text));
      if (index >= 0) setActive(index);
    }
  }

  function updateQuery(text: string) {
    revealActive.current = true;
    if (list.current) list.current.scrollTop = 0;
    setQuery(text);
    setCursor({ selection: value, query: text, index: 0 });
  }

  const rows = visible.map((option, index) => (
    <div key={option.value} id={`${id}-${index}`} ref={node => {
      if (node) optionNodes.current.set(option.value, node);
      else optionNodes.current.delete(option.value);
    }} className="mimi-select__option" role="option"
      aria-selected={option.value === value} data-active={index === active}
      onPointerMove={() => setActive(index, false)} onPointerDown={(event) => event.preventDefault()}
      onClick={() => choose(index)}>
      <span className="mimi-select__content">
        {option.icon && <span className="mimi-select__icon" aria-hidden="true">{option.icon}</span>}
        <span className="mimi-select__label">{option.label}</span>
      </span>{option.value === value && <Icon name="checkmark" />}
    </div>
  ));
  const activeId = open && active >= 0 ? `${id}-${active}` : undefined;

  return (
    <span className="mimi-select">
      <button ref={trigger} type="button" className="mimi-select__trigger" role="combobox"
        aria-label={label} aria-haspopup="listbox" aria-expanded={open}
        aria-controls={open ? id : undefined} aria-activedescendant={activeId}
        disabled={disabled || options.length === 0} onKeyDown={onKeyDown}
        onClick={() => open ? setPopup(null) : show()}>
        {/* Replacing the label node also invalidates retained WebKit pixels on
            external value changes, while the focused trigger remains stable. */}
        <span key={value} className="mimi-select__content">
          {selectedIcon && <span className="mimi-select__icon" aria-hidden="true">{selectedIcon}</span>}
          <span className="mimi-select__label">{options[selected]?.label ?? valueLabel ?? value}</span>
        </span><Icon name="chevron-down" />
      </button>
      {open && createPortal(
        <div ref={menu} id={searchable ? undefined : id} className={`mimi-select__menu${searchable ? " mimi-select__menu--searchable" : ""}`}
          role={searchable ? undefined : "listbox"} aria-label={searchable ? undefined : label} style={popup}>
          {searchable ? <>
            <input ref={input} className="mimi-select__search" type="text" role="combobox"
              aria-label={searchLabel} placeholder={searchLabel} aria-expanded={true}
              aria-autocomplete="list" aria-haspopup="listbox" aria-controls={id} aria-activedescendant={activeId}
              autoComplete="off" spellCheck={false} value={query}
              onChange={event => updateQuery(event.currentTarget.value)} onKeyDown={onKeyDown} />
            {loadingMessage && <div className="mimi-select__empty" role="status">{loadingMessage}</div>}
            <div ref={list} id={id} className="mimi-select__options" role="listbox" aria-label={label} aria-busy={loadingMessage ? true : undefined}>
              {rows}
              {visible.length === 0 && !loadingMessage && <div className="mimi-select__empty" role="status">{emptyMessage}</div>}
            </div>
          </> : rows}
        </div>, document.body,
      )}
    </span>
  );
}
