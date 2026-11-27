import { useState } from "react";
import { Icon } from "../../../components/Icon";
import { I18N } from "../../../lib/i18n";
import {
  normalizeSubtitleHex,
  parseSubtitleHexInput,
  SUBTITLE_COLORS,
  SUBTITLE_COLOR_OPTIONS,
  subtitleColorHex,
} from "../../../lib/subtitles/subtitleColor";
import type { SubtitleColor } from "../../../lib/types";

export interface ColorOption<T extends string = string> {
  readonly value: T;
  readonly hex: string;
  readonly label: string;
}

type Props<T extends string = SubtitleColor> = {
  label?: string;
  value: T;
  options?: readonly ColorOption<T>[];
  resolveHex?: (value: T) => string;
  disabled?: boolean;
  onChange: (color: T) => void;
};

const DEFAULT_TEXT_OPTIONS: readonly ColorOption<SubtitleColor>[] = SUBTITLE_COLOR_OPTIONS.map((option) => ({
  value: option.value,
  hex: SUBTITLE_COLORS[option.value],
  get label() { return option.label; },
}));

export function SubtitleColorControl<T extends string = SubtitleColor>({
  value,
  onChange,
  label = I18N.settings.subtitleColor,
  options = DEFAULT_TEXT_OPTIONS as unknown as readonly ColorOption<T>[],
  resolveHex = subtitleColorHex as unknown as (value: T) => string,
  disabled = false,
}: Props<T>) {
  const hex = resolveHex(value);
  const isCustom = !options.some((option) => hex === option.hex);
  const [edit, setEdit] = useState({ baseHex: hex, draft: hex, error: false });
  const hexDraft = !disabled && edit.baseHex === hex ? edit.draft : hex;
  const hexError = !disabled && edit.baseHex === hex && edit.error;

  const commitHex = () => {
    if (disabled) return;
    const parsed = parseSubtitleHexInput(hexDraft);
    if (!parsed) {
      setEdit({ baseHex: hex, draft: hexDraft, error: true });
      return;
    }
    setEdit({ baseHex: parsed, draft: parsed, error: false });
    const matchingPreset = options.find((option) => option.hex === parsed);
    const nextValue = (matchingPreset ? matchingPreset.value : parsed) as T;
    if (resolveHex(nextValue) !== hex || nextValue !== value) {
      onChange(nextValue);
    }
  };

  return (
    <div className={`subtitle-color-control${disabled ? " is-disabled" : ""}`}>
      <div className="subtitle-color-presets" role="group" aria-label={label}>
        {options.map((option) => {
          const selected = hex === option.hex;
          return (
            <button
              key={option.value}
              type="button"
              className={`subtitle-color-swatch${selected ? " is-selected" : ""}`}
              aria-label={option.label}
              aria-pressed={selected}
              title={option.label}
              disabled={disabled}
              onClick={() => {
                setEdit({ baseHex: option.hex, draft: option.hex, error: false });
                onChange(option.value);
              }}
            >
              <span style={{ backgroundColor: option.hex }} />
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
            disabled={disabled}
            aria-label={`${label} · ${I18N.settings.customSubtitleColor}`}
            onChange={(event) => {
              const color = normalizeSubtitleHex(event.currentTarget.value);
              if (!color) return;
              setEdit({ baseHex: color, draft: color, error: false });
              const matchingPreset = options.find((option) => option.hex === color);
              onChange((matchingPreset ? matchingPreset.value : color) as T);
            }}
          />
          <span aria-hidden="true" style={isCustom ? { backgroundColor: hex } : undefined}>
            {!isCustom && <Icon name="plus" />}
          </span>
        </label>
        <input
          type="text"
          className={`subtitle-color-hex${hexError ? " is-invalid" : ""}`}
          value={hexDraft}
          disabled={disabled}
          maxLength={12}
          spellCheck={false}
          autoComplete="off"
          aria-label={`${label} · ${I18N.settings.hexColorInput}`}
          aria-invalid={hexError || undefined}
          placeholder="#000000"
          onChange={(event) => {
            setEdit({ baseHex: hex, draft: event.currentTarget.value, error: false });
          }}
          onBlur={commitHex}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              commitHex();
            } else if (event.key === "Escape") {
              event.preventDefault();
              setEdit({ baseHex: hex, draft: hex, error: false });
            }
          }}
        />
      </div>
      {hexError && (
        <div className="subtitle-color-hex-error" role="alert">
          {I18N.settings.hexColorInvalid}
        </div>
      )}
    </div>
  );
}
