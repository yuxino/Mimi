import { Captions } from "lucide-react";
import { ProviderIcon } from "../../components/ProviderIcon";
import { Tooltip } from "../../components/Tooltip";
import { I18N } from "../../lib/i18n";
import type { translationService } from "./translationService";
import "./OverlayTranslationService.css";

export function OverlayTranslationService({ service, onClick, disabled }: {
  service: NonNullable<ReturnType<typeof translationService>>;
  onClick: () => void;
  disabled: boolean;
}) {
  return <div className="overlay-service">
    <Tooltip label={`${service.detail}\n${I18N.overlay.openSettings}`} popupClassName="overlay-service-tooltip">
      {(descriptionId, hovered) => <button type="button" className="overlay-service__button"
        aria-label={service.detail} aria-describedby={descriptionId}
        data-hovered={hovered || undefined} disabled={disabled} onClick={onClick}>
        {service.provider ? <ProviderIcon provider={service.provider} size={16} />
          : <Captions size={16} strokeWidth={1.7} aria-hidden="true" />}
        <span className="overlay-service__name">{service.label}</span>
      </button>}
    </Tooltip>
  </div>;
}
