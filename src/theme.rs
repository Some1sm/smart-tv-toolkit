//! One Dark colours and the widget styles built from them.

use iced::widget::{button, container};
use iced::{Background, Border, Color, Shadow, Theme, color};

pub const APP_BG: Color = color!(0x282c34);
pub const SIDEBAR_BG: Color = color!(0x21252b);
pub const SURFACE_1: Color = color!(0x2c313c);
pub const SURFACE_2: Color = color!(0x333842);
pub const SURFACE_3: Color = color!(0x3e4452);
pub const BORDER: Color = color!(0x4b5263);

pub const TEXT: Color = color!(0xabb2bf);
pub const TEXT_MUTED: Color = color!(0x7f848e);
pub const TEXT_DISABLED: Color = color!(0x5c6370);

pub const PRIMARY: Color = color!(0x61afef);
pub const SUCCESS: Color = color!(0x98c379);
pub const WARNING: Color = color!(0xe5c07b);
pub const DANGER: Color = color!(0xe06c75);
pub const DANGER_BG: Color = color!(0x3f2a2e);

pub const RADIUS_MD: f32 = 12.0;
pub const RADIUS_XL: f32 = 18.0;

pub fn app_theme() -> Theme {
    Theme::custom(
        "One Dark",
        iced::theme::Palette {
            background: APP_BG,
            text: TEXT,
            primary: PRIMARY,
            success: SUCCESS,
            warning: WARNING,
            danger: DANGER,
        },
    )
}

fn bg(c: Color) -> Option<Background> {
    Some(Background::Color(c))
}

pub fn sidebar(_: &Theme) -> container::Style {
    container::Style { background: bg(SIDEBAR_BG), text_color: Some(TEXT), ..Default::default() }
}

/// A bordered card, used to group related content.
pub fn panel(_: &Theme) -> container::Style {
    container::Style {
        background: bg(SURFACE_1),
        border: Border { color: BORDER, width: 1.0, radius: RADIUS_XL.into() },
        ..Default::default()
    }
}

pub fn log_box(_: &Theme) -> container::Style {
    container::Style {
        background: bg(SIDEBAR_BG),
        border: Border { color: BORDER, width: 1.0, radius: RADIUS_MD.into() },
        ..Default::default()
    }
}

fn button_style(background: Color, hover: Color, text: Color, border: Color, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => hover,
        _ => background,
    };
    let text_color = if status == button::Status::Disabled { TEXT_DISABLED } else { text };
    button::Style {
        background: bg(background),
        text_color,
        border: Border { color: border, width: 1.0, radius: RADIUS_MD.into() },
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn primary_button(_: &Theme, status: button::Status) -> button::Style {
    button_style(PRIMARY, color!(0x528bff), color!(0x1e2127), PRIMARY, status)
}

pub fn secondary_button(_: &Theme, status: button::Status) -> button::Style {
    button_style(SURFACE_2, SURFACE_3, TEXT, BORDER, status)
}

pub fn danger_button(_: &Theme, status: button::Status) -> button::Style {
    button_style(DANGER_BG, color!(0x4a3035), DANGER, color!(0x5a3439), status)
}

pub fn tab_button(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let (background, text_color, border) = match (active, status) {
            (_, button::Status::Disabled) => (Color::TRANSPARENT, TEXT_DISABLED, Color::TRANSPARENT),
            (true, _) => (SURFACE_3, PRIMARY, BORDER),
            (false, _) if hovered => (SURFACE_2, TEXT, Color::TRANSPARENT),
            (false, _) => (Color::TRANSPARENT, TEXT, Color::TRANSPARENT),
        };
        button::Style {
            background: bg(background),
            text_color,
            border: Border { color: border, width: 1.0, radius: RADIUS_MD.into() },
            ..Default::default()
        }
    }
}

pub fn tab_bar(_: &Theme) -> container::Style {
    container::Style {
        background: bg(SURFACE_1),
        border: Border { color: BORDER, width: 1.0, radius: RADIUS_XL.into() },
        ..Default::default()
    }
}
