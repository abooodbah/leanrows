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

/// The level after `notches` steps from `current`: positive notches zoom in,
/// negative ones zoom out, and the ends hold. A burst of Ctrl+wheel notches
/// becomes one change to this level instead of one change per notch.
#[must_use]
pub(crate) fn zoom_after_notches(current: u32, notches: i32) -> u32 {
    (0..notches.unsigned_abs()).fold(current, |level, _| step_zoom(level, notches.signum()))
}

/// Adds a wheel `delta` to the movement not yet a whole notch and returns the
/// whole notches, 120 units each. Precision touchpads send smaller steps,
/// which add up.
pub(crate) fn wheel_notches(pending: &mut i32, delta: i32) -> i32 {
    *pending = pending.saturating_add(delta);
    let notches = *pending / 120;
    *pending -= notches * 120;
    notches
}

/// Column widths across zoom changes.
///
/// Each column keeps a basis, a width at a known zoom. A zoom change scales
/// the basis, not the width shown before it, so stepping out and back lands
/// on the same widths, and a column at a level is as wide as a column opened
/// at that level. A width that differs from the one last applied was set by
/// the user, and becomes that column's new basis.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ColumnZoom {
    columns: Vec<ColumnBasis>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ColumnBasis {
    width: i32,
    zoom: u32,
    /// The width last given to the list; -1 when none was.
    applied: i32,
}

impl ColumnBasis {
    const UNKNOWN: Self = Self {
        width: 0,
        zoom: DEFAULT_ZOOM,
        applied: -1,
    };
}

impl ColumnZoom {
    /// Records column `index`, `design` pixels wide at 100%, shown at `zoom`,
    /// and returns its width at that zoom.
    pub(crate) fn insert(&mut self, index: usize, design: i32, zoom: u32) -> i32 {
        let applied = scale_by_zoom(design, zoom).max(1);
        if self.columns.len() <= index {
            self.columns.resize(index + 1, ColumnBasis::UNKNOWN);
        }
        self.columns[index] = ColumnBasis {
            width: design,
            zoom: DEFAULT_ZOOM,
            applied,
        };
        applied
    }

    /// The widths at zoom `to`, given the widths `current` shown at `from`.
    pub(crate) fn rescale(&mut self, current: &[i32], from: u32, to: u32) -> Vec<i32> {
        self.columns.resize(current.len(), ColumnBasis::UNKNOWN);
        current
            .iter()
            .zip(self.columns.iter_mut())
            .map(|(&width, basis)| {
                if basis.applied != width {
                    *basis = ColumnBasis {
                        width,
                        zoom: from.max(1),
                        applied: width,
                    };
                }
                basis.applied = scale_width(basis.width, basis.zoom, to);
                basis.applied
            })
            .collect()
    }
}

/// `width` at zoom `from`, scaled to zoom `to`, rounded to the nearest pixel
/// and at least one pixel.
fn scale_width(width: i32, from: u32, to: u32) -> i32 {
    let from = i64::from(from.max(1));
    let scaled = (i64::from(width) * i64::from(to) + from / 2) / from;
    i32::try_from(scaled).unwrap_or(i32::MAX).max(1)
}

#[cfg(test)]
mod tests {
    use super::{
        ColumnZoom, DEFAULT_ZOOM, ZOOM_LEVELS, normalize_zoom, scale_by_zoom, step_zoom,
        wheel_notches, zoom_after_notches,
    };

    #[test]
    fn wheel_movement_adds_up_to_whole_notches() {
        let mut pending = 0;
        assert_eq!(wheel_notches(&mut pending, 120), 1);
        assert_eq!(pending, 0);
        assert_eq!(wheel_notches(&mut pending, 40), 0);
        assert_eq!(wheel_notches(&mut pending, 40), 0);
        assert_eq!(wheel_notches(&mut pending, 40), 1);
        assert_eq!(pending, 0);
        assert_eq!(wheel_notches(&mut pending, -360), -3);
        assert_eq!(wheel_notches(&mut pending, 250), 2);
        assert_eq!(pending, 10);
        assert_eq!(wheel_notches(&mut pending, -20), 0);
        assert_eq!(pending, -10);
    }

    #[test]
    fn queued_notches_in_both_directions_end_where_single_steps_would() {
        // Up eight from 250% holds at 300%, then down one: 250%, as eight
        // single steps and one more would land.
        let target = zoom_after_notches(zoom_after_notches(250, 8), -1);
        let mut level = 250;
        for direction in [1, 1, 1, 1, 1, 1, 1, 1, -1] {
            level = step_zoom(level, direction);
        }
        assert_eq!(target, level);
        assert_eq!(target, 250);
    }

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

    #[test]
    fn a_burst_of_notches_is_one_change_to_the_final_level() {
        // Eight notches up from 100% is one change to 300%, not eight steps.
        assert_eq!(zoom_after_notches(100, 8), 300);
        assert_eq!(zoom_after_notches(300, -12), 50);
        assert_eq!(zoom_after_notches(125, 1), 150);
        assert_eq!(zoom_after_notches(125, -2), 100);
        assert_eq!(zoom_after_notches(110, 0), 110);
        // The result equals the single steps it replaces.
        let mut level = 67;
        for _ in 0..5 {
            level = step_zoom(level, 1);
        }
        assert_eq!(zoom_after_notches(67, 5), level);
        assert_eq!(zoom_after_notches(50, -40), 50);
    }

