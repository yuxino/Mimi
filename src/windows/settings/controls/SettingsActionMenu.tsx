import { useEffect, useId, useRef, useState, type CSSProperties, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { Icon, type IconName } from "../../../components/Icon";

type Action = { id: string; label: string; icon: IconName; disabled?: boolean; destructive?: boolean; onSelect: () => void };
type MenuStyle = CSSProperties & { "--action-hover": string; "--action-danger": string };

export function SettingsActionMenu({ id: triggerId, label, name, disabled, actions }: { id?: string; label: string; name: string; disabled: boolean; actions: Action[] }) {
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const focusLast = useRef(false);
  const [popup, setPopup] = useState<MenuStyle | null>(null);
  if (disabled && popup) setPopup(null);

  const close = (restoreFocus = false) => {
    setPopup(null);
    if (restoreFocus) trigger.current?.focus({ preventScroll: true });
  };
  const show = (last = false) => {
    focusLast.current = last;
    if (!trigger.current || disabled) return;
    const rect = trigger.current.getBoundingClientRect();
    const theme = getComputedStyle(trigger.current);
    const width = Math.min(200, window.innerWidth - 16);
    const height = actions.length * 36 + 10;
    setPopup({ position: "fixed", width,
      left: Math.max(8, Math.min(rect.right - width, window.innerWidth - width - 8)),
      ...(window.innerHeight - rect.bottom < height + 8 && rect.top > height
        ? { bottom: window.innerHeight - rect.top + 4 } : { top: Math.max(8, Math.min(rect.bottom + 4, window.innerHeight - height - 8)) }),
      maxHeight: Math.max(36, window.innerHeight - 16),
      background: theme.getPropertyValue("--settings-control"),
      borderColor: theme.getPropertyValue("--settings-border-strong"),
      color: theme.getPropertyValue("--settings-text"), fontFamily: theme.fontFamily,
      "--action-hover": theme.getPropertyValue("--settings-selection-soft"),
      "--action-danger": theme.getPropertyValue("--settings-danger"),
    });
  };

  useEffect(() => {
    if (!popup) return;
    const enabled = [...menu.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []];
    (enabled[focusLast.current ? enabled.length - 1 : 0] ?? menu.current)?.focus({ preventScroll: true });
    const outside = (event: Event) => {
      if (!menu.current?.contains(event.target as Node) && !trigger.current?.contains(event.target as Node)) setPopup(null);
    };
    const dismiss = () => setPopup(null);
    const scroll = (event: Event) => { if (!menu.current?.contains(event.target as Node)) dismiss(); };
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
  }, [popup]);

  const keyDown = (event: KeyboardEvent) => {
    if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); close(true); }
    else if (event.key === "Tab") close(true);
    else if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
      event.preventDefault();
      const items = [...menu.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []];
      const current = items.findIndex(item => item === document.activeElement);
      const next = event.key === "Home" ? 0 : event.key === "End" ? items.length - 1
        : (current + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
      items[next]?.focus();
    }
  };

  return <>
    <button ref={trigger} id={triggerId} type="button" className="service-row__action service-row__more"
      aria-label={`${label}: ${name}`} aria-haspopup="menu" aria-expanded={!!popup} aria-controls={popup ? id : undefined}
      disabled={disabled} onClick={() => popup ? close() : show()}
      onKeyDown={event => { if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); show(event.key === "ArrowUp"); } }}>
      {label}<Icon name="chevron-down" />
    </button>
    {popup && createPortal(<div ref={menu} id={id} className="settings-action-menu" style={popup}
      role="menu" aria-label={`${label}: ${name}`} tabIndex={-1} onKeyDown={keyDown}>
      {actions.map(action => <button key={action.id} type="button" role="menuitem" className={`service-row__${action.id}`}
        aria-label={`${action.label}: ${name}`} disabled={action.disabled} data-destructive={action.destructive || undefined}
        onClick={() => { if (!action.disabled) { close(true); action.onSelect(); } }}>
        <Icon name={action.icon} />{action.label}
      </button>)}
    </div>, document.body)}
  </>;
}
