//! Geometry for the thin scroll bars, kept apart from Win32 so it can be
//! tested.
//!
//! A bar mirrors the list's own scroll range: `min..=max` in the list's
//! units (rows down, pixels across), `page` units visible, the first one at
//! `pos`. The thumb's length is the visible share of the track, never shorter
//! than a minimum the finger and the mouse can catch.

/// The list's scroll state for one direction, as `GetScrollInfo` reports it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ScrollRange {
    pub(crate) min: i32,
    pub(crate) max: i32,
    pub(crate) page: u32,
    pub(crate) pos: i32,
}

impl ScrollRange {
    /// Units in the whole range.
    fn span(self) -> i64 {
        (i64::from(self.max) - i64::from(self.min) + 1).max(0)
    }

    /// The farthest `pos` can go: the last page starts here.
    fn last_pos(self) -> i64 {
        (i64::from(self.max) - i64::from(self.page) + 1).max(i64::from(self.min))
    }

    /// True when there is more than one page to scroll through.
    #[must_use]
    pub(crate) fn scrolls(self) -> bool {
        self.page > 0 && self.span() > i64::from(self.page)
    }
}

/// Where the thumb lies along a track `track` pixels long.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Thumb {
    pub(crate) start: i32,
    pub(crate) length: i32,
}

impl Thumb {
    #[must_use]
    pub(crate) fn end(self) -> i32 {
        self.start.saturating_add(self.length)
    }
}

/// The thumb for `range` on a track `track` pixels long, at least
/// `min_length` pixels, or `None` when the list does not scroll.
#[must_use]
pub(crate) fn thumb(range: ScrollRange, track: i32, min_length: i32) -> Option<Thumb> {
    if !range.scrolls() || track <= 0 {
        return None;
    }
    let span = range.span();
    let track_i64 = i64::from(track);
    let length = (track_i64 * i64::from(range.page) / span)
        .max(i64::from(min_length.min(track)))
        .min(track_i64);
    let travel = track_i64 - length;
    let steps = range.last_pos() - i64::from(range.min);
    let offset = (i64::from(range.pos) - i64::from(range.min)).clamp(0, steps);
    let start = if steps == 0 {
        0
    } else {
        (travel * offset + steps / 2) / steps
    };
    Some(Thumb {
        start: i32::try_from(start).unwrap_or(0),
        length: i32::try_from(length).unwrap_or(track),
    })
}

/// The scroll position that puts the thumb's start at `start` pixels, for a
/// thumb `length` pixels long: the inverse of [`thumb`], used while the
/// thumb is dragged.
#[must_use]
pub(crate) fn position_at(range: ScrollRange, track: i32, length: i32, start: i32) -> i32 {
    let travel = i64::from(track) - i64::from(length);
    let steps = range.last_pos() - i64::from(range.min);
    if travel <= 0 || steps <= 0 {
        return range.min;
    }
    let start = i64::from(start).clamp(0, travel);
    let pos = i64::from(range.min) + (steps * start + travel / 2) / travel;
    i32::try_from(pos).unwrap_or(range.max)
}

/// Which way a click on the track pages: towards the start (`-1`) when it is
/// before the thumb, towards the end (`1`) when after, `0` on the thumb.
#[must_use]
pub(crate) fn page_direction(thumb: Thumb, at: i32) -> i32 {
    if at < thumb.start {
        -1
    } else {
        i32::from(at >= thumb.end())
    }
}

#[cfg(test)]
mod tests {
    use super::{ScrollRange, Thumb, page_direction, position_at, thumb};

    fn rows(count: i32, page: u32, pos: i32) -> ScrollRange {
        ScrollRange {
            min: 0,
            max: count - 1,
            page,
            pos,
        }
    }

    #[test]
    fn no_thumb_when_everything_fits() {
        assert_eq!(thumb(rows(30, 30, 0), 600, 24), None);
        assert_eq!(thumb(rows(10, 30, 0), 600, 24), None);
        assert_eq!(thumb(rows(0, 0, 0), 600, 24), None);
        assert_eq!(thumb(rows(100, 30, 0), 0, 24), None);
    }

    #[test]
    fn the_thumb_is_the_visible_share_and_moves_with_the_position() {
        // 100 rows, 25 visible, on a 400 px track: a 100 px thumb.
        assert_eq!(
            thumb(rows(100, 25, 0), 400, 24),
            Some(Thumb {
                start: 0,
                length: 100
            })
        );
        // The last page puts the thumb at the end of the track.
        assert_eq!(
            thumb(rows(100, 25, 75), 400, 24),
            Some(Thumb {
                start: 300,
                length: 100
            })
        );
        assert_eq!(
            thumb(rows(100, 25, 37), 400, 24).map(|t| t.start),
            Some(148)
        );
        // A position past the end is held at the end.
        assert_eq!(
            thumb(rows(100, 25, 99), 400, 24).map(|t| t.start),
            Some(300)
        );
    }

    #[test]
    fn ten_million_rows_keep_a_thumb_you_can_catch() {
        let range = rows(10_000_000, 30, 0);
        let top = thumb(range, 800, 24).unwrap_or(Thumb {
            start: -1,
            length: -1,
        });
        assert_eq!(
            top,
            Thumb {
                start: 0,
                length: 24
            }
        );
        let last = rows(10_000_000, 30, 10_000_000 - 30);
        assert_eq!(thumb(last, 800, 24).map(Thumb::end), Some(800));
        let middle = rows(10_000_000, 30, 4_999_985);
        assert_eq!(thumb(middle, 800, 24).map(|t| t.start), Some(388));
    }

    #[test]
    fn dragging_the_thumb_maps_back_to_the_position() {
        let range = rows(100, 25, 0);
        assert_eq!(position_at(range, 400, 100, 0), 0);
        assert_eq!(position_at(range, 400, 100, 300), 75);
        assert_eq!(position_at(range, 400, 100, 400), 75);
        assert_eq!(position_at(range, 400, 100, -50), 0);
        for pos in 0..=75 {
            let at = thumb(rows(100, 25, pos), 400, 24).map_or(-1, |t| t.start);
            assert_eq!(position_at(range, 400, 100, at), pos, "row {pos}");
        }
        // Across ten million rows every pixel of travel is about 12,800 rows.
        let big = rows(10_000_000, 30, 0);
        assert_eq!(position_at(big, 800, 24, 776), 9_999_970);
        assert_eq!(position_at(big, 800, 24, 1), 12_887);
    }

    #[test]
    fn pixel_ranges_scroll_across() {
        // Columns 2,000 px wide in a 1,200 px list.
        let across = ScrollRange {
            min: 0,
            max: 1_999,
            page: 1_200,
            pos: 800,
        };
        assert_eq!(
            thumb(across, 1_200, 24),
            Some(Thumb {
                start: 480,
                length: 720
            })
        );
        assert_eq!(position_at(across, 1_200, 720, 480), 800);
    }

    #[test]
    fn a_track_click_pages_towards_the_click() {
        let thumb = Thumb {
            start: 100,
            length: 50,
        };
        assert_eq!(page_direction(thumb, 99), -1);
        assert_eq!(page_direction(thumb, 100), 0);
        assert_eq!(page_direction(thumb, 149), 0);
        assert_eq!(page_direction(thumb, 150), 1);
    }
}
