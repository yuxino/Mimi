import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { I18N } from "../../../../lib/i18n";
import {
  testProfileConnection,
  type ConnectionCheckStage,
  type ConnectionDiagnostic,
} from "../../../../lib/ipc";
import {
  credentialErrorMessage,
  diagnosticCopy,
} from "../../../../lib/providers/connectionDiagnostics";
import {
  DEFAULT_NETWORK_PROXY,
  networkProxyConfigKey,
} from "../../../../lib/providers/networkProxy";
import type {
  ProviderCredentialsInput,
  ServiceProfile,
  SessionStateEvent,
  SettingsSnapshot,
  SourceLanguage,
} from "../../../../lib/types";

const CONNECTION_CHECK_TIMEOUT_MS = 30_000;

type CheckStage = ConnectionCheckStage | "combined";
type CheckOutcome = {
  profileId: string;
  input: symbol;
  result: ConnectionDiagnostic | null;
  error: string | null;
};

interface ProfileConnectionChecksOptions {
  settings: Pick<SettingsSnapshot, "profiles" | "networkProxy" | "activeProfileId">;
  sessionStatusKind: SessionStateEvent["status"]["kind"];
  selectedProfile: ServiceProfile | undefined;
  actionPending: boolean;
  setPendingAction: (action: "test-connection" | null) => void;
}

/** Owns connection-check requests and results for one mounted catalog page.
 * Results belong to a profile, stage, draft identity and transport revision;
 * changed credentials/proxies, session errors and teardown invalidate them. */
