//! Tab contents. Each function renders from plain data so `main.rs` only
//! wires state and messages.

use std::collections::BTreeSet;

use iced::widget::{button, column, container, progress_bar, row, scrollable, space, text, text_input, tooltip};
use iced::{Alignment, Element, Fill, Font};

use crate::Message;
use crate::android::actions::{self, Action as TvAction, Current};
use crate::android::catalog::{self, AppSpec, SHIZUKU};
use crate::android::files::Entry;
use crate::android::health::{Finding, Fix, Severity};
use crate::android::packages::{self, Action, ListKind, Package};
use crate::android::sysinfo::Section;
use crate::theme;

pub fn empty_state<'a>(title: &str, subtitle: &str) -> Element<'a, Message> {
    container(
        column![
            text(title.to_owned()).size(16).color(theme::TEXT),
            text(subtitle.to_owned()).size(12).color(theme::TEXT_MUTED),
        ]
        .spacing(6)
        .align_x(Alignment::Center),
    )
    .center(Fill)
    .into()
}

fn section_title(title: &str) -> Element<'_, Message> {
    text(title).size(12).color(theme::TEXT_MUTED).into()
}

// ── Info ─────────────────────────────────────────────────────────────────────

pub fn info(sections: Option<&[Section]>, loading: bool) -> Element<'_, Message> {
    let Some(sections) = sections else {
        return if loading {
            empty_state("Reading device info…", "")
        } else {
            empty_state("No TV connected", "Pick a TV in the sidebar, or enter its IP address and press Connect.")
        };
    };

    let cards = sections.iter().map(|section| {
        let rows = section.fields.iter().map(|f| {
            let mut value = column![text(&f.value).size(13).color(theme::TEXT)].spacing(4);
            if let Some(used) = f.usage {
                value = value.push(progress_bar(0.0..=1.0, used).length(320).girth(8).style(theme::usage_bar(used)));
            }
            row![
                text(format!("{}:", f.label))
                    .size(13)
                    .color(theme::TEXT_MUTED)
                    .width(150)
                    .align_x(iced::alignment::Horizontal::Right),
                value.width(Fill),
            ]
            .spacing(12)
            .into()
        });
        container(column![section_title(section.title), column(rows).spacing(10)].spacing(10))
            .padding(16)
            .width(Fill)
            .style(theme::panel)
            .into()
    });

    let refresh = if loading { "Refreshing…" } else { "Refresh Info" };
    let buttons = row![
        button(text(refresh).size(13))
            .padding([8, 16])
            .style(theme::secondary_button)
            .on_press_maybe((!loading).then_some(Message::RefreshInfo)),
        button(text("Copy All").size(13)).padding([8, 16]).style(theme::secondary_button).on_press(Message::CopyInfo),
    ]
    .spacing(8);

    scrollable(
        column(cards)
            .push(container(buttons).center_x(Fill))
            .spacing(10)
            .padding(4),
    )
    .into()
}

// ── Performance: Health Check ────────────────────────────────────────────────

pub enum HealthState<'a> {
    Idle,
    Scanning,
    Failed(&'a str),
    Done { findings: &'a [Finding], scanned_at: &'a str },
}

fn fix_button(fix: &Fix, busy: bool, compact: bool) -> Element<'static, Message> {
    let style = if matches!(fix, Fix::Uninstall(_)) { theme::danger_button } else { theme::secondary_button };
    let mut b = button(text(fix.label()).size(12))
        .padding([5, 12])
        .style(style)
        .on_press_maybe((!busy).then(|| Message::FixPressed(fix.clone())));
    if compact {
        b = b.width(90);
    }
    b.into()
}

fn finding_card(f: &Finding, busy: bool) -> Element<'_, Message> {
    let color = match f.severity {
        Severity::Bad => theme::DANGER,
        Severity::Warn => theme::WARNING,
        Severity::Ok => theme::SUCCESS,
        Severity::Info => theme::PRIMARY,
    };
    let header = row![text("●").size(14).color(color), text(f.title).size(14).color(theme::TEXT), space::horizontal()]
        .extend(f.fixes.iter().map(|fix| fix_button(fix, busy, false)))
        .spacing(8)
        .align_y(Alignment::Center);

    let items = f.items.iter().map(|(label, fixes)| {
        row![text(label).size(12).color(theme::TEXT).width(Fill)]
            .extend(fixes.iter().map(|fix| fix_button(fix, busy, true)))
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
    });

    container(
        column![header, text(&f.detail).size(12).color(theme::TEXT_MUTED)]
            .push(column(items).spacing(4).padding([0, 12]))
            .spacing(6),
    )
    .padding(12)
    .width(Fill)
    .style(theme::card)
    .into()
}

