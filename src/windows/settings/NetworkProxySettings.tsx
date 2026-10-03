import { useEffect, useId, useRef, useState, type FormEvent } from "react";
import { I18N } from "../../lib/i18n";
import { DEFAULT_NETWORK_PROXY, networkProxyConfigKey, validateNetworkProxy, type NetworkProxyValidationError } from "../../lib/networkProxy";
import type { NetworkProxyConfig, NetworkProxyMode } from "../../lib/types";
import { ConfigInput } from "./ConfigInput";
import { SettingsHelp } from "./SettingsHelp";
import { InlineFeedback, SettingsRow, SettingsSection, SettingsSelect } from "./SettingsPrimitives";

function validationMessage(error: NetworkProxyValidationError): string {
  return {
    invalidUrl: I18N.settings.networkProxyInvalidUrl,
    unsupportedScheme: I18N.settings.networkProxyUnsupportedScheme,
    authenticationUnsupported: I18N.settings.networkProxyAuthenticationUnsupported,
  }[error];
}

function saveErrorMessage(error: unknown): string {
  const label = typeof error === "string" ? error : error instanceof Error ? error.message : "";
  switch (label) {
    case "network_proxy_invalid_url": return I18N.settings.networkProxyInvalidUrl;
    case "network_proxy_unsupported_scheme": return I18N.settings.networkProxyUnsupportedScheme;
    case "network_proxy_authentication_unsupported": return I18N.settings.networkProxyAuthenticationUnsupported;
    case "network_proxy_change_requires_stop":
    case "session-active":
    case "Service profiles cannot be changed while a session is active.":
    case "Listening settings cannot be changed while a session is active.": return I18N.settings.networkProxyLocked;
    case "network_proxy_builder_failed": return I18N.settings.networkProxyBuilderFailed;
    default: return I18N.settings.networkProxySaveFailed;
  }
}

export function NetworkProxySettings({ value = DEFAULT_NETWORK_PROXY, disabled, onSave, label = I18N.settings.networkProxyMode, scope = I18N.settings.networkProxyScope, embedded = false }: {
  label?: string;
  scope?: string;
  embedded?: boolean;
  value: NetworkProxyConfig;
  disabled: boolean;
  onSave: (config: NetworkProxyConfig) => Promise<void>;
}) {
  const addressId = useId();
  const [mode, setMode] = useState(value.mode);
  const [address, setAddress] = useState(value.url ?? "");
  const [busy, setBusy] = useState(false);
  const [feedback, setFeedback] = useState<{ tone: "success" | "error"; message: string } | null>(null);
  const savedKey = networkProxyConfigKey(value);
  const [renderedKey, setRenderedKey] = useState(savedKey);
  const inFlight = useRef(false);
  const mounted = useRef(false);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);

  if (renderedKey !== savedKey) {
    setRenderedKey(savedKey);
    // Optimistic snapshots and rollback must not erase a failed save's draft.
    if (!busy) { setMode(value.mode); setAddress(value.url ?? ""); setFeedback(null); }
  }

  const validated = validateNetworkProxy(mode, address);
  const changed = "error" in validated || networkProxyConfigKey(validated.config) !== savedKey;
  const locked = disabled || busy;
  const help = mode === "system" ? I18N.settings.networkProxySystemHelp
    : mode === "direct" ? I18N.settings.networkProxyDirectHelp : I18N.settings.networkProxyCustomHelp;

  const save = async (event: FormEvent) => {
    event.preventDefault();
    if (disabled || inFlight.current || !changed) return;
    if ("error" in validated) { setFeedback({ tone: "error", message: validationMessage(validated.error) }); return; }
    inFlight.current = true;
    setBusy(true); setFeedback(null);
    try {
      await onSave(validated.config);
      if (mounted.current) {
        setMode(validated.config.mode); setAddress(validated.config.url ?? "");
        setFeedback({ tone: "success", message: I18N.settings.networkProxySaved });
      }
    } catch (error: unknown) {
      if (mounted.current) setFeedback({ tone: "error", message: saveErrorMessage(error) });
    } finally {
      inFlight.current = false;
      if (mounted.current) setBusy(false);
    }
  };

  const control = <>
    <form className="network-proxy-form" aria-busy={busy} onSubmit={(event) => { void save(event); }}>
      <SettingsRow label={label} align="start">
        <SettingsHelp id={`${addressId}-help`} text={[scope, help, ...(disabled ? [I18N.settings.networkProxyLocked] : [])].join("\n")} label={I18N.settings.helpLabel} />
        <SettingsSelect label={label} value={mode} disabled={locked}
          options={[
            { value: "system", label: I18N.settings.networkProxySystem },
            { value: "direct", label: I18N.settings.networkProxyDirect },
            { value: "custom", label: I18N.settings.networkProxyCustom },
          ]}
          onChange={(value) => { const next = value as NetworkProxyMode; setMode(next); if (next !== "custom") setAddress(""); setFeedback(null); }} />
      </SettingsRow>
      {mode === "custom" && <div className="settings-field network-proxy-address">
        <label htmlFor={addressId}>{I18N.settings.networkProxyAddress}</label>
        <ConfigInput id={addressId} value={address} type="text" inputMode="url" maxLength={2_048} autoComplete="off" spellCheck={false}
          disabled={locked} aria-describedby={`${addressId}-help`} placeholder="http://127.0.0.1:7890" onValueChange={(value) => { setAddress(value); setFeedback(null); }} />
      </div>}
      {(changed || busy || feedback) && <div className="network-proxy-actions">
        {(changed || busy) && <button type="submit" className="settings-button settings-button--quiet settings-button--compact" disabled={locked} aria-busy={busy || undefined}>
          {busy && <span className="settings-spinner" aria-hidden="true" />}
          {busy ? I18N.settings.networkProxySaving : I18N.settings.networkProxySave}
        </button>}
        {feedback && <InlineFeedback tone={feedback.tone}>{feedback.message}</InlineFeedback>}
      </div>}
    </form>
  </>;
  return embedded ? control : <SettingsSection id="network-proxy" title={I18N.settings.networkProxyTitle}>{control}</SettingsSection>;
}
