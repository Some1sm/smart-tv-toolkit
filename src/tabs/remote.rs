//! Remote: the TV remote's buttons, plus keyboard control while open.

use iced::widget::{button, column, container, row, scrollable, space, text};
use iced::{Alignment, Element, Fill, Task};

use super::{Ctx, View};
use crate::android::remote::{self, Key};
use crate::theme;
use crate::ui::ButtonStyle;

#[derive(Debug, Default)]
pub struct State;

#[derive(Debug, Clone)]
pub enum Msg {
    Press(Key),
    Pressed(Result<(), String>),
}

impl State {
    pub fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::Press(key) => {
                let Some(conn) = ctx.conn() else { return Task::none() };
                return Task::perform(remote::press(conn, key), Msg::Pressed);
            }
            Msg::Pressed(Ok(())) => {}
            Msg::Pressed(Err(e)) => ctx.log(format!("Remote: {e}")),
        }
        Task::none()
    }

    pub fn view<'a>(&'a self, _v: &View<'a>) -> Element<'a, Msg> {
        use remote::*;
        let b = |key: Key, style: ButtonStyle| -> Element<'a, Msg> {
            button(text(key.label).size(15).center().width(Fill).height(Fill))
                .width(100)
                .height(48)
                .style(style)
                .on_press(Msg::Press(key))
                .into()
        };
        let key = |k: Key| b(k, theme::secondary_button);
        let gap = || -> Element<'a, Msg> { space().width(100).height(48).into() };

        let pad = column![
            row![gap(), key(UP), gap(), key(VOLUME_UP)].spacing(8),
            row![key(LEFT), b(OK, theme::primary_button), key(RIGHT), key(MUTE)].spacing(8),
            row![gap(), key(DOWN), gap(), key(VOLUME_DOWN)].spacing(8),
            space().height(8),
            row![key(BACK), key(HOME), key(MENU), gap()].spacing(8),
            row![key(PREVIOUS), key(PLAY_PAUSE), key(NEXT), gap()].spacing(8),
            space().height(8),
            row![gap(), b(POWER, theme::danger_button), gap(), gap()].spacing(8),
        ]
        .spacing(8);

        let hint = text(
            "Keyboard works too while this tab is open: arrow keys, Enter = OK, Backspace or Esc = Back, \
             Home, Space = play/pause, + and − = volume, M = mute.",
        )
        .size(12)
        .color(theme::TEXT_MUTED);

        scrollable(
            column![container(pad).padding(20).style(theme::panel), hint]
                .spacing(12)
                .padding(4)
                .align_x(Alignment::Center)
                .width(Fill),
        )
        .into()
    }
}

/// Map a key press on the computer to a TV remote key (Remote tab only).
pub fn key_for(event: iced::keyboard::Event) -> Option<Msg> {
    use iced::keyboard::{Event, Key as K, key::Named};
    use remote::*;
    let Event::KeyPressed { key, repeat: false, .. } = event else { return None };
    let tv = match key.as_ref() {
        K::Named(Named::ArrowUp) => UP,
        K::Named(Named::ArrowDown) => DOWN,
        K::Named(Named::ArrowLeft) => LEFT,
        K::Named(Named::ArrowRight) => RIGHT,
        K::Named(Named::Enter) => OK,
        K::Named(Named::Backspace | Named::Escape) => BACK,
        K::Named(Named::Home) => HOME,
        K::Named(Named::Space | Named::MediaPlayPause) => PLAY_PAUSE,
        K::Named(Named::AudioVolumeUp) | K::Character("+" | "=") => VOLUME_UP,
        K::Named(Named::AudioVolumeDown) | K::Character("-") => VOLUME_DOWN,
        K::Named(Named::AudioVolumeMute) | K::Character("m" | "M") => MUTE,
        _ => return None,
    };
    Some(Msg::Press(tv))
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::keyboard::{Event, Key, Location, Modifiers, key};

    fn press(k: Key, repeat: bool) -> Event {
        Event::KeyPressed {
            key: k.clone(),
            modified_key: k,
            physical_key: key::Physical::Code(key::Code::KeyA),
            location: Location::Standard,
            modifiers: Modifiers::default(),
            text: None,
            repeat,
        }
    }

    fn tv(event: Event) -> Option<u16> {
        match key_for(event) {
            Some(Msg::Press(k)) => Some(k.code),
            _ => None,
        }
    }

    #[test]
    fn keyboard_maps_to_remote_keys_and_ignores_repeats() {
        assert_eq!(tv(press(Key::Named(key::Named::ArrowUp), false)), Some(remote::UP.code));
        assert_eq!(tv(press(Key::Named(key::Named::Escape), false)), Some(remote::BACK.code));
        assert_eq!(tv(press(Key::Character("+".into()), false)), Some(remote::VOLUME_UP.code));
        assert_eq!(tv(press(Key::Character("m".into()), false)), Some(remote::MUTE.code));
        // Holding a key must not queue dozens of slow `input keyevent` calls.
        assert_eq!(tv(press(Key::Named(key::Named::ArrowUp), true)), None);
        assert_eq!(tv(press(Key::Character("x".into()), false)), None);
    }
}