pub fn performance(health: HealthState<'_>, busy: bool) -> Element<'_, Message> {
    let scanning = matches!(health, HealthState::Scanning);
    let header = row![
        section_title("Health Check"),
        space::horizontal(),
        button(text(if scanning { "Scanning…" } else { "Run Health Check" }).size(13))
            .padding([8, 16])
            .style(theme::primary_button)
            .on_press_maybe((!scanning && !busy).then_some(Message::RunHealthCheck)),
    ]
    .align_y(Alignment::Center);

    let (status, color, findings): (String, _, &[Finding]) = match health {
        HealthState::Idle => (
            "Scans memory, storage, CPU and apps. Read-only: nothing on the TV changes unless you press a fix button."
                .into(),
            theme::TEXT_DISABLED,
            &[],
        ),
        HealthState::Scanning => ("Scanning the TV (about 10 seconds)…".into(), theme::TEXT_DISABLED, &[]),
        HealthState::Failed(e) => (format!("Scan failed: {e}"), theme::DANGER, &[]),
        HealthState::Done { findings, scanned_at } => {
            let problems = findings.iter().filter(|f| matches!(f.severity, Severity::Bad | Severity::Warn)).count();
            let lead = if problems > 0 { format!("{problems} thing(s) worth fixing. ") } else { "Looks healthy. ".into() };
            (
                format!("{lead}Scanned {scanned_at}. Nothing changes unless you press a fix button."),
                theme::TEXT_DISABLED,
                findings,
            )
        }
    };

    let panel = container(
        column![header, text(status).size(12).color(color)]
            .push(column(findings.iter().map(|f| finding_card(f, busy))).spacing(8))
            .spacing(10),
    )
    .padding(16)
    .width(Fill)
    .style(theme::panel);

    let tool = |fix: Fix, style: fn(&iced::Theme, button::Status) -> button::Style| -> Element<'static, Message> {
        button(text(fix.label()).size(13).center().width(Fill))
            .padding([8, 12])
            .style(style)
            .on_press_maybe((!busy).then(|| Message::FixPressed(fix)))
            .into()
    };
    let tools_card = |title: &'static str, buttons: Vec<Element<'static, Message>>| {
        container(column![section_title(title), column(buttons).spacing(6)].spacing(10))
            .padding(16)
            .width(Fill)
            .style(theme::panel)
    };
    let optimizations = tools_card(
        "Optimizations",
        vec![
            tool(Fix::CompileApps, theme::primary_button),
            tool(Fix::EnableFreezer, theme::secondary_button),
            tool(Fix::AllOptimizations, theme::secondary_button),
            text("Compiling takes several minutes.").size(11).color(theme::WARNING).into(),
        ],
    );
    let memory = tools_card(
        "Animations & Memory",
        vec![
            tool(Fix::Animations("0.5"), theme::secondary_button),
            tool(Fix::Animations("0"), theme::secondary_button),
            tool(Fix::Animations("1"), theme::secondary_button),
            tool(Fix::ClearCaches, theme::secondary_button),
            tool(Fix::KillBackground, theme::danger_button),
        ],
    );
    let status: Element<'_, Message> = if busy {
        text("Working on the TV… (compiling can take several minutes)").size(12).color(theme::WARNING).into()
    } else {
        space().into()
    };

    scrollable(column![panel, status, row![optimizations, memory].spacing(8)].spacing(8).padding(4)).into()
}

// ── Packages ─────────────────────────────────────────────────────────────────

pub struct PackagesView<'a> {
    pub kind: Option<ListKind>,
    pub packages: &'a [Package],
    pub filter: &'a str,
    pub selected: &'a BTreeSet<String>,
    pub loading: bool,
    pub busy: bool,
    pub sdk: u32,
}

/// Packages matching the filter (by id or name, case-insensitive).
pub fn visible<'a>(packages: &'a [Package], filter: &str) -> impl Iterator<Item = &'a Package> {
    let q = filter.trim().to_lowercase();
    packages
        .iter()
        .filter(move |p| q.is_empty() || p.id.to_lowercase().contains(&q) || p.name.to_lowercase().contains(&q))
}

