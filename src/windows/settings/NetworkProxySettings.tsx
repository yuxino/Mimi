import { useEffect, useId, useRef, useState } from "react";
import { I18N } from "../../lib/i18n";
import { DEFAULT_NETWORK_PROXY, networkProxyConfigKey, validateNetworkProxy, type NetworkProxyValidationError } from "../../lib/networkProxy";
import type { NetworkProxyConfig, NetworkProxyMode } from "../../lib/types";
import { ConfigInput } from "./ConfigInput";
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
  const [error, setError] = useState<string | null>(null);
  const savedKey = networkProxyConfigKey(value);
  const [renderedKey, setRenderedKey] = useState(savedKey);
  const [acknowledgedKey, setAcknowledgedKey] = useState(savedKey);
  const inFlight = useRef(false);
  const mounted = useRef(false);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);

  if (renderedKey !== savedKey) {
    setRenderedKey(savedKey);
    // Optimistic snapshots and rollback must not erase a failed save's draft.
    if (!busy) { setMode(value.mode); setAddress(value.url ?? ""); setAcknowledgedKey(savedKey); setError(null); }
  }

  const validated = validateNetworkProxy(mode, address);
  const locked = disabled || busy;
  const help = mode === "system" ? I18N.settings.networkProxySystemHelp
    : mode === "direct" ? I18N.settings.networkProxyDirectHelp : I18N.settings.networkProxyCustomHelp;

  const save = async (nextMode = mode, nextAddress = address) => {
    if (disabled || inFlight.current) return;
    const next = validateNetworkProxy(nextMode, nextAddress);
    if ("error" in next) { setError(validationMessage(next.error)); return; }
    if (networkProxyConfigKey(next.config) === acknowledgedKey) return;
    inFlight.current = true;
    setBusy(true); setError(null);
    try {
      await onSave(next.config);
      if (mounted.current) {
        setMode(next.config.mode); setAddress(next.config.url ?? "");
        setAcknowledgedKey(networkProxyConfigKey(next.config));
      }
    } catch (error: unknown) {
      if (mounted.current) setError(saveErrorMessage(error));
    } finally {
      inFlight.current = false;
      if (mounted.current) setBusy(false);
    }
  };

  const control = <>
    <form className="network-proxy-form" aria-busy={busy} onSubmit={(event) => { event.preventDefault(); void save(); }}>
      <SettingsRow label={label} descriptionId={`${addressId}-help`} description={[scope, help, ...(disabled ? [I18N.settings.networkProxyLocked] : [])].join("\n")}>
        {busy && <span className="network-proxy-saving" role="status">
          <span className="settings-spinner" aria-hidden="true" />
          <span className="settings-sr-only">{I18N.settings.networkProxySaving}</span>
        </span>}
        <SettingsSelect label={label} value={mode} disabled={locked}
          options={[
            { value: "system", label: I18N.settings.networkProxySystem },
            { value: "direct", label: I18N.settings.networkProxyDirect },
            { value: "custom", label: I18N.settings.networkProxyCustom },
          ]}
          onChange={(value) => {
            const next = value as NetworkProxyMode;
            setMode(next); setError(null);
            if (next !== "custom") { setAddress(""); void save(next, ""); }
            else if (address.trim()) void save(next, address);
          }} />
      </SettingsRow>
      {mode === "custom" && <div className="settings-field network-proxy-address">
        <label htmlFor={addressId}>{I18N.settings.networkProxyAddress}</label>
        <ConfigInput expandable id={addressId} value={address} type="text" inputMode="url" maxLength={2_048} autoComplete="off" spellCheck={false}
          disabled={locked} aria-invalid={!!error && "error" in validated || undefined}
          aria-describedby={`${addressId}-help${error ? ` ${addressId}-error` : ""}`} placeholder={I18N.settings.proxyAddressPlaceholder}
          onValueChange={(value) => { setAddress(value); setError(null); }}
          onPasteValue={value => { void save("custom", value); }}
          onGroupBlur={event => {
            // Switching modes or pasting commits the new choice, not the old draft.
            if (event.relatedTarget instanceof Element && (event.currentTarget.contains(event.relatedTarget) || event.relatedTarget.closest('[role="combobox"]'))) return;
            void save();
          }} />
      </div>}
      {error && <div className="network-proxy-actions">
        <span id={`${addressId}-error`}><InlineFeedback tone="error">{error}</InlineFeedback></span>
        <button type="submit" className="settings-button settings-button--quiet settings-button--compact" disabled={locked}>
          {I18N.settings.networkProxySave}
        </button>
      </div>}
    </form>
  </>;
  return embedded ? control : <SettingsSection id="network-proxy" title={I18N.settings.networkProxyTitle}>{control}</SettingsSection>;
}