    /// Every level a zoom passes through, 100% to 300%, down to 50% and back.
    fn round_trip() -> Vec<u32> {
        let up = ZOOM_LEVELS.iter().copied().filter(|level| *level > 100);
        let down = ZOOM_LEVELS.iter().rev().copied().skip(1);
        let back = ZOOM_LEVELS
            .iter()
            .copied()
            .filter(|level| *level > 50 && *level <= 100);
        up.chain(down).chain(back).collect()
    }

    fn zoomed(columns: &mut ColumnZoom, widths: &[i32], path: &[u32]) -> Vec<i32> {
        let mut widths = widths.to_vec();
        let mut from = DEFAULT_ZOOM;
        for &to in path {
            widths = columns.rescale(&widths, from, to);
            from = to;
        }
        widths
    }

    #[test]
    fn zooming_out_and_back_returns_the_same_widths() {
        // The Row, Preview, field and sheet defaults.
        let defaults = [112, 720, 220, 160];
        let mut columns = ColumnZoom::default();
        let widths: Vec<i32> = defaults
            .iter()
            .enumerate()
            .map(|(index, design)| columns.insert(index, *design, DEFAULT_ZOOM))
            .collect();
        assert_eq!(widths, defaults);
        let path = round_trip();
        assert_eq!(path.first(), Some(&110));
        assert_eq!(path.last(), Some(&100));
        for _ in 0..3 {
            assert_eq!(zoomed(&mut columns, &widths, &path), defaults);
        }
    }

    #[test]
    fn a_zoomed_column_is_as_wide_as_one_opened_at_that_level() {
        for design in [112, 720, 220, 160, 1, 7] {
            let mut stepped = ColumnZoom::default();
            let mut width = stepped.insert(0, design, DEFAULT_ZOOM);
            let mut from = DEFAULT_ZOOM;
            for to in round_trip() {
                width = stepped.rescale(&[width], from, to)[0];
                from = to;
                let mut cold = ColumnZoom::default();
                assert_eq!(width, cold.insert(0, design, to), "{design} px at {to}%");
            }
        }
        // Columns opened at 300% and zoomed to 50% match ones opened at 50%.
        let mut columns = ColumnZoom::default();
        let at_300 = [columns.insert(0, 112, 300), columns.insert(1, 220, 300)];
        assert_eq!(at_300, [336, 660]);
        assert_eq!(columns.rescale(&at_300, 300, 50), [56, 110]);
        assert_eq!([scale_by_zoom(112, 50), scale_by_zoom(220, 50)], [56, 110]);
    }

    #[test]
    fn a_width_the_user_set_becomes_the_new_basis() {
        let mut columns = ColumnZoom::default();
        let widths = [columns.insert(0, 112, 100), columns.insert(1, 220, 100)];
        let at_150 = columns.rescale(&widths, 100, 150);
        assert_eq!(at_150, [168, 330]);
        // The user drags the second column to 301 px at 150%.
        let dragged = [at_150[0], 301];
        let at_200 = columns.rescale(&dragged, 150, 200);
        assert_eq!(at_200, [224, 401]);
        let back = columns.rescale(&at_200, 200, 150);
        assert_eq!(back, [168, 301]);
        // A column the list gained since the last step starts from its width.
        let grown = columns.rescale(&[168, 301, 90], 150, 300);
        assert_eq!(grown, [336, 602, 180]);
    }

    #[test]
    fn a_tab_shown_again_carries_on_from_its_widths() {
        // A tab comes back at 125%: its columns are inserted at that level,
        // then given the widths it had, one of them set by the user.
        let mut shown = ColumnZoom::default();
        assert_eq!(shown.insert(0, 112, 125), 140);
        assert_eq!(shown.insert(1, 220, 125), 275);
        assert_eq!(shown.insert(2, 220, 125), 275);
        let widths = [140, 301, 275];
        assert_eq!(shown.rescale(&widths, 125, 100), [112, 241, 220]);
        assert_eq!(shown.rescale(&[112, 241, 220], 100, 125), [140, 301, 275]);
        // A new document reuses the slots; inserting replaces each basis.
        assert_eq!(shown.insert(1, 720, 125), 900);
        assert_eq!(shown.rescale(&[140, 900], 125, 50), [56, 360]);
    }

    #[test]
    fn an_index_past_the_end_leaves_no_borrowed_basis() {
        // Inserting column 2 first must not give columns 0 and 1 its basis.
        let mut columns = ColumnZoom::default();
        assert_eq!(columns.insert(2, 220, 100), 220);
        assert_eq!(columns.rescale(&[220, 220, 220], 100, 200), [440, 440, 440]);
        let mut columns = ColumnZoom::default();
        columns.insert(2, 220, 100);
        // Column 0 is 112 px; it starts from its own width, not 220.
        assert_eq!(columns.rescale(&[112, 50, 220], 100, 50), [56, 25, 110]);
    }
}