export function useProfileConnectionChecks({
  settings,
  sessionStatusKind,
  selectedProfile,
  actionPending,
  setPendingAction,
}: ProfileConnectionChecksOptions) {
  const [diagnostics, setDiagnostics] = useState<Partial<Record<CheckStage, CheckOutcome>>>({});
  const [pendingCheckStage, setPendingCheckStage] = useState<CheckStage | null>(null);
  const mounted = useRef(false);
  const checkRequest = useRef(0);
  const checkInFlight = useRef(false);
  const checkTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const profileCheckEpochs = useRef(new Map<string, number>());

  const proxyKeys = useMemo(() => Object.fromEntries(
    settings.profiles.map(profile => {
      const speech = JSON.stringify([
        networkProxyConfigKey(
          profile.speechNetworkProxy ?? settings.networkProxy ?? DEFAULT_NETWORK_PROXY,
        ),
        profile.alibabaRealtimeMode ?? "disabled",
        profile.alibabaWorkspaceId ?? "",
        profile.xaiSpeechMode ?? "voice",
        profile.speechTranslationMode ?? "simultaneous",
      ]);
      const text = JSON.stringify([
        networkProxyConfigKey(
          profile.textNetworkProxy ?? settings.networkProxy ?? DEFAULT_NETWORK_PROXY,
        ),
        profile.xaiSpeechMode ?? "voice",
        profile.speechTranslationMode ?? "simultaneous",
      ]);
      return [profile.id, { speech, text, combined: JSON.stringify([speech, text]) }];
    }),
  ), [settings.profiles, settings.networkProxy]);
  const proxyKey = JSON.stringify(proxyKeys);
  const [renderedProxies, setRenderedProxies] = useState({ key: proxyKey, keys: proxyKeys });
  const latestProxyKeys = useRef(proxyKeys);
  useEffect(() => { latestProxyKeys.current = proxyKeys; }, [proxyKeys]);
  const [renderedSession, setRenderedSession] = useState({
    kind: sessionStatusKind,
    profileId: settings.activeProfileId,
  });

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      checkRequest.current += 1;
      checkInFlight.current = false;
      if (checkTimer.current !== null) clearTimeout(checkTimer.current);
    };
  }, []);

  const invalidateProfileCheck = useCallback((profileId: string, stage?: ConnectionCheckStage) => {
    const epochs = profileCheckEpochs.current;
    epochs.set(profileId, (epochs.get(profileId) ?? 0) + 1);
    setDiagnostics(current => Object.fromEntries(
      Object.entries(current).filter(([key, outcome]) =>
        outcome?.profileId !== profileId ||
        (stage && key !== stage && key !== "combined"),
      ),
    ));
  }, []);

  if (renderedProxies.key !== proxyKey) {
    const previous = renderedProxies.keys;
    setRenderedProxies({ key: proxyKey, keys: proxyKeys });
    setDiagnostics(current => Object.fromEntries(
      Object.entries(current).filter(([stage, outcome]) =>
        outcome &&
        previous[outcome.profileId]?.[stage as CheckStage] ===
          proxyKeys[outcome.profileId]?.[stage as CheckStage],
      ),
    ));
  }

  useEffect(() => {
    if (sessionStatusKind !== "error") return;
    const epochs = profileCheckEpochs.current;
    epochs.set(settings.activeProfileId, (epochs.get(settings.activeProfileId) ?? 0) + 1);
  }, [sessionStatusKind, settings.activeProfileId]);

  if (renderedSession.kind !== sessionStatusKind ||
    renderedSession.profileId !== settings.activeProfileId) {
    setRenderedSession({ kind: sessionStatusKind, profileId: settings.activeProfileId });
    if (sessionStatusKind === "error") {
      setDiagnostics(current => Object.fromEntries(
        Object.entries(current).filter(([, outcome]) =>
          outcome?.profileId !== settings.activeProfileId,
        ),
      ));
    }
  }

  const handleConnectionCheck = (
    input: symbol,
    stage?: ConnectionCheckStage,
    credentials?: ProviderCredentialsInput,
    sourceLanguage?: SourceLanguage,
  ) => {
    if (!selectedProfile || actionPending || checkInFlight.current) return;
    checkInFlight.current = true;
    const profileId = selectedProfile.id;
    const key = stage ?? "combined";
    const publish = (result: ConnectionDiagnostic | null, error: string | null) =>
      setDiagnostics(current => ({
        ...current,
        [key]: { profileId, input, result, error },
      }));
    const request = ++checkRequest.current;
    const epoch = profileCheckEpochs.current.get(profileId) ?? 0;
    const checkedProxyKey = proxyKeys[profileId]?.[key];
    const canPublish = () => mounted.current &&
      request === checkRequest.current &&
      checkedProxyKey === latestProxyKeys.current[profileId]?.[key] &&
      epoch === (profileCheckEpochs.current.get(profileId) ?? 0);

    setPendingAction("test-connection");
    setPendingCheckStage(key);
    setDiagnostics(current => ({ ...current, [key]: undefined }));
    checkTimer.current = setTimeout(() => {
      if (!mounted.current || request !== checkRequest.current) return;
      const publishTimeout = canPublish();
      checkRequest.current += 1;
      checkInFlight.current = false;
      checkTimer.current = null;
      if (publishTimeout) publish(null, I18N.settings.profileCheckTimedOut);
      setPendingCheckStage(null);
      setPendingAction(null);
    }, CONNECTION_CHECK_TIMEOUT_MS);

    void (sourceLanguage
      ? testProfileConnection(profileId, stage, credentials, sourceLanguage)
      : credentials
        ? testProfileConnection(profileId, stage, credentials)
        : stage
          ? testProfileConnection(profileId, stage)
          : testProfileConnection(profileId))
      .then(result => {
        if (canPublish()) publish(result, null);
      })
      .catch((error: unknown) => {
        if (canPublish()) {
          publish(null, credentialErrorMessage(error) ??
            `${diagnosticCopy().unavailable}: ${diagnosticCopy().checkFailed}`);
        }
      })
      .finally(() => {
        if (mounted.current && request === checkRequest.current) {
          if (checkTimer.current !== null) clearTimeout(checkTimer.current);
          checkTimer.current = null;
          checkInFlight.current = false;
          setPendingCheckStage(null);
          setPendingAction(null);
        }
      });
  };

  return { diagnostics, pendingCheckStage, invalidateProfileCheck, handleConnectionCheck };
}
