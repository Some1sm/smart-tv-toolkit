//! Tab contents. Each function renders from plain data so `main.rs` only
//! wires state and messages.

use std::collections::BTreeSet;

use iced::widget::{button, column, container, progress_bar, row, scrollable, space, text, text_input, tooltip};
use iced::{Alignment, Element, Fill, Font};

use crate::Message;
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
