//! Minimum expanded subtitle geometry, independent of native windows.
use crate::core::audio_input::AudioInput;
use crate::core::models::{SubtitleDisplayMode, TargetLanguage};

/// Retain the saved-frame compatibility floor; current requirements add chrome.
pub const BASE_MINIMUM_HEIGHT: f64 = 136.0;
const SERVICE_ROW_HEIGHT: f64 = 20.0;
/// Active band 61 + service row 20 + inset 12 + inner padding 10 + border 2.
pub const MAXIMUM_CHROME_HEIGHT: f64 = 85.0 + SERVICE_ROW_HEIGHT;
const MIN_FONT_SIZE: f64 = 14.0;
const MAX_FONT_SIZE: f64 = 20.0;
pub const DEFAULT_FONT_SIZE: f64 = 16.0;
const LINE_HEIGHT: f64 = 1.32;
const COMPACT_SOURCE_SCALE: f64 = 0.82;
/// One source/timestamp metadata line (18px) plus the gap to its subtitle (4px).
const METADATA_ROW_HEIGHT: f64 = 22.0;

pub fn minimum_overlay_height(
    audio_input: AudioInput,
    display_mode: SubtitleDisplayMode,
    target_language: TargetLanguage,
    font_size: f64,
    show_subtitle_timestamps: bool,
) -> f64 {
    let font_size = if font_size.is_finite() {
        font_size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
    } else {
        DEFAULT_FONT_SIZE
    };
    // Reserve the narrow service row at every width: native resize gestures
    // must never shrink the subtitle body before updating a width-based limit.
    let minimum_height = BASE_MINIMUM_HEIGHT + SERVICE_ROW_HEIGHT;
    // A single tight block fits even at font 20: source 22 + translation 27
    // + one-pixel gap + one-pixel padding = the 51px body at height 156.
    // Either single input reserves a metadata row when timestamps are enabled.
    if audio_input.sources().len() == 1 {
        return if show_subtitle_timestamps {
            ((minimum_height + METADATA_ROW_HEIGHT) / 4.0).ceil() * 4.0
        } else {
            minimum_height
        };
    }
    // Each source keeps its own metadata row, regardless of timestamps.
    // Reserve a 2px bilingual lane gap and up to 5px block padding.
    let translation_line = (font_size * LINE_HEIGHT).ceil();
    let row_height = if display_mode == SubtitleDisplayMode::Bilingual
        && target_language != TargetLanguage::Original
    {
        let source_line = ((font_size * COMPACT_SOURCE_SCALE).max(12.0) * LINE_HEIGHT).ceil();
        source_line + translation_line + 2.0 + 5.0
    } else {
        translation_line + 5.0
    };
    let required = MAXIMUM_CHROME_HEIGHT
        + audio_input.sources().len() as f64 * (row_height + METADATA_ROW_HEIGHT);
    minimum_height.max((required / 4.0).ceil() * 4.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct LayoutCase {
        audio_input: AudioInput,
        display_mode: SubtitleDisplayMode,
        target_language: TargetLanguage,
        font_size: f64,
        show_subtitle_timestamps: bool,
        minimum_height: f64,
    }

    #[test]
    fn native_and_browser_share_the_layout_contract() {
        let cases: Vec<LayoutCase> = serde_json::from_str(include_str!(
            "../../../shared/overlay-layout-contracts.json"
        ))
        .unwrap();
        for case in cases {
            assert_eq!(
                minimum_overlay_height(
                    case.audio_input,
                    case.display_mode,
                    case.target_language,
                    case.font_size,
                    case.show_subtitle_timestamps,
                ),
                case.minimum_height
            );
        }
    }

    #[test]
    fn all_supported_single_source_fonts_keep_the_fitting_compact_height() {
        for input in [AudioInput::System, AudioInput::Microphone] {
            for mode in [
                SubtitleDisplayMode::Original,
                SubtitleDisplayMode::Translation,
                SubtitleDisplayMode::Bilingual,
            ] {
                for font in 14..=20 {
                    assert_eq!(
                        minimum_overlay_height(
                            input,
                            mode,
                            TargetLanguage::English,
                            font as f64,
                            false,
                        ),
                        156.0
                    );
                    assert_eq!(
                        minimum_overlay_height(
                            input,
                            mode,
                            TargetLanguage::English,
                            font as f64,
                            true,
                        ),
                        180.0
                    );
                }
            }
        }
    }

    #[test]
    fn dual_bilingual_minimum_has_room_for_both_maximum_font_rows() {
        let minimum = minimum_overlay_height(
            AudioInput::Both,
            SubtitleDisplayMode::Bilingual,
            TargetLanguage::English,
            20.0,
            false,
        );
        assert_eq!(minimum, 264.0);
        assert!(
            minimum - MAXIMUM_CHROME_HEIGHT
                >= 2.0 * (METADATA_ROW_HEIGHT + 22.0 + 27.0 + 2.0 + 5.0)
        );
        assert_eq!(
            minimum,
            minimum_overlay_height(
                AudioInput::Both,
                SubtitleDisplayMode::Bilingual,
                TargetLanguage::English,
                20.0,
                true,
            )
        );
        let former_minimum = minimum_overlay_height(
            AudioInput::System,
            SubtitleDisplayMode::Bilingual,
            TargetLanguage::English,
            18.0,
            false,
        );
        assert!(former_minimum - MAXIMUM_CHROME_HEIGHT < 2.0 * (20.0 + 24.0 + 1.0 + 1.0));
    }

    #[test]
    fn unsupported_font_values_cannot_break_the_geometry_bound() {
        let minimum = |font| {
            minimum_overlay_height(
                AudioInput::Both,
                SubtitleDisplayMode::Bilingual,
                TargetLanguage::English,
                font,
                false,
            )
        };
        assert_eq!(minimum(-1.0), minimum(14.0));
        assert_eq!(minimum(100.0), minimum(20.0));
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(minimum(invalid), minimum(DEFAULT_FONT_SIZE));
        }
    }
}
