import { connectionDiagnosticMessage, diagnosticCopy, type DiagnosticPlatform } from "../../lib/connectionDiagnostics";
import type { ConnectionDiagnostic } from "../../lib/ipc";
import { Icon } from "../../components/Icon";
import { InlineFeedback } from "./SettingsPrimitives";
import { SettingsHelp } from "./SettingsHelp";
import { I18N } from "../../lib/i18n";
import { useMemo } from "react";
import type { ProviderCredentialsInput } from "../../lib/types";

export type DraftCheckOutcome = { input: symbol; result: ConnectionDiagnostic | null; error: string | null };

/** A content-free identity keeps earlier results off a changed draft. Private
 * input comparisons stay local; neither input nor signature enters diagnostics. */
export function DraftConnectionCheck({ draft, outcome, onCheck, ...props }: {
  draft?: ProviderCredentialsInput | null;
  outcome?: DraftCheckOutcome;
  onCheck: (input: symbol) => void;
  pending: boolean;
  disabled: boolean;
  label?: string;
}) {
  const signature = JSON.stringify(draft);
  const input = useMemo(() => Symbol(signature === undefined ? "saved" : "draft"), [signature]);
  const current = outcome?.input === input ? outcome : undefined;
  return <ConnectionCheck {...props} result={current?.result ?? null} error={current?.error ?? null}
    disabled={props.disabled || draft === null} onCheck={() => onCheck(input)} />;
}

export function ConnectionCheck({ result, error, pending, disabled, onCheck, platform, label }: {
  result: ConnectionDiagnostic | null; error: string | null;
  pending: boolean; disabled: boolean; onCheck: () => void; platform?: DiagnosticPlatform;
  label?: string;
}) {
  const labels = diagnosticCopy(platform);
  const tone = result?.service === "available" ? "success" : result && (result.service === "unavailable" || result.credential !== "present" || result.reason) ? "error" : "info";
  const elapsedMs = result?.service !== "notTested" && typeof result?.elapsedMs === "number" && Number.isFinite(result.elapsedMs) && result.elapsedMs >= 0 ? Math.round(result.elapsedMs) : null;
  return <div className="connection-check">
    {result && !pending && <InlineFeedback tone={tone} icon={tone === "info" ? "help" : undefined}>{connectionDiagnosticMessage(result, platform)}{elapsedMs !== null && <span className="connection-check__elapsed">{labels.elapsed}: {elapsedMs} ms</span>}</InlineFeedback>}
    {error && <InlineFeedback tone="error">{error}</InlineFeedback>}
    {elapsedMs !== null && !pending && <SettingsHelp text={labels.elapsedHelp} label={I18N.settings.helpLabel} />}
    <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={disabled || pending} aria-busy={pending || undefined} onClick={onCheck}>{pending ? <span className="settings-spinner" aria-hidden="true" /> : <Icon name="checkmark-circle" />}{pending ? labels.testing : label ?? labels.test}</button>
  </div>;
}
