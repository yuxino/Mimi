import { Icon } from "../../components/Icon";
import { I18N } from "../../lib/i18n";
import {
  normalizeSubtitleHex,
  SUBTITLE_COLORS,
  SUBTITLE_COLOR_OPTIONS,
  subtitleColorHex,
} from "../../lib/subtitleColor";
import type { SubtitleColor } from "../../lib/types";

type Props = { label?: string; value: SubtitleColor; onChange: (color: SubtitleColor) => void };

export function SubtitleColorControl({ value, onChange, label = I18N.settings.subtitleColor }: Props) {
  const hex = subtitleColorHex(value);
  const isCustom = !SUBTITLE_COLOR_OPTIONS.some((option) => hex === SUBTITLE_COLORS[option.value]);
  return (
    <div className="subtitle-color-presets" role="group" aria-label={label}>
      {SUBTITLE_COLOR_OPTIONS.map((option) => {
        const selected = hex === SUBTITLE_COLORS[option.value];
        return (
          <button
            key={option.value}
            type="button"
            className={`subtitle-color-swatch${selected ? " is-selected" : ""}`}
            aria-label={option.label}
            aria-pressed={selected}
            title={option.label}
            onClick={() => onChange(option.value)}
          >
            <span style={{ backgroundColor: SUBTITLE_COLORS[option.value] }} />
          </button>
        );
      })}
      <label
        className={`subtitle-color-swatch subtitle-color-picker${isCustom ? " is-selected" : ""}`}
        title={I18N.settings.customSubtitleColor}
      >
        <input
          type="color"
          value={hex}
          aria-label={I18N.settings.customSubtitleColor}
          onChange={(event) => {
            const color = normalizeSubtitleHex(event.currentTarget.value);
            if (color) onChange(color);
          }}
        />
        <span aria-hidden="true" style={isCustom ? { backgroundColor: hex } : undefined}>
          {!isCustom && <Icon name="plus" />}
        </span>
      </label>
    </div>
  );
}
