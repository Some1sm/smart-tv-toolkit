//! Widget styles. Each function is passed to a widget's `.style(…)`.

use iced::border::Radius;
use iced::widget::{button, container, progress_bar, scrollable, text_input};
use iced::widget::{checkbox as cb, pick_list as pl};
use iced::{Background, Border, Color, Shadow, Theme, Vector, color};

use super::tokens::*;

fn bg(c: Color) -> Option<Background> {
    Some(Background::Color(c))
}

fn border(color: Color, radius: f32) -> Border {
    Border { color, width: 1.0, radius: radius.into() }
}

fn hovered(status: button::Status) -> bool {
    matches!(status, button::Status::Hovered | button::Status::Pressed)
}

// ── Containers ───────────────────────────────────────────────────────────────

pub fn fill(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style { background: bg(color), text_color: Some(TEXT), ..Default::default() }
}

/// A surface with a 1px border, e.g. a device card or gauge well.
pub fn panel(background: Color, line: Color, radius: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style { background: bg(background), border: border(line, radius), ..Default::default() }
}

/// A page section (titled card on a tab): `#272727`, 8px corners.
pub fn section(_: &Theme) -> container::Style {
    container::Style { background: bg(CARD), border: border(BORDER, RADIUS_LG), ..Default::default() }
}

/// A section that needs attention, e.g. a restore preview.
pub fn preview(_: &Theme) -> container::Style {
    container::Style { background: bg(CARD), border: border(alpha(ACCENT, 0.5), RADIUS_LG), ..Default::default() }
}

/// A recessed well inside a section, like the telemetry gauges.
pub fn well(_: &Theme) -> container::Style {
    container::Style { background: bg(WELL), border: border(alpha(BORDER, 0.6), RADIUS), ..Default::default() }
}

/// Flyout / context menu (Level 3): acrylic tone, contour stroke, drop shadow.
pub fn flyout(_: &Theme) -> container::Style {
    container::Style {
        background: bg(alpha(color_36(), 0.97)),
        border: border(FLYOUT_BORDER, RADIUS_LG),
        shadow: Shadow { color: alpha(Color::BLACK, 0.65), offset: Vector::new(0.0, 16.0), blur_radius: 36.0 },
        ..Default::default()
    }
}

const fn color_36() -> Color {
    Color { r: 36.0 / 255.0, g: 36.0 / 255.0, b: 36.0 / 255.0, a: 1.0 }
}

/// A small tinted badge: `color` text over a 10% fill with a 20% border.
pub fn tint(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: bg(alpha(color, 0.10)),
        border: border(alpha(color, 0.20), RADIUS),
        text_color: Some(color),
        ..Default::default()
    }
}

/// Neutral count badge (`#303030`, or `#1a1a1a` on an active tab).
pub fn count_badge(background: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: bg(background),
        border: border(alpha(BORDER, 0.8), RADIUS),
        text_color: Some(TEXT_SECONDARY),
        ..Default::default()
    }
}

/// Keyboard accelerator chip.
pub fn kbd(background: Color, line: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style { background: bg(background), border: border(line, RADIUS), ..Default::default() }
}

/// Status dot.
pub fn dot(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style { background: bg(color), border: Border { radius: 999.0.into(), ..Default::default() }, ..Default::default() }
}

/// Plain accent fill (the selected grid row's left edge).
pub fn accent_fill(_: &Theme) -> container::Style {
    container::Style { background: bg(ACCENT), ..Default::default() }
}

/// Nothing at all; the unselected twin of [`accent_fill`].
pub fn clear(_: &Theme) -> container::Style {
    container::Style::default()
}

/// Selected-row accent bar and the device card's selection pill.
pub fn accent_bar(_: &Theme) -> container::Style {
    container::Style {
        background: bg(ACCENT),
        border: Border { radius: Radius { top_left: 0.0, top_right: 2.0, bottom_right: 2.0, bottom_left: 0.0 }, ..Default::default() },
        ..Default::default()
    }
}

