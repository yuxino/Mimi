import { I18N } from "../../lib/i18n";
import { SettingsHelp } from "./SettingsHelp";
import type { ReactNode } from "react";
import { Select, type SelectOption } from "../../components/Select";
import { Icon, type IconName } from "../../components/Icon";

export function SettingsSection({
  id,
  title,
  children,
  hideHeading = false,
}: {
  id: string;
  title: string;
  children: ReactNode;
  hideHeading?: boolean;
}) {
  return (
    <section id={id} className="settings-card" aria-labelledby={`${id}-title`}>
      <header
        className={hideHeading ? "settings-sr-only" : "settings-card__header"}
      >
        <span className="settings-card__heading">
          <h2 id={`${id}-title`}>{title}</h2>
        </span>
      </header>
      <div className="settings-card__body">{children}</div>
    </section>
  );
}

export function SettingsRow({
  label,
  description,
  descriptionId,
  hint,
  children,
  feedback,
  align = "center",
}: {
  label: string;
  description?: string;
  descriptionId?: string;
  hint?: string;
  children: ReactNode;
  /** Ongoing state belongs below the controls, outside their intrinsic width. */
  feedback?: ReactNode;
  align?: "center" | "start";
}) {
  return (
    <div className={`settings-row settings-row--${align}`}>
      <span className="settings-row__copy">
        <span className="settings-row__label">{label}{(description || hint) && <SettingsHelp id={descriptionId} text={[description, hint].filter(Boolean).join("\n")} label={I18N.settings.helpLabel} />}</span>
      </span>
      <span className="settings-row__control">{children}</span>
      {feedback && <div className="settings-row__feedback">{feedback}</div>}
    </div>
  );
}

export function SettingsSelect({
  value,
  disabled = false,
  onChange,
  options,
  label,
}: {
  value: string;
  disabled?: boolean;
  onChange: (value: string) => void;
  options: readonly SelectOption[];
  label: string;
}) {
  return (
    <span className="settings-select-wrap">
      <Select label={label} value={value} disabled={disabled} options={options} onChange={onChange} />
    </span>
  );
}

export function InlineFeedback({
  tone,
  icon,
  children,
}: {
  tone: "success" | "error" | "info";
  icon?: IconName;
  children: ReactNode;
}) {
  return (
    <p
      className="settings-feedback"
      data-tone={tone}
      role={tone === "error" ? "alert" : "status"}
    >
      <Icon
        name={
          icon ??
          (tone === "success"
            ? "checkmark-circle"
            : tone === "error"
              ? "exclamation-triangle"
              : "sparkles")
        }
      />
      <span>{children}</span>
    </p>
  );
}
