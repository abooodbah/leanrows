//! Pure, DPI-aware geometry for the native `LeanRows` shell.

const BASE_DPI: u32 = 96;
const TOP_BAR_HEIGHT_DIP: i32 = 56;
const PROGRESS_HEIGHT_DIP: i32 = 2;
const STATUS_HEIGHT_DIP: i32 = 32;
const SPACE_1_DIP: i32 = 4;
const SPACE_2_DIP: i32 = 8;
const SPACE_3_DIP: i32 = 12;
const SPACE_4_DIP: i32 = 16;
const SPACE_5_DIP: i32 = 20;
const SPACE_6_DIP: i32 = 24;
const SPACE_8_DIP: i32 = 32;
const WORDMARK_SIZE_DIP: i32 = 32;
const CONTROL_HEIGHT_DIP: i32 = 40;
const PRIMARY_TARGET_HEIGHT_DIP: i32 = 44;
const COMPACT_BUTTON_WIDTH_DIP: i32 = 40;
const OPEN_BUTTON_WIDTH_DIP: i32 = 104;
const WIDE_RELOAD_WIDTH_DIP: i32 = 88;
const WIDE_GOTO_WIDTH_DIP: i32 = 96;
const NARROW_SEARCH_MIN_WIDTH_DIP: i32 = 132;
const SEARCH_MIN_WIDTH_DIP: i32 = 220;
const SEARCH_MAX_WIDTH_DIP: i32 = 360;
const FILE_IDENTITY_MIN_WIDTH_DIP: i32 = 160;
const FILE_IDENTITY_MAX_WIDTH_DIP: i32 = 440;
const MIN_CLIENT_WIDTH_DIP: i32 = 640;
const MIN_CLIENT_HEIGHT_DIP: i32 = 480;
const NARROW_MAX_WIDTH_DIP: i32 = 760;
const COMPACT_IDENTITY_MAX_WIDTH_DIP: i32 = 560;
const WIDE_MIN_WIDTH_DIP: i32 = 1_180;

/// A Win32-ready rectangle represented as an origin and extent.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct UiRect {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) width: i32,
    pub(super) height: i32,
}

impl UiRect {
    const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    #[must_use]
    pub(super) const fn right(self) -> i32 {
        self.x.saturating_add(self.width)
    }

    #[must_use]
    pub(super) const fn bottom(self) -> i32 {
        self.y.saturating_add(self.height)
    }
}

/// Responsive top-bar variants. Hidden commands remain available in overflow.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CommandLayoutMode {
    Narrow,
    Default,
    Wide,
}

/// Shared visual metrics scaled for the window's current monitor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct UiMetrics {
    pub(super) dpi: u32,
    pub(super) top_bar_height: i32,
    pub(super) progress_height: i32,
    pub(super) status_height: i32,
    pub(super) space_1: i32,
    pub(super) space_2: i32,
    pub(super) space_3: i32,
    pub(super) space_4: i32,
    pub(super) space_5: i32,
    pub(super) space_6: i32,
    pub(super) space_8: i32,
    pub(super) wordmark_size: i32,
    pub(super) control_height: i32,
    pub(super) primary_target_height: i32,
    pub(super) compact_button_width: i32,
    pub(super) open_button_width: i32,
    pub(super) wide_reload_width: i32,
    pub(super) wide_goto_width: i32,
    pub(super) narrow_search_min_width: i32,
    pub(super) search_min_width: i32,
    pub(super) search_max_width: i32,
    pub(super) file_identity_min_width: i32,
    pub(super) file_identity_max_width: i32,
    pub(super) minimum_client_width: i32,
    pub(super) minimum_client_height: i32,
    pub(super) narrow_max_width: i32,
    pub(super) compact_identity_max_width: i32,
    pub(super) wide_min_width: i32,
}

