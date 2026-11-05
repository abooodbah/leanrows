//! Zoom levels for the row grid, shared with `LeanMark`.
//!
//! Only the rows zoom, as in `LeanMark` only the document zooms: the toolbar,
//! tabs and panels keep their size. The levels are the familiar browser
//! steps, so repeated zooming lands on round values.

/// Zoom levels in percent, `LeanMark`'s `kZoomLevels`.
pub(crate) const ZOOM_LEVELS: [u32; 13] =
    [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300];
/// Actual size.
pub(crate) const DEFAULT_ZOOM: u32 = 100;

/// The next level above (`direction > 0`) or below (`direction < 0`) the
/// current zoom, staying at the ends. A zero direction resets to 100%.
#[must_use]
pub(crate) fn step_zoom(current: u32, direction: i32) -> u32 {
    match direction.cmp(&0) {
        std::cmp::Ordering::Greater => ZOOM_LEVELS
            .into_iter()
            .find(|level| *level > current)
            .unwrap_or(ZOOM_LEVELS[ZOOM_LEVELS.len() - 1]),
        std::cmp::Ordering::Less => ZOOM_LEVELS
            .into_iter()
            .rev()
            .find(|level| *level < current)
            .unwrap_or(ZOOM_LEVELS[0]),
        std::cmp::Ordering::Equal => DEFAULT_ZOOM,
    }
}

/// Snaps a saved zoom to the nearest level. Values more than 1% outside the
/// range give actual size, as `LeanMark`'s `NormalizeZoom`.
#[must_use]
pub(crate) fn normalize_zoom(value: u32) -> u32 {
    let lowest = ZOOM_LEVELS[0];
    let highest = ZOOM_LEVELS[ZOOM_LEVELS.len() - 1];
    if value < lowest.saturating_sub(1) || value > highest.saturating_add(1) {
        return DEFAULT_ZOOM;
    }
    ZOOM_LEVELS
        .into_iter()
        .min_by_key(|level| level.abs_diff(value))
        .unwrap_or(DEFAULT_ZOOM)
}

/// Scales a pixel size, such as a font height or a column width, by `zoom`
/// percent, rounding to the nearest pixel. A nonzero size stays nonzero.
#[must_use]
pub(crate) fn scale_by_zoom(size: i32, zoom: u32) -> i32 {
    let scaled = (i64::from(size) * i64::from(zoom) + i64::from(size.signum()) * 50) / 100;
    let scaled = if scaled == 0 {
        i64::from(size.signum())
    } else {
        scaled
    };
    i32::try_from(scaled).unwrap_or(if size < 0 { i32::MIN } else { i32::MAX })
}

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
        // Half a pixel rounds away from zero.
        assert_eq!(scale_by_zoom(15, 110), 17);
        assert_eq!(scale_by_zoom(-15, 110), -17);
        assert_eq!(scale_by_zoom(1, 50), 1);
        assert_eq!(scale_by_zoom(0, 200), 0);
        assert_eq!(scale_by_zoom(i32::MAX, 300), i32::MAX);
    }
}
