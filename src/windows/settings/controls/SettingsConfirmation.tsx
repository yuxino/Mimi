import { useId, useLayoutEffect, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Icon, type IconName } from "../../../components/Icon";
import { I18N } from "../../../lib/i18n";
import "./settings-confirmation.css";

export interface SettingsConfirmationProps {
  message: string;
  children?: ReactNode;
  disabled?: boolean;
  onCancel: () => void;
  onConfirm: () => void;
  cancelLabel?: string;
  hideConfirm?: boolean;
  confirmLabel?: string;
  confirmIcon?: IconName;
  variant?: "default" | "danger";
}

const THEME_PROPERTIES = [
  "--mimi-ui-font", "--settings-canvas", "--settings-panel-raised",
  "--settings-control", "--settings-border-strong", "--settings-text",
  "--settings-text-muted", "--settings-text-faint", "--settings-border", "--settings-danger",
  "--settings-selection-soft", "--settings-ink",
  "--provider-light-display", "--provider-dark-display", "--provider-backing-background",
];

function focusableElements(dialog: HTMLElement): HTMLElement[] {
  return [...dialog.querySelectorAll<HTMLElement>(
    'button, a[href], input, select, textarea, [tabindex]',
  )].filter(element => element.tabIndex >= 0 && !element.matches(":disabled")
    && !element.closest("[hidden], [inert]")
    && getComputedStyle(element).display !== "none"
    && getComputedStyle(element).visibility !== "hidden");
}

/** Standard settings modal. Mount to open; unmount to restore the prior focus. */
export function SettingsConfirmation({
  message,
  children,
  disabled = false,
  onCancel,
  onConfirm,
  cancelLabel = I18N.settings.cancel,
  hideConfirm = false,
  confirmLabel = I18N.settings.confirmDelete,
  confirmIcon = "trash",
  variant = "danger",
}: SettingsConfirmationProps) {
  const messageId = useId();
  const anchorRef = useRef<HTMLSpanElement>(null);
  const backdropRef = useRef<HTMLDivElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const cancelAction = useRef(onCancel);

  useLayoutEffect(() => { cancelAction.current = onCancel; }, [onCancel]);

  useLayoutEffect(() => {
    const backdrop = backdropRef.current;
    const dialog = dialogRef.current;
    if (!backdrop || !dialog) return;
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const source = anchorRef.current?.closest(".settings-console") ?? anchorRef.current?.parentElement;
    const syncTheme = () => {
      if (!source) return;
      const theme = getComputedStyle(source);
      for (const property of THEME_PROPERTIES) {
        const value = theme.getPropertyValue(property);
        if (value) backdrop.style.setProperty(property, value);
      }
      backdrop.style.colorScheme = theme.colorScheme;
      backdrop.style.fontFamily = theme.fontFamily;
    };
    syncTheme();
    const themeObserver = new MutationObserver(syncTheme);
    if (source) themeObserver.observe(source, { attributes: true, attributeFilter: ["class", "style"] });
    themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ["lang", "class", "style"] });

    const background = new Map<HTMLElement, { inert: string | null; ariaHidden: string | null }>();
    const restoreBackground = (element: HTMLElement) => {
      const original = background.get(element);
      if (!original) return;
      for (const [attribute, value] of [["inert", original.inert], ["aria-hidden", original.ariaHidden]] as const) {
        if (value === null) element.removeAttribute(attribute);
        else element.setAttribute(attribute, value);
      }
      background.delete(element);
    };
    const isDialogTooltip = (element: HTMLElement) => element.getAttribute("role") === "tooltip"
      && Boolean(element.id)
      && ([...dialog.querySelectorAll("[aria-describedby]")].some(trigger => trigger.getAttribute("aria-describedby")?.split(/\s+/).includes(element.id))
        || [...dialog.querySelectorAll(".settings-help-control__description")].some(description => description.textContent === element.textContent));
    const blockBackground = () => {
      for (const element of [...document.body.children]) {
        if (!(element instanceof HTMLElement) || element === backdrop || isDialogTooltip(element)) continue;
        if (!background.has(element)) background.set(element, { inert: element.getAttribute("inert"), ariaHidden: element.getAttribute("aria-hidden") });
        element.setAttribute("inert", "");
        element.setAttribute("aria-hidden", "true");
      }
      for (const element of [...background.keys()]) {
        if (element.parentElement !== document.body) restoreBackground(element);
      }
    };
    blockBackground();
    const observer = new MutationObserver(blockBackground);
    observer.observe(document.body, { childList: true });
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";

    const focusCancel = () => cancelRef.current?.focus({ preventScroll: true });
    const trapFocus = (event: FocusEvent) => {
      if (event.target instanceof Node && !dialog.contains(event.target)) focusCancel();
    };
    const stopBackgroundInteraction = (event: Event) => {
      if (event.target instanceof Node && !dialog.contains(event.target)) {
        event.preventDefault();
        event.stopImmediatePropagation();
      }
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        if (event.isComposing || event.keyCode === 229) return;
        event.preventDefault();
        event.stopImmediatePropagation();
        cancelAction.current();
      } else if (event.key === "Tab") {
        event.preventDefault();
        const elements = focusableElements(dialog);
        const current = elements.indexOf(document.activeElement as HTMLElement);
        const next = current < 0 ? 0 : (current + (event.shiftKey ? -1 : 1) + elements.length) % elements.length;
        (elements[next] ?? cancelRef.current)?.focus({ preventScroll: true });
      }
    };
    document.addEventListener("focusin", trapFocus, true);
    // Nested controls consume Escape before the dialog. Their Tab handlers
    // restore the form anchor before this trap advances within the modal.
    document.addEventListener("keydown", handleKeyDown);
    for (const event of ["pointerdown", "mousedown", "click"]) document.addEventListener(event, stopBackgroundInteraction, true);
    focusCancel();

    return () => {
      observer.disconnect();
      themeObserver.disconnect();
      document.removeEventListener("focusin", trapFocus, true);
      document.removeEventListener("keydown", handleKeyDown);
      for (const event of ["pointerdown", "mousedown", "click"]) document.removeEventListener(event, stopBackgroundInteraction, true);
      for (const element of [...background.keys()]) restoreBackground(element);
      document.body.style.overflow = previousOverflow;
      if (previousFocus?.isConnected) previousFocus.focus({ preventScroll: true });
    };
  }, []);

  return <>
    <span ref={anchorRef} hidden />
    {createPortal(
      <div ref={backdropRef} className="settings-confirmation-backdrop">
        <div ref={dialogRef} className={`settings-confirmation settings-confirmation--${variant}`}
          role={variant === "danger" ? "alertdialog" : "dialog"} aria-modal="true" aria-labelledby={messageId}>
          <p id={messageId} className="settings-confirmation__message">{message}</p>
          {children && <div className="settings-confirmation__body">{children}</div>}
          <div className="settings-confirmation__actions">
            <button ref={cancelRef} type="button" className="settings-confirmation__button settings-confirmation__cancel" onClick={onCancel}>
              {cancelLabel}
            </button>
            {!hideConfirm && <button type="button" className={`settings-confirmation__button settings-confirmation__confirm settings-confirmation__confirm--${variant}`}
              disabled={disabled} onClick={onConfirm}>
              <Icon name={confirmIcon} />
              {confirmLabel}
            </button>}
          </div>
        </div>
      </div>, document.body,
    )}
  </>;
}
