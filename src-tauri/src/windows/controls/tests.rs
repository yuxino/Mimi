use super::{
    control_mode_for_presentation, embedded_control_geometry, measured_island_width,
    overlay_control_geometry, unscaled_webview_zoom, windows_control_focus_policy,
    windows_webview_zoom, OverlayControlMode, OverlayControlState, OverlayControlWindowManager,
    WindowsControlFocusPolicy,
};
use crate::windows::test_support::WORK_AREA;
use crate::windows::work_area::LogicalWorkArea;

#[test]
fn embedded_controls_fit_the_canvas_and_hidden_controls_have_no_input() {
    assert_eq!(
        embedded_control_geometry(OverlayControlMode::Hidden, 200.0, 270.0, 640, 482),
        None
    );
    let island =
        embedded_control_geometry(OverlayControlMode::Island, 180.0, 270.0, 640, 482).unwrap();
    assert_eq!(
        (island.x, island.y, island.width, island.height),
        (18.0, 16.0, 180.0, 30.0)
    );
    let panel =
        embedded_control_geometry(OverlayControlMode::Panel, 180.0, 520.0, 256, 136).unwrap();
    assert_eq!(
        (panel.x, panel.y, panel.width, panel.height),
        (18.0, 16.0, 220.0, 112.0)
    );
    assert!(panel.x + panel.width <= 256.0);
    assert!(panel.y + panel.height <= 136.0);
    assert_eq!(
        embedded_control_geometry(OverlayControlMode::Island, 200.0, 270.0, 0, 0),
        None
    );
}

#[test]
fn active_expanded_overlay_keeps_an_open_panel() {
    assert_eq!(
        control_mode_for_presentation(OverlayControlMode::Panel, true, false, false, false),
        OverlayControlMode::Panel
    );
    assert_eq!(
        control_mode_for_presentation(OverlayControlMode::Hidden, true, false, false, false),
        OverlayControlMode::Island
    );
}

#[test]
fn inactive_or_collapsed_overlay_hides_control_surface() {
    assert_eq!(
        control_mode_for_presentation(OverlayControlMode::Panel, false, false, false, false),
        OverlayControlMode::Hidden
    );
    assert_eq!(
        control_mode_for_presentation(OverlayControlMode::Panel, true, true, false, false),
        OverlayControlMode::Hidden
    );
    assert_eq!(
        control_mode_for_presentation(OverlayControlMode::Panel, true, false, true, true),
        OverlayControlMode::Hidden
    );
}

#[test]
fn locking_returns_to_the_island_and_unlocking_keeps_it_compact() {
    for current in [
        OverlayControlMode::Hidden,
        OverlayControlMode::Island,
        OverlayControlMode::Panel,
    ] {
        let locked = control_mode_for_presentation(current, true, false, true, false);
        assert_eq!(locked, OverlayControlMode::Island);
        assert_eq!(
            control_mode_for_presentation(locked, true, false, false, false),
            OverlayControlMode::Island,
        );
    }
}

#[test]
fn locked_recovery_island_stays_hidden_for_inactive_collapsed_or_immersive_overlays() {
    for (active, collapsed, immersive) in [
        (false, false, false),
        (true, true, false),
        (true, false, true),
    ] {
        assert_eq!(
            control_mode_for_presentation(
                OverlayControlMode::Island,
                active,
                collapsed,
                true,
                immersive,
            ),
            OverlayControlMode::Hidden,
        );
    }
}

#[test]
fn windows_control_focus_policy_never_activates_passive_surfaces() {
    assert_eq!(
        windows_control_focus_policy(OverlayControlMode::Island, false),
        WindowsControlFocusPolicy {
            focusable_before_show: false,
            focusable_after_show: false,
            focus: false,
        }
    );
    assert_eq!(
        windows_control_focus_policy(OverlayControlMode::Panel, false),
        WindowsControlFocusPolicy {
            focusable_before_show: false,
            focusable_after_show: true,
            focus: false,
        }
    );
    assert_eq!(
        windows_control_focus_policy(OverlayControlMode::Panel, true),
        WindowsControlFocusPolicy {
            focusable_before_show: true,
            focusable_after_show: true,
            focus: true,
        }
    );
}