pub fn packages(v: PackagesView<'_>) -> Element<'_, Message> {
    let chips = row(ListKind::ALL.iter().map(|&kind| {
        button(text(kind.label()).size(12))
            .padding([5, 12])
            .style(theme::tab_button(v.kind == Some(kind)))
            .on_press_maybe((!v.loading).then_some(Message::ListPackages(kind)))
            .into()
    }))
    .spacing(4);
    let top = row![
        chips,
        space::horizontal(),
        text_input("Filter packages…", v.filter).on_input(Message::PackageFilter).size(13).width(200),
    ]
    .align_y(Alignment::Center)
    .spacing(8);

    let shown: Vec<&Package> = visible(v.packages, v.filter).collect();
    let list: Element<'_, Message> = if v.loading {
        empty_state("Loading packages…", "")
    } else if v.kind.is_none() {
        empty_state("No package list loaded", "Choose a list above, e.g. All or User.")
    } else if v.packages.is_empty() {
        empty_state("No packages in this list", "")
    } else {
        let header = row![
            text("").width(24),
            text("Package").size(11).color(theme::TEXT_MUTED).width(Fill),
            text("App Name").size(11).color(theme::TEXT_MUTED).width(220),
            text("Version").size(11).color(theme::TEXT_MUTED).width(140),
        ]
        .padding([0, 10]);
        let rows = shown.iter().map(|p| {
            let selected = v.selected.contains(&p.id);
            let name = if p.disabled { format!("{} (disabled)", p.name) } else { p.name.clone() };
            let line = row![
                text(if selected { "☑" } else { "☐" }).size(13).width(24),
                text(p.id.clone()).size(12).font(Font::MONOSPACE).width(Fill),
                text(name).size(12).color(if p.disabled { theme::TEXT_DISABLED } else { theme::TEXT }).width(220),
                text(p.version.clone()).size(12).font(Font::MONOSPACE).color(theme::TEXT_MUTED).width(140),
            ]
            .align_y(Alignment::Center);
            button(line)
                .padding([3, 10])
                .width(Fill)
                .style(theme::tab_button(selected))
                .on_press(Message::TogglePackage(p.id.clone()))
                .into()
        });
        column![header, scrollable(column(rows).spacing(1)).height(Fill)].spacing(4).into()
    };
    let list = container(list).padding(8).width(Fill).height(Fill).style(theme::log_box);

    let n = v.selected.len();
    let can_act = n > 0 && !v.busy && !v.loading;
    let action = |a: Action| -> Element<'_, Message> {
        let cache_unsupported = a == Action::ClearCache && v.sdk < packages::CACHE_ONLY_API;
        let style = if a.destructive() { theme::danger_button } else { theme::secondary_button };
        let b = button(text(a.label()).size(12))
            .padding([6, 12])
            .style(style)
            .on_press_maybe((can_act && !cache_unsupported).then_some(Message::PackageAction(a)));
        if cache_unsupported {
            tooltip(
                b,
                container(text("Needs Android 14 or newer. Use Clear All App Caches in Performance instead.").size(11))
                    .padding(8)
                    .style(theme::card),
                tooltip::Position::Top,
            )
            .into()
        } else {
            b.into()
        }
    };
    let actions = row(Action::ALL.iter().map(|&a| action(a)))
        .push(
            button(text("Save APK").size(12))
                .padding([6, 12])
                .style(theme::primary_button)
                .on_press_maybe(can_act.then_some(Message::SaveApk)),
        )
        .spacing(4);
    let status = if v.busy { "Working…".to_owned() } else { format!("{n} selected · {} shown", shown.len()) };
    let selection = row![
        text(status).size(12).color(theme::TEXT_MUTED),
        space::horizontal(),
        button(text("Select shown").size(12))
            .padding([4, 10])
            .style(theme::tab_button(false))
            .on_press_maybe((!shown.is_empty()).then_some(Message::SelectShown)),
        button(text("Clear").size(12))
            .padding([4, 10])
            .style(theme::tab_button(false))
            .on_press_maybe((n > 0).then_some(Message::ClearSelection)),
    ]
    .align_y(Alignment::Center)
    .spacing(4);

    column![top, list, selection, actions].spacing(8).padding(4).into()
}

// ── Install ──────────────────────────────────────────────────────────────────