// ── Buttons ──────────────────────────────────────────────────────────────────

/// Standard action: `#272727`, 1px border, brightens on hover.
pub fn standard(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: bg(if hovered(status) { SUBTLE } else { CARD }),
        text_color: if status == button::Status::Disabled { TEXT_TERTIARY } else { TEXT },
        border: border(BORDER, RADIUS),
        shadow: Shadow::default(),
        snap: true,
    }
}

/// Primary action: solid accent, dark text.
pub fn primary(_: &Theme, status: button::Status) -> button::Style {
    let disabled = status == button::Status::Disabled;
    button::Style {
        background: bg(if disabled { alpha(ACCENT, 0.35) } else if hovered(status) { ACCENT_HOVER } else { ACCENT }),
        text_color: ACCENT_TEXT,
        border: Border { radius: RADIUS.into(), ..Default::default() },
        shadow: Shadow { color: alpha(Color::BLACK, 0.25), offset: Vector::new(0.0, 1.0), blur_radius: 2.0 },
        snap: true,
    }
}

/// Destructive action: dark red surface, red border at 30%, soft red text.
pub fn danger(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: bg(if hovered(status) { DANGER_SURFACE_HOVER } else { DANGER_SURFACE }),
        text_color: if status == button::Status::Disabled { alpha(RED_SOFT, 0.4) } else { RED_SOFT },
        border: border(alpha(color_red_500(), 0.30), RADIUS),
        shadow: Shadow::default(),
        snap: true,
    }
}

const fn color_red_500() -> Color {
    Color { r: 239.0 / 255.0, g: 68.0 / 255.0, b: 68.0 / 255.0, a: 1.0 }
}

/// Borderless until hovered, e.g. row actions, toolbar icons.
pub fn ghost(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: if hovered(status) { bg(SUBTLE) } else { None },
        text_color: if status == button::Status::Disabled {
            TEXT_TERTIARY
        } else if hovered(status) {
            TEXT
        } else {
            TEXT_SECONDARY
        },
        border: Border { radius: RADIUS.into(), ..Default::default() },
        shadow: Shadow::default(),
        snap: true,
    }
}

/// Small outlined button inside grid rows (`h-6`, 1px border, subtle hover).
pub fn outline(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: if hovered(status) { bg(SUBTLE) } else { None },
        text_color: if hovered(status) { TEXT } else { TEXT_SECONDARY },
        border: border(BORDER, RADIUS),
        shadow: Shadow::default(),
        snap: true,
    }
}

/// Small toolbar chip (`#242424`, 1px border), e.g. the terminal's Stream toggle.
pub fn chip(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: bg(if hovered(status) { SUBTLE } else { GROUP }),
        text_color: TEXT,
        border: border(BORDER, RADIUS),
        shadow: Shadow::default(),
        snap: true,
    }
}

/// Sortable grid column header: hover tint only.
pub fn header_cell(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: if hovered(status) { bg(SUBTLE) } else { None },
        text_color: TEXT_SECONDARY,
        border: Border::default(),
        shadow: Shadow::default(),
        snap: true,
    }
}

/// [`outline`] in its pressed-in state, e.g. the "⋯" of the inspected row.
pub fn outline_active(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: bg(if hovered(status) { SUBTLE } else { ACCENT_WELL }),
        text_color: ACCENT,
        border: border(BORDER, RADIUS),
        shadow: Shadow::default(),
        snap: true,
    }
}

/// Text-only link that turns accent (or white) on hover.
pub fn link(hover: Color) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: None,
        text_color: if hovered(status) { hover } else { TEXT_SECONDARY },
        border: Border::default(),
        shadow: Shadow::default(),
        snap: true,
    }
}

