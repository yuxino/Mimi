//! Minimum expanded subtitle geometry, independent of native windows.
use crate::core::audio_input::AudioInput;
use crate::core::models::{SubtitleDisplayMode, TargetLanguage};

/// Retain the saved-frame compatibility floor; current requirements add chrome.
pub const BASE_MINIMUM_HEIGHT: f64 = 136.0;
const SERVICE_ROW_HEIGHT: f64 = 20.0;
/// Active band 61 + service row 20 + inset 12 + inner padding 10 + border 2.
pub const MAXIMUM_CHROME_HEIGHT: f64 = 85.0 + SERVICE_ROW_HEIGHT;
pub const FONT_SIZE_RANGE: std::ops::RangeInclusive<f64> = 14.0..=48.0;
pub const DEFAULT_FONT_SIZE: f64 = 16.0;
const LINE_HEIGHT: f64 = 1.32;
const SOURCE_SCALE: f64 = 0.9;
/// One source/timestamp metadata line (18px) plus the gap to its subtitle (4px).
const METADATA_ROW_HEIGHT: f64 = 22.0;
/// One optional divider between two live sources: 7px margins and a 1px rule.
const SOURCE_DIVIDER_HEIGHT: f64 = 15.0;

/// Desktop typography and persisted preferences must accept the same range.
pub fn normalize_font_size(font_size: f64) -> f64 {
    if font_size.is_finite() {
        font_size.clamp(*FONT_SIZE_RANGE.start(), *FONT_SIZE_RANGE.end())
    } else {
        DEFAULT_FONT_SIZE
    }
}

pub fn minimum_overlay_height(
    audio_input: AudioInput,
    display_mode: SubtitleDisplayMode,
    target_language: TargetLanguage,
    font_size: f64,
    show_subtitle_timestamps: bool,
) -> f64 {
    let font_size = normalize_font_size(font_size);
    let source_count = audio_input.sources().len();
    // Reserve the narrow service row at every width: native resize gestures
    // must never shrink the subtitle body before updating a width-based limit.
    let minimum_height = BASE_MINIMUM_HEIGHT
        + SERVICE_ROW_HEIGHT
        + if source_count == 1 && show_subtitle_timestamps {
            METADATA_ROW_HEIGHT
        } else {
            0.0
        };
    // Preserve the chosen font even in a compact window. Each visible source
    // needs a complete line in each language, a 2px lane gap and 5px padding.
    // Microphone identity can occupy its own metadata row, including when
    // timestamps are hidden. Reserve it without depending on the inline-icon
    // threshold, which changes with the actual reading viewport.
    let translation_line = (font_size * LINE_HEIGHT).ceil();
    let row_height = if display_mode == SubtitleDisplayMode::Bilingual
        && target_language != TargetLanguage::Original
    {
        let source_line = ((font_size * SOURCE_SCALE).max(12.0) * LINE_HEIGHT).ceil();
        source_line + translation_line + 2.0 + 5.0
    } else {
        translation_line + 5.0
    };
    let metadata_height = if audio_input != AudioInput::System || show_subtitle_timestamps {
        METADATA_ROW_HEIGHT
    } else {
        0.0
    };
    // Keep the minimum stable when the user toggles the optional divider.
    let divider_height = (source_count - 1) as f64 * SOURCE_DIVIDER_HEIGHT;
    let required = MAXIMUM_CHROME_HEIGHT
        + source_count as f64 * (row_height + metadata_height)
        + divider_height;
    (minimum_height.max(required) / 4.0).ceil() * 4.0
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
    fn default_system_source_font_keeps_existing_compact_height() {
        for mode in [
            SubtitleDisplayMode::Original,
            SubtitleDisplayMode::Translation,
            SubtitleDisplayMode::Bilingual,
        ] {
            for font in 14..=16 {
                assert_eq!(
                    minimum_overlay_height(
                        AudioInput::System,
                        mode,
                        TargetLanguage::English,
                        font as f64,
                        false,
                    ),
                    156.0
                );
                assert_eq!(
                    minimum_overlay_height(
                        AudioInput::System,
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

    #[test]
    fn microphone_identity_has_room_even_without_timestamps() {
        for (font, height) in [(16.0, 176.0), (24.0, 196.0), (48.0, 256.0)] {
            assert_eq!(
                minimum_overlay_height(
                    AudioInput::Microphone,
                    SubtitleDisplayMode::Bilingual,
                    TargetLanguage::English,
                    font,
                    false,
                ),
                height
            );
        }
    }

    #[test]
    fn maximum_font_has_room_for_both_languages_and_each_sources_metadata() {
        let minimum = minimum_overlay_height(
            AudioInput::Both,
            SubtitleDisplayMode::Bilingual,
            TargetLanguage::English,
            48.0,
            false,
        );
        assert_eq!(minimum, 424.0);
        assert!(
            minimum - MAXIMUM_CHROME_HEIGHT
                >= 2.0 * (METADATA_ROW_HEIGHT + 58.0 + 64.0 + 2.0 + 5.0) + 15.0
        );
        assert_eq!(
            minimum,
            minimum_overlay_height(
                AudioInput::Both,
                SubtitleDisplayMode::Bilingual,
                TargetLanguage::English,
                48.0,
                true,
            )
        );
        let single_minimum = minimum_overlay_height(
            AudioInput::System,
            SubtitleDisplayMode::Bilingual,
            TargetLanguage::English,
            48.0,
            false,
        );
        assert_eq!(single_minimum, 236.0);
        assert!(single_minimum - MAXIMUM_CHROME_HEIGHT >= 58.0 + 64.0 + 2.0 + 5.0);
        assert_eq!(
            minimum_overlay_height(
                AudioInput::Microphone,
                SubtitleDisplayMode::Bilingual,
                TargetLanguage::English,
                48.0,
                true,
            ),
            256.0
        );
    }

    #[test]
    fn every_supported_font_reserves_complete_lines_without_shrinking_the_source() {
        for font in 14..=48 {
            let font = font as f64;
            let bilingual_row =
                (font * SOURCE_SCALE * LINE_HEIGHT).ceil() + (font * LINE_HEIGHT).ceil() + 7.0;
            for input in [AudioInput::System, AudioInput::Microphone, AudioInput::Both] {
                for timestamps in [false, true] {
                    let sources = input.sources().len();
                    let metadata = if input != AudioInput::System || timestamps {
                        22.0
                    } else {
                        0.0
                    };
                    let minimum = minimum_overlay_height(
                        input,
                        SubtitleDisplayMode::Bilingual,
                        TargetLanguage::English,
                        font,
                        timestamps,
                    );
                    assert!(
                        minimum - MAXIMUM_CHROME_HEIGHT
                            >= sources as f64 * (bilingual_row + metadata)
                                + (sources - 1) as f64 * 15.0,
                        "font {font}, input {input:?}, timestamps {timestamps}"
                    );
                }
            }
        }
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
        assert_eq!(minimum(100.0), minimum(48.0));
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(minimum(invalid), minimum(DEFAULT_FONT_SIZE));
        }
    }
}