impl UiMetrics {
    /// Zero DPI safely falls back to the Win32 96-DPI baseline.
    #[must_use]
    pub(super) fn for_dpi(dpi: u32) -> Self {
        let dpi = if dpi == 0 { BASE_DPI } else { dpi };
        Self {
            dpi,
            top_bar_height: scale_dip(TOP_BAR_HEIGHT_DIP, dpi),
            progress_height: scale_dip(PROGRESS_HEIGHT_DIP, dpi),
            status_height: scale_dip(STATUS_HEIGHT_DIP, dpi),
            space_1: scale_dip(SPACE_1_DIP, dpi),
            space_2: scale_dip(SPACE_2_DIP, dpi),
            space_3: scale_dip(SPACE_3_DIP, dpi),
            space_4: scale_dip(SPACE_4_DIP, dpi),
            space_5: scale_dip(SPACE_5_DIP, dpi),
            space_6: scale_dip(SPACE_6_DIP, dpi),
            space_8: scale_dip(SPACE_8_DIP, dpi),
            wordmark_size: scale_dip(WORDMARK_SIZE_DIP, dpi),
            control_height: scale_dip(CONTROL_HEIGHT_DIP, dpi),
            primary_target_height: scale_dip(PRIMARY_TARGET_HEIGHT_DIP, dpi),
            compact_button_width: scale_dip(COMPACT_BUTTON_WIDTH_DIP, dpi),
            open_button_width: scale_dip(OPEN_BUTTON_WIDTH_DIP, dpi),
            wide_reload_width: scale_dip(WIDE_RELOAD_WIDTH_DIP, dpi),
            wide_goto_width: scale_dip(WIDE_GOTO_WIDTH_DIP, dpi),
            narrow_search_min_width: scale_dip(NARROW_SEARCH_MIN_WIDTH_DIP, dpi),
            search_min_width: scale_dip(SEARCH_MIN_WIDTH_DIP, dpi),
            search_max_width: scale_dip(SEARCH_MAX_WIDTH_DIP, dpi),
            file_identity_min_width: scale_dip(FILE_IDENTITY_MIN_WIDTH_DIP, dpi),
            file_identity_max_width: scale_dip(FILE_IDENTITY_MAX_WIDTH_DIP, dpi),
            minimum_client_width: scale_dip(MIN_CLIENT_WIDTH_DIP, dpi),
            minimum_client_height: scale_dip(MIN_CLIENT_HEIGHT_DIP, dpi),
            narrow_max_width: scale_dip(NARROW_MAX_WIDTH_DIP, dpi),
            compact_identity_max_width: scale_dip(COMPACT_IDENTITY_MAX_WIDTH_DIP, dpi),
            wide_min_width: scale_dip(WIDE_MIN_WIDTH_DIP, dpi),
        }
    }
}

/// Top-bar child rectangles. `None` places that command in overflow.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CommandLayout {
    pub(super) mode: CommandLayoutMode,
    pub(super) wordmark: Option<UiRect>,
    pub(super) file_identity: Option<UiRect>,
    pub(super) search_field: UiRect,
    pub(super) previous_match_button: Option<UiRect>,
    pub(super) next_match_button: Option<UiRect>,
    pub(super) reload_button: Option<UiRect>,
    pub(super) goto_button: Option<UiRect>,
    pub(super) theme_button: Option<UiRect>,
    pub(super) open_button: UiRect,
    pub(super) overflow_button: UiRect,
}

impl CommandLayout {
    fn calculate(bar: UiRect, metrics: UiMetrics) -> Self {
        if bar.width <= metrics.narrow_max_width {
            Self::narrow(bar, metrics)
        } else if bar.width < metrics.wide_min_width {
            Self::default_layout(bar, metrics)
        } else {
            Self::wide(bar, metrics)
        }
    }

