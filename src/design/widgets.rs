//! Components: small building blocks the views are assembled from, so every
//! tab looks the same without repeating style details.

use iced::widget::{Space, button, container, row, scrollable, svg, text, text_input};
use iced::{Alignment, Color, Element, Font, Length, Padding, font};

use super::style;
use super::tokens::*;
use crate::i18n::Text;
pub use lucide_static_svg::Icon;

// ── Fonts ────────────────────────────────────────────────────────────────────

pub const INTER_BYTES: &[u8] = include_bytes!("../../assets/fonts/Inter.ttf");
pub const MONO_BYTES: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono.ttf");

pub const SANS: Font = Font::with_name("Inter");
pub const SANS_MEDIUM: Font = Font { weight: font::Weight::Medium, ..SANS };
pub const SANS_SEMIBOLD: Font = Font { weight: font::Weight::Semibold, ..SANS };
pub const MONO: Font = Font::with_name("JetBrains Mono");
pub const MONO_MEDIUM: Font = Font { weight: font::Weight::Medium, ..MONO };
pub const MONO_SEMIBOLD: Font = Font { weight: font::Weight::Semibold, ..MONO };

// ── Icons ────────────────────────────────────────────────────────────────────

/// A Lucide icon at `size` px, stroked in `color`.
pub fn icon_any<'a, M: 'a>(icon: Icon, size: f32, color: Color) -> Element<'a, M> {
    svg(svg::Handle::from_memory(icon.svg_bytes()))
        .width(size)
        .height(size)
        .style(move |_, _| svg::Style { color: Some(color) })
        .into()
}

// ── Text ─────────────────────────────────────────────────────────────────────

// Fixed English passed as `&str` is translated (see `crate::i18n`); text
// built at run time (`String`) is shown as it is.

pub fn label<'a>(content: impl Text<'a>, size: f32, color: Color) -> text::Text<'a> {
    text(content.localized()).size(size).color(color).font(SANS)
}

pub fn medium<'a>(content: impl Text<'a>, size: f32, color: Color) -> text::Text<'a> {
    text(content.localized()).size(size).color(color).font(SANS_MEDIUM)
}

pub fn semibold<'a>(content: impl Text<'a>, size: f32, color: Color) -> text::Text<'a> {
    text(content.localized()).size(size).color(color).font(SANS_SEMIBOLD)
}

pub fn mono<'a>(content: impl Text<'a>, size: f32, color: Color) -> text::Text<'a> {
    text(content.localized()).size(size).color(color).font(MONO)
}

/// Section heading: small, semibold, uppercase, wide tracking.
pub fn overline<'a>(content: &str) -> text::Text<'a> {
    text(spaced_upper(crate::i18n::tr(content))).size(TEXT_BASE).color(TEXT_SECONDARY).font(SANS_SEMIBOLD)
}

/// iced has no letter-spacing; thin spaces between capitals stand in for
/// Tailwind's `tracking-wider` on short headings.
fn spaced_upper(s: &str) -> String {
    s.to_uppercase()
}

// ── Badges & chips ───────────────────────────────────────────────────────────

/// Mono count/status badge tinted with `color` (10% fill, 20% border).
pub fn tinted_badge<'a, M: 'a>(content: impl Text<'a>, color: Color) -> Element<'a, M> {
    container(mono(content.localized(), TEXT_XS, color).wrapping(text::Wrapping::None)).padding([1, 6]).style(style::tint(color)).into()
}

/// Neutral count badge, e.g. the "42" on a tab or group.
pub fn count<'a, M: 'a>(n: usize, background: Color) -> Element<'a, M> {
    container(mono(n.to_string(), TEXT_XS, TEXT_SECONDARY)).padding([1, 6]).style(style::count_badge(background)).into()
}

