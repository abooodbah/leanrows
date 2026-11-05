//! Zoom levels for the row grid, shared with `LeanMark`.

#[cfg(test)]
mod tests {
    use super::{DEFAULT_ZOOM, ZOOM_LEVELS, normalize_zoom, scale_by_zoom, step_zoom};

    #[test]
    fn levels_are_leanmarks() {
        // LeanMark's kZoomLevels, as whole percentages.
        assert_eq!(
            ZOOM_LEVELS,
            [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300]
        );
        assert_eq!(DEFAULT_ZOOM, 100);
    }

    #[test]
    fn steps_move_one_level_and_stay_at_the_ends() {
        assert_eq!(step_zoom(100, 1), 110);
        assert_eq!(step_zoom(100, -1), 90);
        assert_eq!(step_zoom(125, 1), 150);
        assert_eq!(step_zoom(67, -1), 50);
        assert_eq!(step_zoom(300, 1), 300);
        assert_eq!(step_zoom(50, -1), 50);
        assert_eq!(step_zoom(175, 0), 100);
        // A level between two steps moves to the next one in that direction.
        assert_eq!(step_zoom(105, 1), 110);
        assert_eq!(step_zoom(105, -1), 100);
    }

    #[test]
    fn saved_values_snap_to_a_level_or_fall_back() {
        assert_eq!(normalize_zoom(110), 110);
        assert_eq!(normalize_zoom(112), 110);
        assert_eq!(normalize_zoom(118), 125);
        assert_eq!(normalize_zoom(49), 50);
        assert_eq!(normalize_zoom(301), 300);
        assert_eq!(normalize_zoom(48), DEFAULT_ZOOM);
        assert_eq!(normalize_zoom(303), DEFAULT_ZOOM);
        assert_eq!(normalize_zoom(0), DEFAULT_ZOOM);
        assert_eq!(normalize_zoom(u32::MAX), DEFAULT_ZOOM);
    }

    #[test]
    fn sizes_scale_with_rounding_and_keep_their_sign() {
        assert_eq!(scale_by_zoom(-12, 100), -12);
        assert_eq!(scale_by_zoom(-12, 110), -13);
        assert_eq!(scale_by_zoom(-12, 125), -15);
        assert_eq!(scale_by_zoom(-12, 50), -6);
        assert_eq!(scale_by_zoom(112, 150), 168);
        assert_eq!(scale_by_zoom(1, 50), 1);
        assert_eq!(scale_by_zoom(0, 200), 0);
        assert_eq!(scale_by_zoom(i32::MAX, 300), i32::MAX);
    }
}