pub struct InstallView<'a> {
    pub abis: &'a [String],
    /// Names of apps (catalog names, or "Shizuku") currently downloading.
    pub downloading: &'a BTreeSet<&'static str>,
    pub busy: bool,
}

fn card<'a>(title: &'a str, body: Element<'a, Message>) -> Element<'a, Message> {
    container(column![section_title(title), body].spacing(10))
        .padding(16)
        .width(Fill)
        .style(theme::panel)
        .into()
}

fn small_button(label: &str, style: fn(&iced::Theme, button::Status) -> button::Style, msg: Option<Message>) -> Element<'_, Message> {
    button(text(label).size(12)).padding([5, 12]).style(style).on_press_maybe(msg).into()
}

fn app_cell<'a>(spec: &'static AppSpec, v: &InstallView<'a>) -> Element<'a, Message> {
    let downloading = v.downloading.contains(spec.name);
    let build = catalog::build_for(spec, v.abis);
    let downloaded = catalog::download_path(spec, v.abis).is_some_and(|p| p.is_file());
    let status = match (&build, downloaded) {
        (Err(e), _) if !v.abis.is_empty() => e.clone(),
        (Err(_), _) => "Connect to pick the build for your TV's CPU.".into(),
        (Ok((_, _, note)), true) => format!("Downloaded{}", note.as_ref().map(|n| format!(" ({n})")).unwrap_or_default()),
        (Ok(_), false) => String::new(),
    };
    let download_label = if downloading { "Downloading…" } else if downloaded { "Update" } else { "Download" };
    let buttons = row![
        small_button(download_label, theme::secondary_button,
                     (!downloading && build.is_ok()).then_some(Message::QuickDownload(spec.name))),
        small_button("Install", theme::primary_button,
                     (downloaded && !downloading && !v.busy).then_some(Message::QuickInstall(spec.name))),
        text(status).size(11).color(theme::TEXT_DISABLED),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    container(
        column![
            button(text(spec.name).size(13).color(theme::PRIMARY))
                .padding(0)
                .style(|_, _| button::Style::default())
                .on_press(Message::OpenUrl(spec.homepage)),
            text(spec.description).size(11).color(theme::TEXT_MUTED),
            buttons,
        ]
        .spacing(6),
    )
    .padding(12)
    .width(Fill)
    .style(theme::card)
    .into()
}

pub fn install(v: InstallView<'_>) -> Element<'_, Message> {
    let apk = card(
        "Install APK",
        column![
            text("Pick APK files, or a folder of them. Play Protect's install checks are paused during the install and put back as they were afterwards.")
                .size(12)
                .color(theme::TEXT_MUTED),
            row![
                small_button("Install APK files…", theme::primary_button, (!v.busy).then_some(Message::PickApks)),
                small_button("Install folder…", theme::secondary_button, (!v.busy).then_some(Message::PickApkFolder)),
                text(if v.busy { "Installing…" } else { "" }).size(12).color(theme::WARNING),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        ]
        .spacing(8)
        .into(),
    );

    let cells: Vec<Element<'_, Message>> = catalog::APPS.iter().map(|spec| app_cell(spec, &v)).collect();
    let mut grid = column![].spacing(8);
    let mut cells = cells.into_iter();
    while let Some(left) = cells.next() {
        let right = cells.next().unwrap_or_else(|| space().width(Fill).into());
        grid = grid.push(row![left, right].spacing(8));
    }
    let quick = card("Quick Install Popular Apps", grid.into());

    let shizuku_downloading = v.downloading.contains(SHIZUKU.name);
    let shizuku_ready = catalog::download_path(&SHIZUKU, &[]).is_some_and(|p| p.is_file());
    let shizuku = card(
        "Shizuku",
        column![
            text(SHIZUKU.description).size(12).color(theme::TEXT_MUTED),
            row![
                small_button(if shizuku_downloading { "Downloading…" } else if shizuku_ready { "Update Shizuku" } else { "Download Shizuku" },
                             theme::secondary_button, (!shizuku_downloading).then_some(Message::QuickDownload(SHIZUKU.name))),
                small_button("Install & Start", theme::primary_button,
                             (shizuku_ready && !v.busy).then_some(Message::ShizukuInstall)),
                small_button("Start Shizuku", theme::secondary_button, (!v.busy).then_some(Message::ShizukuStart)),
            ]
            .spacing(6),
        ]
        .spacing(8)
        .into(),
    );

    scrollable(column![apk, quick, shizuku].spacing(8).padding(4)).into()
}

// ── Remote ───────────────────────────────────────────────────────────────────

pub fn remote<'a>() -> Element<'a, Message> {
    use crate::android::remote::*;
    let b = |key: Key, style: fn(&iced::Theme, button::Status) -> button::Style| -> Element<'a, Message> {
        button(text(key.label).size(15).center().width(Fill).height(Fill))
            .width(100)
            .height(48)
            .style(style)
            .on_press(Message::RemoteKey(key))
            .into()
    };
    let key = |k: Key| b(k, theme::secondary_button);
    let gap = || -> Element<'a, Message> { space().width(100).height(48).into() };

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

/// Map a key press on the computer to a TV remote key (Remote tab only).
pub fn remote_key(event: iced::keyboard::Event) -> Option<Message> {
    use crate::android::remote::*;
    use iced::keyboard::{Event, Key as K, key::Named};
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
    Some(Message::RemoteKey(tv))
}

// ── Screenshot ───────────────────────────────────────────────────────────────

pub fn screenshot<'a>(shot: Option<&'a iced::widget::image::Handle>, status: &'a str, busy: bool) -> Element<'a, Message> {
    let buttons = row![
        button(text(if busy { "Capturing…" } else { "Take Screenshot" }).size(13))
            .padding([8, 16])
            .style(theme::primary_button)
            .on_press_maybe((!busy).then_some(Message::TakeScreenshot)),
        button(text("Save As…").size(13))
            .padding([8, 16])
            .style(theme::secondary_button)
            .on_press_maybe(shot.is_some().then_some(Message::SaveScreenshot)),
        text(status).size(12).color(theme::TEXT_MUTED),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let view: Element<'a, Message> = match shot {
        Some(handle) => iced::widget::image(handle.clone()).content_fit(iced::ContentFit::Contain).width(Fill).height(Fill).into(),
        None => empty_state("No screenshot yet", "Press Take Screenshot to capture what's on the TV."),
    };
    column![buttons, container(view).padding(8).width(Fill).height(Fill).style(theme::panel)]
        .spacing(8)
        .padding(4)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::android::remote;
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
        match remote_key(event) {
            Some(Message::RemoteKey(k)) => Some(k.code),
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

// ── Settings ─────────────────────────────────────────────────────────────────

fn act(label: &str, action: TvAction, busy: bool) -> Element<'_, Message> {
    let style: fn(&iced::Theme, button::Status) -> button::Style =
        if action.confirmation().is_some() { theme::danger_button } else { theme::secondary_button };
    button(text(label).size(12))
        .padding([6, 12])
        .style(style)
        .on_press_maybe((!busy).then_some(Message::TvAction(action)))
        .into()
}

fn current_value(value: Option<&str>) -> Element<'_, Message> {
    text(format!("Now: {}", value.unwrap_or("…"))).size(12).color(theme::TEXT_MUTED).width(170).into()
}

fn setting_row<'a>(label: &'a str, now: Option<&'a str>, controls: Element<'a, Message>) -> Element<'a, Message> {
    row![
        text(label).size(13).color(theme::TEXT).width(150),
        current_value(now),
        controls,
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .into()
}

pub fn settings<'a>(current: Option<&'a Current>, density: &'a str, busy: bool) -> Element<'a, Message> {
    let now = |f: fn(&Current) -> &String| current.map(|c| f(c).as_str());
    let rotation_labels: Vec<&'static str> = actions::ROTATIONS.iter().map(|(l, _)| *l).collect();
    let timeout_labels: Vec<&'static str> = actions::TIMEOUTS.iter().map(|(l, _)| *l).collect();
    let pick_rotation = iced::widget::pick_list(rotation_labels, None::<&'static str>, |label| {
        let v = actions::ROTATIONS.iter().find(|(l, _)| *l == label).map(|(_, v)| *v).unwrap_or("0");
        Message::TvAction(TvAction::Rotation(v))
    })
    .placeholder("Change…")
    .text_size(12)
    .width(200);
    let pick_timeout = iced::widget::pick_list(timeout_labels, None::<&'static str>, |label| {
        let v = actions::TIMEOUTS.iter().find(|(l, _)| *l == label).map(|(_, v)| *v).unwrap_or("600000");
        Message::TvAction(TvAction::ScreenTimeout(v))
    })
    .placeholder("Change…")
    .text_size(12)
    .width(200);
    let density_controls = row![
        text_input("e.g. 320", density).on_input(Message::DensityInput).on_submit(Message::ApplyDensity).size(12).width(90),
        button(text("Apply").size(12)).padding([6, 12]).style(theme::secondary_button)
            .on_press_maybe((!busy).then_some(Message::ApplyDensity)),
        act("Reset", TvAction::ResetDensity, busy),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    let pair = |on: TvAction, off: TvAction| -> Element<'a, Message> {
        row![act("Turn on", on, busy), act("Turn off", off, busy)].spacing(6).into()
    };

    let display = card(
        "Display",
        column![
            setting_row("Rotation", now(|c| &c.rotation), pick_rotation.into()),
            setting_row("Screen timeout", now(|c| &c.timeout), pick_timeout.into()),
            setting_row("Density", now(|c| &c.density), density_controls.into()),
        ]
        .spacing(10)
        .into(),
    );
    let system = card(
        "System",
        column![
            setting_row("Developer options", now(|c| &c.developer_options),
                        row![act("Show", TvAction::DeveloperOptions(true), busy), act("Hide", TvAction::DeveloperOptions(false), busy)].spacing(6).into()),
            setting_row("Location", now(|c| &c.location), pair(TvAction::Location(true), TvAction::Location(false))),
            setting_row("Play Protect", now(|c| &c.play_protect), pair(TvAction::PlayProtect(true), TvAction::PlayProtect(false))),
            setting_row("Ambient display", now(|c| &c.ambient), pair(TvAction::AmbientDisplay(true), TvAction::AmbientDisplay(false))),
            setting_row("Time server", now(|c| &c.ntp), act("Use pool.ntp.org", TvAction::RepairNtp, busy)),
        ]
        .spacing(10)
        .into(),
    );
    let refresh = button(text("Refresh").size(12)).padding([6, 12]).style(theme::tab_button(false)).on_press(Message::LoadSettings);
    scrollable(column![display, system, refresh].spacing(8).padding(4)).into()
}

