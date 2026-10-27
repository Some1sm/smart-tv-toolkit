//! The design system, from `design/DESIGN.md` and `design/reference.html`
//! (Fluent / Windows 11 dark, "Smart TV Toolkit" mock-up):
//!
//! - [`tokens`]: colours, radii, spacing, type sizes and fixed dimensions.
//! - [`style`]: style functions for iced widgets (buttons, inputs, cards…).
//! - [`widgets`]: components built from both (icons, badges, kbd chips…).
//!
//! Views should take colours and sizes from here rather than hard-coding them.

pub mod style;
pub mod tokens;
pub mod widgets;

pub use tokens::*;
pub use widgets::*;

/// The iced theme: only the palette matters, since widgets use [`style`].
pub fn theme() -> iced::Theme {
    iced::Theme::custom(
        "Fluent Dark",
        iced::theme::Palette {
            background: SOLID,
            text: TEXT,
            primary: ACCENT,
            success: GREEN,
            warning: AMBER,
            danger: RED,
        },
    )
}