#[test]
fn panel_grows_upward_when_it_would_overflow_bottom() {
    let geometry = overlay_control_geometry(
        OverlayControlMode::Panel,
        400.0,
        900.0,
        180.0,
        356.0,
        WORK_AREA,
    );
    assert_eq!(geometry.y, 900.0 + 30.0 - 356.0);
    assert_eq!(geometry.width, 280.0);
    assert_eq!(geometry.height, 356.0);
}

#[test]
fn panel_keeps_its_anchor_when_there_is_room_below() {
    let geometry = overlay_control_geometry(
        OverlayControlMode::Panel,
        400.0,
        300.0,
        180.0,
        356.0,
        WORK_AREA,
    );
    assert_eq!(geometry.y, 300.0);
}

#[test]
fn control_geometry_honors_negative_monitor_origins() {
    let work_area = LogicalWorkArea {
        x: -1728.0,
        y: -120.0,
        width: 1728.0,
        height: 1080.0,
        coordinate_scale: 1.0,
    };
    let geometry = overlay_control_geometry(
        OverlayControlMode::Island,
        -1800.0,
        -200.0,
        180.0,
        356.0,
        work_area,
    );
    assert_eq!(geometry.x, -1720.0);
    assert_eq!(geometry.y, -112.0);
}

#[test]
fn panel_clamps_inside_right_edge_and_short_work_area() {
    let work_area = LogicalWorkArea {
        x: 100.0,
        y: 50.0,
        width: 320.0,
        height: 180.0,
        coordinate_scale: 1.0,
    };
    let geometry = overlay_control_geometry(
        OverlayControlMode::Panel,
        400.0,
        160.0,
        180.0,
        520.0,
        work_area,
    );
    assert_eq!(geometry.x, 132.0);
    assert_eq!(geometry.width, 280.0);
    assert_eq!(geometry.y, 58.0);
    assert_eq!(geometry.height, 164.0);
}

#[test]
fn island_hugs_measured_content_without_changing_the_panel_width_or_left_anchor() {
    for width in [148.0, 188.0, 312.0] {
        let island = overlay_control_geometry(
            OverlayControlMode::Island,
            400.0,
            300.0,
            width,
            356.0,
            WORK_AREA,
        );
        let panel = overlay_control_geometry(
            OverlayControlMode::Panel,
            400.0,
            300.0,
            width,
            356.0,
            WORK_AREA,
        );
        assert_eq!((island.x, island.y, island.width), (400.0, 300.0, width));
        assert_eq!((panel.x, panel.y, panel.width), (400.0, 300.0, 280.0));
    }
}

#[test]
fn measured_width_updates_keep_hidden_and_panel_modes_and_dismiss_generation() {
    let state = OverlayControlState::default();
    assert!(state.set_island_width(148.0, 1.0));
    assert_eq!(state.snapshot(), (OverlayControlMode::Hidden, 270.0, 0));
    assert_eq!(state.dimensions(), (148.0, 270.0, 1.0));
    assert!(state.set_mode(OverlayControlMode::Panel));
    assert!(state.set_panel_height(356.0, 1.0));
    assert!(state.set_island_width(312.0, 1.0));
    assert!(!state.set_island_width(312.0, 1.0));
    assert_eq!(state.snapshot(), (OverlayControlMode::Panel, 356.0, 1));
    assert_eq!(state.dimensions(), (312.0, 356.0, 1.0));
    assert!(state.set_island_width(312.0, 1.2));
    assert_eq!(state.dimensions(), (312.0, 356.0, 1.2));
}