// ── Tools ────────────────────────────────────────────────────────────────────

pub struct ToolsView<'a> {
    pub launch: &'a str,
    pub text: &'a str,
    pub console: &'a str,
    pub busy: bool,
}

pub fn tools<'a>(v: ToolsView<'a>) -> Element<'a, Message> {
    let busy = v.busy;
    let col = |items: Vec<Element<'static, Message>>| column(items).spacing(6).width(Fill);
    let power = card(
        "Power",
        col(vec![
            act("Wake", TvAction::Wake, busy),
            act("Sleep / stand-by", TvAction::Sleep, busy),
            act("Reboot", TvAction::Reboot, busy),
            act("Reboot to recovery", TvAction::RebootRecovery, busy),
        ])
        .into(),
    );
    let navigation = card(
        "Navigation",
        col(vec![
            act("Open notifications", TvAction::Notifications, busy),
            act("Google search (weather)", TvAction::SearchWeather, busy),
            act("System updates", TvAction::SystemUpdates, busy),
        ])
        .into(),
    );
    let shortcuts = card(
        "Open on the TV",
        col(actions::SETTINGS_SHORTCUTS.iter().map(|(label, a)| act(label, TvAction::OpenSettings(a), busy)).collect())
            .into(),
    );

    let input_row = |placeholder: &'a str, value: &'a str, on_input: fn(String) -> Message, submit: Message, label: &'a str| -> Element<'a, Message> {
        row![
            text_input(placeholder, value).on_input(on_input).on_submit(submit.clone()).size(13),
            button(text(label).size(12)).padding([6, 14]).style(theme::secondary_button)
                .on_press_maybe((!busy && !value.trim().is_empty()).then_some(submit)),
        ]
        .spacing(6)
        .align_y(Alignment::Center)
        .into()
    };
    let inputs = card(
        "Apps, Text & Console",
        column![
            text("Launch an app by package name").size(12).color(theme::TEXT_MUTED),
            input_row("com.example.app", v.launch, Message::LaunchInput, Message::LaunchSubmit, "Launch"),
            text("Type text on the TV (into the focused field)").size(12).color(theme::TEXT_MUTED),
            input_row("Text to send…", v.text, Message::SendTextInput, Message::SendTextSubmit, "Send"),
            text("Shell command (an 'adb shell' prefix is fine); output goes to the log").size(12).color(theme::TEXT_MUTED),
            input_row("input keyevent 3", v.console, Message::ConsoleInput, Message::ConsoleSubmit, "Run"),
        ]
        .spacing(6)
        .into(),
    );
    scrollable(column![row![power, navigation, shortcuts].spacing(8), inputs].spacing(8).padding(4)).into()
}

