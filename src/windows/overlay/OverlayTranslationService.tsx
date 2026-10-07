import { Fragment } from "react";
import { ArrowRight } from "lucide-react";
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
  return <div className={`overlay-service${service.stages.length > 1 ? " overlay-service--split" : ""}`}>
    <Tooltip label={`${service.detail}\n${I18N.overlay.openCurrentProfile}`} popupClassName="overlay-service-tooltip">
      {(descriptionId, hovered) => <button type="button" className="overlay-service__button"
        aria-label={service.detail} aria-describedby={descriptionId}
        data-hovered={hovered || undefined} disabled={disabled} onClick={onClick}>
        {service.stages.map((stage, index) => <Fragment key={stage.role}>
          {index > 0 && <ArrowRight className="overlay-service__separator" size={10} strokeWidth={1.7} aria-hidden="true" />}
          <span className="overlay-service__stage" data-stage={stage.role}>
            <ProviderIcon provider={stage.provider} size={16} />
            <span className="overlay-service__name">{stage.label}</span>
          </span>
        </Fragment>)}
      </button>}
    </Tooltip>
  </div>;
}