#[test]
fn windows_panel_drag_owns_focus_handoff_until_native_completion() {
    let state = OverlayControlState::default();
    assert_eq!(state.begin_panel_drag(), None);
    state.set_mode(OverlayControlMode::Panel);
    let before = state.snapshot().2;
    let drag = state.begin_panel_drag().unwrap();
    assert!(state.panel_drag_active());
    assert_ne!(state.snapshot().2, before);
    assert_eq!(state.begin_panel_drag(), None);
    // A native focus return can cancel an older dismissal without losing the
    // gesture that still owns its eventual completion.
    state.cancel_scheduled_dismiss();
    assert!(state.has_panel_drag(drag));
    assert!(state.finish_panel_drag(drag));
    assert!(!state.panel_drag_active());
    assert_eq!(state.mode(), OverlayControlMode::Panel);
    assert!(!state.finish_panel_drag(drag));
}

#[test]
fn closing_hiding_or_reopening_controls_retires_a_windows_drag_completion() {
    for next in [OverlayControlMode::Island, OverlayControlMode::Hidden] {
        let state = OverlayControlState::default();
        state.set_mode(OverlayControlMode::Panel);
        let old_drag = state.begin_panel_drag().unwrap();
        state.set_mode(next);
        assert!(!state.panel_drag_active());
        assert!(!state.has_panel_drag(old_drag));
        assert!(!state.finish_panel_drag(old_drag));
        state.set_mode(OverlayControlMode::Panel);
        let current_drag = state.begin_panel_drag().unwrap();
        assert_ne!(current_drag, old_drag);
        assert!(!state.finish_panel_drag(old_drag));
        assert!(state.has_panel_drag(current_drag));
        assert!(state.finish_panel_drag(current_drag));
    }
}

#[test]
fn delayed_dismiss_cannot_replace_a_new_drag_or_hidden_panel() {
    let state = OverlayControlState::default();
    assert_eq!(state.dismissal_generation(), None);
    state.set_mode(OverlayControlMode::Panel);
    let scheduled = state.dismissal_generation().unwrap();
    let drag = state.begin_panel_drag().unwrap();
    assert_eq!(state.dismissal_generation(), None);
    assert!(!state.dismiss_if_current(scheduled));
    assert!(state.has_panel_drag(drag));
    state.finish_panel_drag(drag);

    let scheduled = state.dismissal_generation().unwrap();
    state.set_mode(OverlayControlMode::Hidden);
    assert!(!state.dismiss_if_current(scheduled));
    assert!(!state.dismiss_panel());
    assert_eq!(state.mode(), OverlayControlMode::Hidden);
    state.set_mode(OverlayControlMode::Panel);
    assert!(!state.dismiss_if_current(scheduled));
    let current = state.dismissal_generation().unwrap();
    assert!(state.dismiss_if_current(current));
    assert_eq!(state.mode(), OverlayControlMode::Island);
}

#[test]
fn explicit_panel_dismiss_retires_a_drag_and_late_completion() {
    let state = OverlayControlState::default();
    state.set_mode(OverlayControlMode::Panel);
    let drag = state.begin_panel_drag().unwrap();
    assert!(state.dismiss_panel());
    assert!(!state.finish_panel_drag(drag));
    assert_eq!(state.mode(), OverlayControlMode::Island);
}

#[test]
fn island_measurements_are_finite_rounded_and_bounded() {
    assert_eq!(measured_island_width(143.2), Some(144.0));
    assert_eq!(measured_island_width(2.0), Some(80.0));
    assert_eq!(measured_island_width(10000.0), Some(156.0));
    assert_eq!(measured_island_width(f64::NAN), None);
    assert_eq!(measured_island_width(f64::INFINITY), None);
}

#[test]
fn windows_text_zoom_is_independent_of_monitor_dpi_and_rejects_invalid_scales() {
    for scale in [1.0, 1.25, 1.5, 2.0] {
        assert_eq!(windows_webview_zoom(scale, scale), Some(1.0));
        assert!((windows_webview_zoom(scale * 1.2, scale).unwrap() - 1.2).abs() < 0.0001);
    }
    for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY, 17.0] {
        assert_eq!(windows_webview_zoom(invalid, 1.0), None);
        assert_eq!(windows_webview_zoom(1.0, invalid), None);
    }
}

