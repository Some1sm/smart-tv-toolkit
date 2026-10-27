//! Design tokens: the Fluent (Windows 11 / WinUI 3) dark palette from
//! `design/reference.html` (its Tailwind `fluent.*` colours) and the Tailwind
//! status colours it uses. See `design/DESIGN.md` for the intent behind them.

use iced::{Color, color};

// ── Surfaces (darkest → lightest) ────────────────────────────────────────────
/// Terminal body.
pub const TERMINAL: Color = color!(0x121212);
/// Text fields and other inset wells.
pub const INSET: Color = color!(0x161616);
/// Terminal header, gauge wells, kbd chips on dark.
pub const WELL: Color = color!(0x1a1a1a);
/// Sidebar's bottom strips (status footer).
pub const FOOTER: Color = color!(0x181818);
/// Sidebar manual-connect box.
pub const SIDEBAR_LOW: Color = color!(0x1b1b1b);
/// Title bar, data grid canvas ("Mica alt").
pub const BG: Color = color!(0x1c1c1c);
/// Sidebar and sub-toolbars.
pub const SIDEBAR: Color = color!(0x1e1e1e);
/// Tab bar strip.
pub const HEADER: Color = color!(0x1f1f1f);
/// Main workspace.
pub const SOLID: Color = color!(0x202020);
/// Data grid column header.
pub const GRID_HEADER: Color = color!(0x222222);
/// Telemetry card.
pub const CARD_DIM: Color = color!(0x232323);
/// Group header rows.
pub const GROUP: Color = color!(0x242424);
/// Row hover.
pub const CELL_HOVER: Color = color!(0x262626);
/// Selected row.
pub const SELECTED_ROW: Color = color!(0x262b32);
/// Cards, standard buttons.
pub const CARD: Color = color!(0x272727);
/// Active tab pill.
pub const TAB_ACTIVE: Color = color!(0x2b2b2b);
/// Subtle fill / hover, active filter pill.
pub const SUBTLE: Color = color!(0x2d2d2d);
/// Count badges inside group rows.
pub const BADGE: Color = color!(0x303030);
/// Letter avatars.
pub const AVATAR: Color = color!(0x333333);

// ── Lines ────────────────────────────────────────────────────────────────────
pub const BORDER: Color = color!(0x343434);
pub const BORDER_SUBTLE: Color = color!(0x2c2c2c);
pub const DIVIDER: Color = color!(0x2a2a2a);
pub const FLYOUT_BORDER: Color = color!(0x3e3e3e);

// ── Text ─────────────────────────────────────────────────────────────────────
pub const TEXT: Color = color!(0xf3f3f3);
pub const TEXT_SECONDARY: Color = color!(0xa0a0a0);
pub const TEXT_TERTIARY: Color = color!(0x777777);
/// Terminal and log text.
pub const TERMINAL_TEXT: Color = color!(0xcccccc);

// ── Accent ───────────────────────────────────────────────────────────────────
/// Windows 11 Fluent blue (light accent on dark).
pub const ACCENT: Color = color!(0x60cdff);
pub const ACCENT_HOVER: Color = color!(0x72d4ff);
/// Text on an accent fill.
pub const ACCENT_TEXT: Color = color!(0x002033);
/// Selected "⋯" button fill.
pub const ACCENT_WELL: Color = color!(0x2c343d);
/// kbd chip inside the primary button.
pub const ACCENT_KBD: Color = color!(0x003852);

// ── Status (Tailwind) ────────────────────────────────────────────────────────
pub const GREEN: Color = color!(0x34d399); // emerald-400
pub const AMBER: Color = color!(0xfbbf24); // amber-400
pub const RED: Color = color!(0xf87171); // red-400
pub const RED_SOFT: Color = color!(0xfca5a5); // red-300
pub const RED_PALE: Color = color!(0xfecaca); // red-200
pub const PURPLE: Color = color!(0xc084fc); // purple-400
pub const PURPLE_SOFT: Color = color!(0xd8b4fe); // purple-300
pub const CYAN: Color = color!(0x22d3ee); // cyan-400

// ── Danger surfaces ──────────────────────────────────────────────────────────
pub const DANGER_SURFACE: Color = color!(0x2a2121);
pub const DANGER_SURFACE_HOVER: Color = color!(0x352525);
pub const DANGER_SUBTLE: Color = color!(0x3b2323);
/// Windows' close-button red.
pub const CLOSE_HOVER: Color = color!(0xc42b1c);

/// `color` at `alpha` (0–1), for the `/10`, `/20`, `/30` Tailwind tints.
pub const fn alpha(color: Color, alpha: f32) -> Color {
    Color { a: alpha, ..color }
}

// ── Shape & spacing (px) ─────────────────────────────────────────────────────
pub const RADIUS: f32 = 4.0;
pub const RADIUS_MD: f32 = 6.0;
pub const RADIUS_LG: f32 = 8.0;

// ── Type scale (px) ──────────────────────────────────────────────────────────
pub const TEXT_2XS: f32 = 9.5;
pub const TEXT_XS: f32 = 10.0;
pub const TEXT_SM: f32 = 11.0;
pub const TEXT_BASE: f32 = 12.0;
pub const TEXT_MD: f32 = 13.0;
pub const TEXT_LG: f32 = 16.0;

// ── Fixed sizes ──────────────────────────────────────────────────────────────
pub const TITLE_BAR_HEIGHT: f32 = 36.0;
pub const SIDEBAR_WIDTH: f32 = 300.0;
pub const TAB_BAR_HEIGHT: f32 = 40.0;
pub const TOOLBAR_HEIGHT: f32 = 40.0;
pub const COMMAND_BAR_HEIGHT: f32 = 48.0;
pub const TERMINAL_HEIGHT: f32 = 176.0;
pub const CONTROL_HEIGHT: f32 = 28.0;