/// Keyboard accelerator chip, e.g. `Ctrl+A`.
pub fn kbd<'a, M: 'a>(keys: &'a str, background: Color, line: Color, color: Color) -> Element<'a, M> {
    container(mono(keys, 9.0, color).wrapping(text::Wrapping::None)).padding([0, 4]).style(style::kbd(background, line)).into()
}

pub fn dot<'a, M: 'a>(color: Color, size: f32) -> Element<'a, M> {
    container(Space::new()).width(size).height(size).style(style::dot(color)).into()
}

/// Letter avatar for an app without an icon.
pub fn avatar<'a, M: 'a>(name: &str, size: f32, color: Color) -> Element<'a, M> {
    let letter = name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    container(mono(letter, if size > 20.0 { TEXT_MD } else { TEXT_XS }, color).font(MONO_SEMIBOLD))
        .width(size)
        .height(size)
        .center(Length::Fixed(size))
        .style(style::panel(AVATAR, Color::TRANSPARENT, RADIUS))
        .into()
}

/// 1px vertical separator of `height`.
pub fn vline<'a, M: 'a>(height: f32) -> Element<'a, M> {
    container(Space::new()).width(1).height(height).style(style::fill(BORDER)).into()
}

/// 1px vertical separator as tall as the parent.
pub fn vline_fill<'a, M: 'a>() -> Element<'a, M> {
    container(Space::new()).width(1).height(Length::Fill).style(style::fill(BORDER)).into()
}

/// 1px horizontal line across the parent.
pub fn hline<'a, M: 'a>(color: Color) -> Element<'a, M> {
    container(Space::new()).width(Length::Fill).height(1).style(style::fill(color)).into()
}

// ── Buttons ──────────────────────────────────────────────────────────────────

/// Icon + label (+ optional kbd hint) button, `height` px tall.
pub fn icon_button<'a, M: Clone + 'a>(
    glyph: Option<(Icon, Color)>,
    caption: &'a str,
    hint: Option<Element<'a, M>>,
    style_fn: impl Fn(&iced::Theme, button::Status) -> button::Style + 'a,
    height: f32,
    on_press: Option<M>,
) -> button::Button<'a, M> {
    let mut content = row![].spacing(6).align_y(Alignment::Center);
    if let Some((g, c)) = glyph {
        content = content.push(icon_any(g, 12.0, c));
    }
    content = content.push(text(crate::i18n::tr(caption)).size(TEXT_BASE).font(SANS).wrapping(text::Wrapping::None));
    if let Some(hint) = hint {
        content = content.push(hint);
    }
    button(container(content).center_y(Length::Fill))
        .height(height)
        .padding(Padding::from([0, 9]))
        .style(style_fn)
        .on_press_maybe(on_press)
}

/// Square icon-only button (e.g. refresh, "⋯").
pub fn square_button<'a, M: Clone + 'a>(
    glyph: Icon,
    color: Color,
    size: f32,
    style_fn: impl Fn(&iced::Theme, button::Status) -> button::Style + 'a,
    on_press: Option<M>,
) -> button::Button<'a, M> {
    button(container(icon_any(glyph, size * 0.55, color)).center(Length::Fill))
        .width(size)
        .height(size)
        .padding(0)
        .style(style_fn)
        .on_press_maybe(on_press)
}

/// The thin WinUI scrollbar.
pub fn thin_scrollbar() -> scrollable::Direction {
    scrollable::Direction::Vertical(scrollable::Scrollbar::new().width(6).scroller_width(6).margin(2))
}

// ── Inputs ───────────────────────────────────────────────────────────────────

/// 28px text field; `mono` for IPs, ports and package names.
pub fn input<'a, M: Clone + 'a>(placeholder: &str, value: &str, mono_font: bool, background: Color) -> text_input::TextInput<'a, M> {
    text_input(crate::i18n::tr(placeholder), value)
        .size(TEXT_BASE)
        .font(if mono_font { MONO } else { SANS })
        .padding(Padding::from([5, 8]))
        .style(style::input(background))
}