/// Top pivot tabs: active is a raised pill with a border.
pub fn tab(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let (background, text, line) = match (active, status) {
            (_, button::Status::Disabled) => (None, TEXT_TERTIARY, Color::TRANSPARENT),
            (true, _) => (bg(TAB_ACTIVE), TEXT, BORDER),
            (false, s) if hovered(s) => (bg(SUBTLE), TEXT, Color::TRANSPARENT),
            (false, _) => (None, TEXT_SECONDARY, Color::TRANSPARENT),
        };
        button::Style {
            background,
            text_color: text,
            border: border(line, RADIUS),
            shadow: if active {
                Shadow { color: alpha(Color::BLACK, 0.25), offset: Vector::new(0.0, 1.0), blur_radius: 2.0 }
            } else {
                Shadow::default()
            },
            snap: true,
        }
    }
}

/// Filter pills in sub-toolbars.
pub fn pill(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let (background, text, line) = if active {
            (bg(SUBTLE), TEXT, BORDER)
        } else if hovered(status) {
            (bg(SUBTLE), TEXT, Color::TRANSPARENT)
        } else {
            (None, TEXT_SECONDARY, Color::TRANSPARENT)
        };
        button::Style { background, text_color: text, border: border(line, RADIUS), shadow: Shadow::default(), snap: true }
    }
}

/// A list row that highlights on hover (device cards, flyout menu items).
pub fn row(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let (background, line) = if selected {
            (bg(CARD), BORDER)
        } else if hovered(status) {
            (bg(alpha(SUBTLE, 0.7)), BORDER)
        } else {
            (None, Color::TRANSPARENT)
        };
        button::Style { background, text_color: TEXT, border: border(line, RADIUS), shadow: Shadow::default(), snap: true }
    }
}

/// Flyout menu entry; `danger` entries are red with a red-tinted hover.
pub fn menu_item(danger: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: match (hovered(status), danger) {
            (true, true) => bg(DANGER_SUBTLE),
            (true, false) => bg(SUBTLE),
            _ => None,
        },
        text_color: match (danger, hovered(status)) {
            (true, true) => RED_PALE,
            (true, false) => RED_SOFT,
            _ => TEXT,
        },
        border: Border { radius: RADIUS.into(), ..Default::default() },
        shadow: Shadow::default(),
        snap: true,
    }
}

/// Window caption buttons; `close` turns Windows red on hover.
pub fn caption(close: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let h = hovered(status);
        button::Style {
            background: match (h, close) {
                (true, true) => bg(CLOSE_HOVER),
                (true, false) => bg(SUBTLE),
                _ => None,
            },
            text_color: match (h, close) {
                (true, true) => Color::WHITE,
                (true, false) => TEXT,
                _ => TEXT_SECONDARY,
            },
            border: Border::default(),
            shadow: Shadow::default(),
            snap: true,
        }
    }
}

/// Grid row / group row backgrounds are buttons so a click selects them.
pub fn grid_row(selected: bool, group: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let h = hovered(status);
        let background = match (group, selected, h) {
            (true, _, true) => color!(0x292929),
            (true, _, false) => GROUP,
            (false, true, true) => color!(0x2b3038),
            (false, true, false) => alpha(SELECTED_ROW, 0.8),
            (false, false, true) => CELL_HOVER,
            (false, false, false) => Color::TRANSPARENT,
        };
        button::Style { background: bg(background), text_color: TEXT, border: Border::default(), shadow: Shadow::default(), snap: true }
    }
}


// ── Inputs ───────────────────────────────────────────────────────────────────

/// Text field on `background` (`#161616` or `#181818`); accent border on focus.
pub fn input(background: Color) -> impl Fn(&Theme, text_input::Status) -> text_input::Style {
    move |_, status| text_input::Style {
        background: Background::Color(background),
        border: border(
            match status {
                text_input::Status::Focused { .. } => ACCENT,
                _ => BORDER,
            },
            RADIUS,
        ),
        icon: TEXT_TERTIARY,
        placeholder: TEXT_TERTIARY,
        value: if matches!(status, text_input::Status::Disabled) { TEXT_TERTIARY } else { TEXT },
        selection: alpha(ACCENT, 0.35),
    }
}

