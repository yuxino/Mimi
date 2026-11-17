import { useShallow } from "zustand/react/shallow";
import { useStore } from "../../lib/store";
import { OverlayLatency } from "../overlay/OverlayLatency";
import { OverlayTranslationService } from "../overlay/OverlayTranslationService";
import { translationService } from "../overlay/translationService";
import "./OverlaySessionDetails.css";

/** Keep service observations independent of the panel's controls and streaming captions. */
export function OverlaySessionDetails({ onOpenProfile, disabled }: {
  onOpenProfile: () => void;
  disabled: boolean;
}) {
  const settings = useStore(state => state.settings);
  const status = useStore(state => state.session.status.kind);
  const session = useStore(useShallow(({ session }) => ({
    isActive: session.isActive,
    isPaused: session.isPaused,
    apiLatencyMs: session.apiLatencyMs,
    translationLatencyMs: session.translationLatencyMs,
    translationLatencyKind: session.translationLatencyKind,
    translationRecovery: session.translationRecovery,
    isTranslationPending: session.isTranslationPending,
    isTranslationPreviewPending: session.isTranslationPreviewPending,
  })));
  const service = translationService(settings);
  return <div className="overlay-session-details">
    {service?.showInProfileDetails && <OverlayTranslationService service={service} onClick={onOpenProfile} disabled={disabled} />}
    <OverlayLatency session={{ ...session, status: { kind: status } }} translationRequired={settings.targetLanguage !== "original"} />
  </div>;
}