    fn narrow(bar: UiRect, metrics: UiMetrics) -> Self {
        let inset = metrics.space_3.min(bar.width / 2);
        let left = bar.x.saturating_add(inset);
        let mut right = bar.right().saturating_sub(inset);
        let overflow_button = take_right(
            &mut right,
            left,
            metrics.compact_button_width,
            bar,
            metrics.control_height,
        );
        retreat(&mut right, left, metrics.space_2);
        let open_button = take_right(
            &mut right,
            left,
            metrics.open_button_width,
            bar,
            metrics.primary_target_height,
        );
        retreat(&mut right, left, metrics.space_2);

        let show_identity = bar.width > metrics.compact_identity_max_width;
        let identity_width = if show_identity {
            metrics.file_identity_min_width.min(
                right
                    .saturating_sub(left)
                    .saturating_sub(metrics.space_4)
                    .saturating_sub(metrics.narrow_search_min_width)
                    .max(0),
            )
        } else {
            0
        };
        let file_identity = (identity_width > 0).then(|| {
            UiRect::new(
                left,
                centered_y(bar, metrics.control_height),
                identity_width,
                metrics.control_height.min(bar.height),
            )
        });
        let search_left = file_identity.map_or(left, |identity| {
            identity.right().saturating_add(metrics.space_4).min(right)
        });
        let search_field = UiRect::new(
            search_left,
            centered_y(bar, metrics.control_height),
            right.saturating_sub(search_left),
            metrics.control_height.min(bar.height),
        );
        Self {
            mode: CommandLayoutMode::Narrow,
            wordmark: None,
            file_identity,
            search_field,
            previous_match_button: None,
            next_match_button: None,
            reload_button: None,
            goto_button: None,
            theme_button: None,
            open_button,
            overflow_button,
        }
    }

    fn default_layout(bar: UiRect, metrics: UiMetrics) -> Self {
        let inset = metrics.space_5.min(bar.width / 2);
        let left = bar.x.saturating_add(inset);
        let mut right = bar.right().saturating_sub(inset);
        let overflow_button = take_right(
            &mut right,
            left,
            metrics.compact_button_width,
            bar,
            metrics.control_height,
        );
        retreat(&mut right, left, metrics.space_2);
        let open_button = take_right(
            &mut right,
            left,
            metrics.open_button_width,
            bar,
            metrics.primary_target_height,
        );
        retreat(&mut right, left, metrics.space_2);
        let theme_button = Some(take_right(
            &mut right,
            left,
            metrics.wide_reload_width,
            bar,
            metrics.control_height,
        ));
        retreat(&mut right, left, metrics.space_2);
        let (wordmark, file_identity, search_field) =
            place_identity_and_search(left, right, bar, metrics);
        Self {
            mode: CommandLayoutMode::Default,
            wordmark: Some(wordmark),
            file_identity: Some(file_identity),
            search_field,
            previous_match_button: None,
            next_match_button: None,
            reload_button: None,
            goto_button: None,
            theme_button,
            open_button,
            overflow_button,
        }
    }

    fn wide(bar: UiRect, metrics: UiMetrics) -> Self {
        let inset = metrics.space_5.min(bar.width / 2);
        let left = bar.x.saturating_add(inset);
        let mut right = bar.right().saturating_sub(inset);
        let overflow_button = take_right(
            &mut right,
            left,
            metrics.compact_button_width,
            bar,
            metrics.control_height,
        );
        retreat(&mut right, left, metrics.space_2);
        let open_button = take_right(
            &mut right,
            left,
            metrics.open_button_width,
            bar,
            metrics.primary_target_height,
        );
        retreat(&mut right, left, metrics.space_2);
        let theme_button = Some(take_right(
            &mut right,
            left,
            metrics.wide_reload_width,
            bar,
            metrics.control_height,
        ));
        retreat(&mut right, left, metrics.space_2);
        let goto_button = Some(take_right(
            &mut right,
            left,
            metrics.wide_goto_width,
            bar,
            metrics.control_height,
        ));
        retreat(&mut right, left, metrics.space_2);
        let reload_button = Some(take_right(
            &mut right,
            left,
            metrics.wide_reload_width,
            bar,
            metrics.control_height,
        ));
        retreat(&mut right, left, metrics.space_2);
        let next_match_button = Some(take_right(
            &mut right,
            left,
            metrics.compact_button_width,
            bar,
            metrics.control_height,
        ));
        retreat(&mut right, left, metrics.space_1);
        let previous_match_button = Some(take_right(
            &mut right,
            left,
            metrics.compact_button_width,
            bar,
            metrics.control_height,
        ));
        retreat(&mut right, left, metrics.space_2);
        let (wordmark, file_identity, search_field) =
            place_identity_and_search(left, right, bar, metrics);
        Self {
            mode: CommandLayoutMode::Wide,
            wordmark: Some(wordmark),
            file_identity: Some(file_identity),
            search_field,
            previous_match_button,
            next_match_button,
            reload_button,
            goto_button,
            theme_button,
            open_button,
            overflow_button,
        }
    }
}