/// A text field drawn inside its own bordered box (e.g. after a search icon).
pub fn bare_input(_: &Theme, status: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: Background::Color(Color::TRANSPARENT),
        border: Border::default(),
        icon: TEXT_TERTIARY,
        placeholder: TEXT_TERTIARY,
        value: if matches!(status, text_input::Status::Disabled) { TEXT_TERTIARY } else { TEXT },
        selection: alpha(ACCENT, 0.35),
    }
}

pub fn checkbox(_: &Theme, status: cb::Status) -> cb::Style {
    let checked = match status {
        cb::Status::Active { is_checked }
        | cb::Status::Hovered { is_checked }
        | cb::Status::Disabled { is_checked } => is_checked,
    };
    cb::Style {
        background: Background::Color(if checked { ACCENT } else { color!(0x181818) }),
        icon_color: ACCENT_TEXT,
        border: border(if checked { ACCENT } else { BORDER }, 3.0),
        text_color: None,
    }
}

pub fn pick_list(_: &Theme, status: pl::Status) -> pl::Style {
    pl::Style {
        text_color: TEXT,
        placeholder_color: TEXT_TERTIARY,
        handle_color: TEXT_SECONDARY,
        background: Background::Color(if matches!(status, pl::Status::Hovered) { SUBTLE } else { GROUP }),
        border: border(if matches!(status, pl::Status::Opened { .. }) { ACCENT } else { BORDER }, RADIUS),
    }
}

pub fn menu(_: &Theme) -> iced::overlay::menu::Style {
    iced::overlay::menu::Style {
        background: Background::Color(color_36()),
        border: border(FLYOUT_BORDER, RADIUS_MD),
        text_color: TEXT,
        selected_text_color: TEXT,
        selected_background: Background::Color(SUBTLE),
        shadow: Shadow { color: alpha(Color::BLACK, 0.5), offset: Vector::new(0.0, 8.0), blur_radius: 24.0 },
    }
}

/// Thin WinUI scrollbar: transparent rail, `#3e3e3e` thumb.
pub fn scrollbar(_: &Theme, status: scrollable::Status) -> scrollable::Style {
    let active = matches!(status, scrollable::Status::Dragged { .. })
        || matches!(status, scrollable::Status::Hovered { is_vertical_scrollbar_hovered: true, .. });
    let rail = scrollable::Rail {
        background: None,
        border: Border::default(),
        scroller: scrollable::Scroller {
            background: Background::Color(if active { color!(0x505050) } else { FLYOUT_BORDER }),
            border: Border { radius: 3.0.into(), ..Default::default() },
        },
    };
    scrollable::Style {
        container: container::Style::default(),
        vertical_rail: rail,
        horizontal_rail: rail,
        gap: None,
        auto_scroll: scrollable::AutoScroll {
            background: Background::Color(SUBTLE),
            border: border(BORDER, RADIUS),
            shadow: Shadow::default(),
            icon: TEXT,
        },
    }
}

/// Gauge bar: `#2d2d2d` track with a coloured fill.
pub fn gauge(fill: Color) -> impl Fn(&Theme) -> progress_bar::Style {
    move |_| progress_bar::Style {
        background: Background::Color(SUBTLE),
        bar: Background::Color(fill),
        border: Border { radius: 999.0.into(), ..Default::default() },
    }
}

/// Usage gauge that warns as it fills (amber at 75%, red at 90%).
pub fn usage(fraction: f32) -> impl Fn(&Theme) -> progress_bar::Style {
    gauge(if fraction >= 0.9 {
        RED
    } else if fraction >= 0.75 {
        AMBER
    } else {
        ACCENT
    })
}
