use super::{
    choose_work_area_for_frame, clamp_frame_origin_to_work_area,
    default_overlay_origin_in_work_area, fit_frame_to_work_area, map_frame_between_work_areas,
    LogicalWorkArea,
};
use crate::windows::test_support::{frame, WORK_AREA};

#[test]
fn mixed_dpi_work_areas_keep_distinct_physical_desktop_origins() {
    let primary = LogicalWorkArea {
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1040.0,
        coordinate_scale: 1.0,
    };
    let right = LogicalWorkArea {
        x: 1920.0,
        y: 0.0,
        width: 2560.0 / 1.5,
        height: 1400.0 / 1.5,
        coordinate_scale: 1.5,
    };
    let on_right = frame(2070.0, 150.0, 640.0, 136.0);

    assert_eq!(
        choose_work_area_for_frame(&on_right, [primary, right], Some(primary)),
        Some(right)
    );
    assert_eq!(
        default_overlay_origin_in_work_area(640.0, 136.0, right),
        (2720.0, 1088.0)
    );
    let straddling_from_primary = frame(1500.0, 150.0, 640.0, 136.0);
    assert_eq!(
        choose_work_area_for_frame(&straddling_from_primary, [primary, right], Some(primary)),
        Some(primary)
    );
}

#[test]
fn saved_frame_prefers_the_secondary_work_area_it_occupies() {
    let primary = LogicalWorkArea {
        x: 0.0,
        y: 24.0,
        width: 1512.0,
        height: 958.0,
        coordinate_scale: 1.0,
    };
    let left = LogicalWorkArea {
        x: -1728.0,
        y: -120.0,
        width: 1728.0,
        height: 1080.0,
        coordinate_scale: 1.0,
    };
    let saved = frame(-1400.0, 680.0, 640.0, 136.0);

    assert_eq!(
        choose_work_area_for_frame(&saved, [primary, left], Some(primary)),
        Some(left)
    );
}

#[test]
fn active_space_mapping_preserves_center_and_bottom_inset() {
    let target = LogicalWorkArea {
        x: -1728.0,
        y: -120.0,
        width: 1728.0,
        height: 1080.0,
        coordinate_scale: 1.0,
    };
    let saved = frame(436.0, 774.0, 640.0, 136.0);

    let mapped = map_frame_between_work_areas(saved, WORK_AREA, target, 136.0);

    assert_eq!(mapped, frame(-1184.0, 752.0, 640.0, 136.0));
}

#[test]
fn active_space_mapping_fits_a_smaller_target_completely() {
    let target = LogicalWorkArea {
        x: 1600.0,
        y: 100.0,
        width: 520.0,
        height: 200.0,
        coordinate_scale: 1.0,
    };
    let saved = frame(400.0, 400.0, 640.0, 300.0);

    let mapped = map_frame_between_work_areas(saved, WORK_AREA, target, 136.0);

    assert_eq!(mapped, frame(1600.0, 100.0, 520.0, 200.0));
}

#[test]
fn same_screen_mapping_still_fits_a_stale_offscreen_frame() {
    let stale = frame(1400.0, 900.0, 900.0, 400.0);

    let mapped = map_frame_between_work_areas(stale, WORK_AREA, WORK_AREA, 136.0);

    assert_eq!(mapped, frame(612.0, 582.0, 900.0, 400.0));
}

#[test]
fn frame_clamp_honors_negative_work_area_origin() {
    let left = LogicalWorkArea {
        x: -1728.0,
        y: -120.0,
        width: 1728.0,
        height: 1080.0,
        coordinate_scale: 1.0,
    };
    let mut saved = frame(-2200.0, -300.0, 640.0, 136.0);

    clamp_frame_origin_to_work_area(&mut saved, left);

    assert_eq!(saved.x, -1728.0);
    assert_eq!(saved.y, -120.0);
}

#[test]
fn settled_frame_returns_fully_inside_the_right_edge() {
    let mut moved = frame(1_200.0, 400.0, 640.0, 136.0);

    assert!(fit_frame_to_work_area(&mut moved, WORK_AREA, 136.0));

    assert_eq!(moved, frame(872.0, 400.0, 640.0, 136.0));
}

#[test]
fn settled_oversized_frame_shrinks_before_origin_is_clamped() {
    let narrow = LogicalWorkArea {
        x: 100.0,
        y: 50.0,
        width: 520.0,
        height: 320.0,
        coordinate_scale: 1.0,
    };
    let mut oversized = frame(480.0, 260.0, 1_200.0, 600.0);

    assert!(fit_frame_to_work_area(&mut oversized, narrow, 136.0));

    assert_eq!(oversized, frame(100.0, 50.0, 520.0, 320.0));
}

#[test]
fn settled_frame_respects_a_negative_origin_secondary_display() {
    let left = LogicalWorkArea {
        x: -1728.0,
        y: -120.0,
        width: 1728.0,
        height: 1080.0,
        coordinate_scale: 1.0,
    };
    let mut moved = frame(-400.0, 900.0, 640.0, 136.0);

    assert!(fit_frame_to_work_area(&mut moved, left, 136.0));

    assert_eq!(moved, frame(-640.0, 824.0, 640.0, 136.0));
}

#[test]
fn settled_frame_that_is_already_visible_stays_exactly_unchanged() {
    let mut visible = frame(400.0, 300.0, 640.0, 136.0);

    assert!(!fit_frame_to_work_area(&mut visible, WORK_AREA, 136.0));

    assert_eq!(visible, frame(400.0, 300.0, 640.0, 136.0));
}

#[test]
fn missing_saved_monitor_falls_back_to_primary_work_area() {
    let primary = LogicalWorkArea {
        x: 0.0,
        y: 24.0,
        width: 1512.0,
        height: 958.0,
        coordinate_scale: 1.0,
    };
    let saved = frame(-2200.0, 700.0, 640.0, 136.0);

    assert_eq!(
        choose_work_area_for_frame(&saved, [primary], Some(primary)),
        Some(primary)
    );
}

#[test]
fn default_origin_uses_work_area_origin_and_bottom_inset() {
    let upper = LogicalWorkArea {
        x: -1280.0,
        y: -1024.0,
        width: 1280.0,
        height: 984.0,
        coordinate_scale: 1.0,
    };

    assert_eq!(
        default_overlay_origin_in_work_area(640.0, 136.0, upper),
        (-960.0, -248.0)
    );
}