/// Complete client geometry for the single-topbar shell and dense grid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct UiLayout {
    pub(super) metrics: UiMetrics,
    pub(super) client: UiRect,
    pub(super) top_bar: UiRect,
    pub(super) progress_track: UiRect,
    pub(super) main_content: UiRect,
    pub(super) grid: UiRect,
    pub(super) empty_state: UiRect,
    pub(super) status_strip: UiRect,
    pub(super) commands: CommandLayout,
}

impl UiLayout {
    /// Returns non-negative, non-overlapping shell rectangles for any extent.
    #[must_use]
    pub(super) fn calculate(client_width: i32, client_height: i32, dpi: u32) -> Self {
        let metrics = UiMetrics::for_dpi(dpi);
        let width = client_width.max(0);
        let height = client_height.max(0);
        let client = UiRect::new(0, 0, width, height);
        let top_height = metrics.top_bar_height.min(height);
        let top_bar = UiRect::new(0, 0, width, top_height);
        let after_top = height.saturating_sub(top_height);
        let status_height = metrics.status_height.min(after_top);
        let status_strip = UiRect::new(
            0,
            height.saturating_sub(status_height),
            width,
            status_height,
        );
        let main_content = UiRect::new(
            0,
            top_bar.bottom(),
            width,
            status_strip.y.saturating_sub(top_bar.bottom()),
        );
        let progress_height = metrics.progress_height.min(height);
        let progress_track = UiRect::new(
            0,
            top_bar.bottom().saturating_sub(progress_height / 2),
            width,
            progress_height,
        );
        Self {
            metrics,
            client,
            top_bar,
            progress_track,
            main_content,
            grid: main_content,
            empty_state: inset(main_content, metrics.space_8),
            status_strip,
            commands: CommandLayout::calculate(top_bar, metrics),
        }
    }
}

fn place_identity_and_search(
    left: i32,
    search_right: i32,
    bar: UiRect,
    metrics: UiMetrics,
) -> (UiRect, UiRect, UiRect) {
    let wordmark = UiRect::new(
        left,
        centered_y(bar, metrics.wordmark_size),
        metrics
            .wordmark_size
            .min(search_right.saturating_sub(left).max(0)),
        metrics.wordmark_size.min(bar.height),
    );
    let file_left = wordmark
        .right()
        .saturating_add(metrics.space_3)
        .min(search_right);
    let maximum_search_preserving_identity = search_right
        .saturating_sub(file_left)
        .saturating_sub(metrics.file_identity_min_width)
        .saturating_sub(metrics.space_4)
        .max(0);
    let available_search = search_right
        .saturating_sub(file_left)
        .saturating_sub(metrics.space_4)
        .max(0);
    let search_width = if maximum_search_preserving_identity >= metrics.search_min_width {
        maximum_search_preserving_identity.min(metrics.search_max_width)
    } else {
        available_search.min(metrics.search_min_width)
    };
    let search_field = UiRect::new(
        search_right.saturating_sub(search_width),
        centered_y(bar, metrics.control_height),
        search_width,
        metrics.control_height.min(bar.height),
    );
    let file_right = search_field
        .x
        .saturating_sub(metrics.space_4)
        .max(file_left);
    let file_identity = UiRect::new(
        file_left,
        centered_y(bar, metrics.control_height),
        file_right
            .saturating_sub(file_left)
            .min(metrics.file_identity_max_width),
        metrics.control_height.min(bar.height),
    );
    (wordmark, file_identity, search_field)
}

fn take_right(right: &mut i32, left: i32, desired_width: i32, bar: UiRect, height: i32) -> UiRect {
    let width = desired_width.min(right.saturating_sub(left).max(0));
    *right = right.saturating_sub(width);
    UiRect::new(
        *right,
        centered_y(bar, height),
        width,
        height.min(bar.height),
    )
}

fn retreat(right: &mut i32, left: i32, distance: i32) {
    *right = right.saturating_sub(distance).max(left);
}

fn centered_y(container: UiRect, desired_height: i32) -> i32 {
    let height = desired_height.min(container.height);
    container
        .y
        .saturating_add(container.height.saturating_sub(height) / 2)
}