// ── Media ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recording {
    Idle,
    Recording,
    Stopping,
}

pub struct MediaView<'a> {
    pub recording: Recording,
    pub scrcpy_found: bool,
    pub cwd: &'a str,
    pub path_input: &'a str,
    pub entries: Option<&'a Result<Vec<Entry>, String>>,
    pub selected: Option<&'a str>,
    pub busy: bool,
}

pub fn media(v: MediaView<'_>) -> Element<'_, Message> {
    let scrcpy = card(
        "scrcpy: mirror and control the TV",
        column![
            text(if v.scrcpy_found {
                "scrcpy shows the TV's screen in a window on your computer, with mouse and keyboard control."
            } else {
                "scrcpy wasn't found. Download it, then put it next to this app or on your PATH."
            })
            .size(12)
            .color(theme::TEXT_MUTED),
            row![
                small_button("Launch scrcpy", theme::primary_button, v.scrcpy_found.then_some(Message::LaunchScrcpy)),
                small_button("Download scrcpy", theme::secondary_button,
                             Some(Message::OpenUrl("https://github.com/Genymobile/scrcpy/releases/latest"))),
            ]
            .spacing(6),
        ]
        .spacing(8)
        .into(),
    );

    let (status, color) = match v.recording {
        Recording::Idle => ("Not recording. Recordings stop by themselves after 3 minutes.", theme::TEXT_DISABLED),
        Recording::Recording => ("● Recording…", theme::DANGER),
        Recording::Stopping => ("Finishing the file…", theme::WARNING),
    };
    let recording = card(
        "Screen Recording",
        column![
            text(status).size(12).color(color),
            row![
                small_button("Start Recording", theme::primary_button,
                             (v.recording == Recording::Idle).then_some(Message::RecordStart)),
                small_button("Stop & Save", theme::danger_button,
                             (v.recording == Recording::Recording).then_some(Message::RecordStop)),
            ]
            .spacing(6),
        ]
        .spacing(8)
        .into(),
    );

    let nav = row![
        text_input("/sdcard/", v.path_input).on_input(Message::FilesPathInput).on_submit(Message::FilesGo).size(12),
        small_button("Go", theme::secondary_button, Some(Message::FilesGo)),
        small_button("Up", theme::secondary_button, (v.cwd != "/").then_some(Message::FilesUp)),
        small_button("↻", theme::secondary_button, Some(Message::FilesRefresh)),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    let listing: Element<'_, Message> = match v.entries {
        None => text("Loading…").size(12).color(theme::TEXT_MUTED).into(),
        Some(Err(e)) => text(e.as_str()).size(12).color(theme::DANGER).into(),
        Some(Ok(list)) if list.is_empty() => text("(empty folder)").size(12).color(theme::TEXT_DISABLED).into(),
        Some(Ok(list)) => column(list.iter().map(|e| {
            let full = format!("{}{}", v.cwd, e.name);
            let selected = v.selected == Some(full.as_str());
            let label = if e.is_dir { format!("📁 {}/", e.name) } else { e.name.clone() };
            button(text(label).size(12).color(if e.is_dir { theme::PRIMARY } else { theme::TEXT }))
                .padding([3, 10])
                .width(Fill)
                .style(theme::tab_button(selected))
                .on_press(Message::FileClicked(e.name.clone(), e.is_dir))
                .into()
        }))
        .spacing(1)
        .into(),
    };
    let target = v.selected.unwrap_or(v.cwd);
    let files = card(
        "Files on the TV",
        column![
            nav,
            container(scrollable(listing).height(260)).padding(6).width(Fill).style(theme::log_box),
            text(format!("Selected: {target}  (click a file to select it; with nothing selected, the whole folder)"))
                .size(11)
                .color(theme::TEXT_MUTED),
            row![
                small_button("Download to PC…", theme::primary_button, (!v.busy).then_some(Message::FilesDownload)),
                small_button("Send file here…", theme::secondary_button, (!v.busy).then_some(Message::FilesSend)),
                text(if v.busy { "Working…" } else { "" }).size(12).color(theme::WARNING),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        ]
        .spacing(8)
        .into(),
    );

    scrollable(column![scrcpy, recording, files].spacing(8).padding(4)).into()
}