#[test]
fn non_windows_dimensions_ignore_pixel_ratio_including_invalid_values() {
    let state = OverlayControlState::default();
    for pixel_ratio in [0.0, -1.0, f64::NAN, f64::INFINITY, 1.5, 17.0] {
        let zoom = unscaled_webview_zoom(pixel_ratio);
        state.set_panel_height(356.0, zoom);
        state.set_island_width(148.0, zoom);
        assert_eq!(state.dimensions(), (148.0, 356.0, 1.0));
    }
}

#[test]
fn unchanged_panel_height_still_reapplies_changed_windows_text_zoom() {
    let state = OverlayControlState::default();
    assert!(state.set_panel_height(356.0, 1.0));
    assert!(!state.set_panel_height(356.0, 1.0));
    assert!(state.set_panel_height(356.0, 1.2));
    assert_eq!(
        state.dimensions(),
        (
            OverlayControlWindowManager::DEFAULT_ISLAND_WIDTH,
            356.0,
            1.2
        )
    );
}

#[test]
fn island_width_clamps_to_short_screens_without_using_panel_width() {
    let work_area = LogicalWorkArea {
        x: 100.0,
        y: 50.0,
        width: 320.0,
        height: 180.0,
        coordinate_scale: 1.0,
    };
    let island = overlay_control_geometry(
        OverlayControlMode::Island,
        400.0,
        160.0,
        180.0,
        520.0,
        work_area,
    );
    assert_eq!((island.x, island.y, island.width), (232.0, 160.0, 180.0));
    let narrow = LogicalWorkArea {
        width: 100.0,
        ..work_area
    };
    let island = overlay_control_geometry(
        OverlayControlMode::Island,
        400.0,
        160.0,
        312.0,
        520.0,
        narrow,
    );
    assert_eq!((island.x, island.y, island.width), (108.0, 160.0, 84.0));
}

#[test]
fn zoomed_controls_fit_css_content_without_changing_monitor_bounds() {
    for monitor_scale in [1.0, 1.5, 2.0] {
        let monitor = LogicalWorkArea {
            x: -1920.0,
            y: 80.0,
            width: 1280.0,
            height: 720.0,
            coordinate_scale: monitor_scale,
        };
        for zoom in [1.0, 1.2, 1.5] {
            let css_area = monitor.with_webview_zoom(zoom);
            assert!((css_area.coordinate_width() - monitor.coordinate_width()).abs() < 0.001);
            assert!((css_area.coordinate_height() - monitor.coordinate_height()).abs() < 0.001);
            for mode in [OverlayControlMode::Island, OverlayControlMode::Panel] {
                let geometry = overlay_control_geometry(
                    mode,
                    monitor.x + 100.0,
                    monitor.y + 100.0,
                    148.0,
                    356.0,
                    css_area,
                );
                let (css_width, css_height) = match mode {
                    OverlayControlMode::Island => (148.0, 30.0),
                    _ => (280.0, 356.0),
                };
                assert!(
                    (geometry.width * css_area.coordinate_scale - css_width * monitor_scale * zoom)
                        .abs()
                        < 0.001
                );
                assert!(
                    (geometry.height * css_area.coordinate_scale
                        - css_height * monitor_scale * zoom)
                        .abs()
                        < 0.001
                );
                assert!(geometry.x >= monitor.x);
                assert!(geometry.y >= monitor.y);
                assert!(
                    geometry.x + geometry.width * css_area.coordinate_scale
                        <= monitor.x + monitor.coordinate_width()
                );
                assert!(
                    geometry.y + geometry.height * css_area.coordinate_scale
                        <= monitor.y + monitor.coordinate_height()
                );
            }
        }
    }
}