fn inset(rect: UiRect, distance: i32) -> UiRect {
    let horizontal = distance.min(rect.width / 2);
    let vertical = distance.min(rect.height / 2);
    UiRect::new(
        rect.x.saturating_add(horizontal),
        rect.y.saturating_add(vertical),
        rect.width.saturating_sub(horizontal.saturating_mul(2)),
        rect.height.saturating_sub(vertical.saturating_mul(2)),
    )
}

fn scale_dip(value: i32, dpi: u32) -> i32 {
    let product = i64::from(value).saturating_mul(i64::from(dpi));
    let rounded = product.saturating_add(i64::from(BASE_DPI / 2)) / i64::from(BASE_DPI);
    i32::try_from(rounded).unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use super::{CommandLayout, CommandLayoutMode, UiLayout, UiMetrics, UiRect, scale_dip};

    const DPIS: [u32; 3] = [96, 144, 192];
    const SIZES: [(i32, i32, CommandLayoutMode); 3] = [
        (640, 480, CommandLayoutMode::Narrow),
        (960, 640, CommandLayoutMode::Default),
        (1_440, 900, CommandLayoutMode::Wide),
    ];

    #[test]
    fn leanmark_rhythm_scales_at_96_144_and_192_dpi() {
        let expected = [
            (96, 56, 2, 32, 32, 40, 44, 12, 20),
            (144, 84, 3, 48, 48, 60, 66, 18, 30),
            (192, 112, 4, 64, 64, 80, 88, 24, 40),
        ];
        for (dpi, top, progress, status, wordmark, control, primary, narrow_pad, pad) in expected {
            let metrics = UiMetrics::for_dpi(dpi);
            assert_eq!(metrics.dpi, dpi);
            assert_eq!(metrics.top_bar_height, top);
            assert_eq!(metrics.progress_height, progress);
            assert_eq!(metrics.status_height, status);
            assert_eq!(metrics.wordmark_size, wordmark);
            assert_eq!(metrics.control_height, control);
            assert_eq!(metrics.primary_target_height, primary);
            assert_eq!(metrics.space_3, narrow_pad);
            assert_eq!(metrics.space_5, pad);
            assert_eq!(metrics.minimum_client_width, scale_dip(640, dpi));
            assert_eq!(metrics.minimum_client_height, scale_dip(480, dpi));
        }
    }

    #[test]
    fn zero_dpi_uses_the_win32_baseline() {
        assert_eq!(UiMetrics::for_dpi(0), UiMetrics::for_dpi(96));
    }

    #[test]
    fn shell_regions_scale_without_a_separate_document_strip() {
        for dpi in DPIS {
            let width = scale_dip(960, dpi);
            let height = scale_dip(640, dpi);
            let layout = UiLayout::calculate(width, height, dpi);
            assert_eq!(layout.top_bar, UiRect::new(0, 0, width, scale_dip(56, dpi)));
            assert_eq!(
                layout.progress_track,
                UiRect::new(0, scale_dip(55, dpi), width, scale_dip(2, dpi))
            );
            assert_eq!(
                layout.main_content,
                UiRect::new(0, scale_dip(56, dpi), width, scale_dip(552, dpi))
            );
            assert_eq!(layout.grid, layout.main_content);
            assert_eq!(
                layout.status_strip,
                UiRect::new(0, scale_dip(608, dpi), width, scale_dip(32, dpi))
            );
            assert_eq!(layout.status_strip.bottom(), layout.client.bottom());
        }
    }

    #[test]
    fn responsive_modes_and_rectangles_hold_for_every_dpi_and_size() {
        for dpi in DPIS {
            for (width, height, expected_mode) in SIZES {
                let layout =
                    UiLayout::calculate(scale_dip(width, dpi), scale_dip(height, dpi), dpi);
                assert_eq!(layout.commands.mode, expected_mode);
                assert_commands_fit(layout.top_bar, layout.commands);
            }
        }
    }

    #[test]
    fn narrow_matches_leanmark_compaction() {
        let commands = UiLayout::calculate(640, 480, 96).commands;
        assert_eq!(commands.mode, CommandLayoutMode::Narrow);
        assert!(commands.wordmark.is_none());
        assert_eq!(commands.file_identity.map(|rect| rect.width), Some(160));
        assert_eq!(commands.search_field.height, 40);
        assert!(commands.theme_button.is_none());
        assert!(commands.reload_button.is_none());
        assert!(commands.goto_button.is_none());
        assert_eq!(commands.open_button.height, 44);
        assert_eq!(commands.overflow_button.width, 40);

        let compact = UiLayout::calculate(560, 480, 96).commands;
        assert!(compact.file_identity.is_none());
        assert_eq!(compact.search_field.x, 12);
    }

    #[test]
    fn default_matches_the_core_leanmark_toolbar() {
        let commands = UiLayout::calculate(960, 640, 96).commands;
        assert_eq!(commands.mode, CommandLayoutMode::Default);
        assert_eq!(commands.wordmark.map(|rect| rect.width), Some(32));
        assert!(commands.file_identity.is_some());
        assert_eq!(commands.search_field.width, 360);
        assert_eq!(commands.theme_button.map(|rect| rect.width), Some(88));
        assert_eq!(commands.open_button.width, 104);
        assert!(commands.previous_match_button.is_none());
        assert!(commands.next_match_button.is_none());
    }

    #[test]
    fn wide_exposes_dense_grid_actions_without_changing_the_core_rhythm() {
        let commands = UiLayout::calculate(1_440, 900, 96).commands;
        assert_eq!(commands.mode, CommandLayoutMode::Wide);
        assert_eq!(commands.search_field.width, 360);
        assert_eq!(commands.reload_button.map(|rect| rect.width), Some(88));
        assert_eq!(commands.goto_button.map(|rect| rect.width), Some(96));
        assert!(commands.previous_match_button.is_some());
        assert!(commands.next_match_button.is_some());
        assert!(commands.theme_button.is_some());
    }

    #[test]
    fn scaled_breakpoints_select_the_same_modes() {
        for dpi in DPIS {
            let metrics = UiMetrics::for_dpi(dpi);
            assert_eq!(
                UiLayout::calculate(metrics.narrow_max_width, scale_dip(640, dpi), dpi)
                    .commands
                    .mode,
                CommandLayoutMode::Narrow
            );
            assert_eq!(
                UiLayout::calculate(metrics.narrow_max_width + 1, scale_dip(640, dpi), dpi)
                    .commands
                    .mode,
                CommandLayoutMode::Default
            );
            assert_eq!(
                UiLayout::calculate(metrics.wide_min_width, scale_dip(640, dpi), dpi)
                    .commands
                    .mode,
                CommandLayoutMode::Wide
            );
        }
    }

    #[test]
    fn constrained_extents_saturate_instead_of_overlapping() {
        for (width, height) in [(0, 0), (-20, -10), (1, 1), (120, 60), (320, 100)] {
            let layout = UiLayout::calculate(width, height, 192);
            for rect in [
                layout.client,
                layout.top_bar,
                layout.main_content,
                layout.grid,
                layout.empty_state,
                layout.status_strip,
            ] {
                assert!(rect.x >= 0 && rect.y >= 0);
                assert!(rect.width >= 0 && rect.height >= 0);
                assert!(rect.right() <= layout.client.right());
                assert!(rect.bottom() <= layout.client.bottom());
            }
            assert_commands_fit(layout.top_bar, layout.commands);
        }
    }

    fn assert_commands_fit(bar: UiRect, commands: CommandLayout) {
        let mut rects: Vec<UiRect> = [
            commands.wordmark,
            commands.file_identity,
            Some(commands.search_field),
            commands.previous_match_button,
            commands.next_match_button,
            commands.reload_button,
            commands.goto_button,
            commands.theme_button,
            Some(commands.open_button),
            Some(commands.overflow_button),
        ]
        .into_iter()
        .flatten()
        .collect();
        for rect in &rects {
            assert!(rect.x >= bar.x && rect.y >= bar.y);
            assert!(rect.right() <= bar.right() && rect.bottom() <= bar.bottom());
        }
        rects.sort_unstable_by_key(|rect| rect.x);
        for pair in rects.windows(2) {
            assert!(pair[0].right() <= pair[1].x, "overlap: {pair:?}");
        }
    }
}
