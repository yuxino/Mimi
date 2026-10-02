import { connectionDiagnosticMessage, diagnosticCopy, type DiagnosticPlatform } from "../../lib/connectionDiagnostics";
import type { ConnectionDiagnostic } from "../../lib/ipc";
import { Icon } from "../../components/Icon";
import { InlineFeedback } from "./SettingsPrimitives";

export function ConnectionCheck({ result, error, pending, disabled, onCheck, platform }: {
  result: ConnectionDiagnostic | null; error: string | null;
  pending: boolean; disabled: boolean; onCheck: () => void; platform?: DiagnosticPlatform;
}) {
  const labels = diagnosticCopy(platform);
  const tone = result?.service === "available" ? "success" : result?.service === "unavailable" ? "error" : "info";
  return <div className="connection-check">
    <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={disabled || pending} onClick={onCheck}><Icon name="checkmark-circle" />{pending ? labels.testing : labels.test}</button>
    {result && <InlineFeedback tone={tone}>{connectionDiagnosticMessage(result, platform)}</InlineFeedback>}
    {error && <InlineFeedback tone="error">{error}</InlineFeedback>}
  </div>;
}
