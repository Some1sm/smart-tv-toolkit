//! Tab contents. Each function renders from plain data so `main.rs` only
//! wires state and messages.

use iced::widget::{button, column, container, progress_bar, row, scrollable, space, text};
use iced::{Alignment, Element, Fill};

use crate::Message;
use crate::android::health::{Finding, Fix, Severity};
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

    let later = container(
        text("The other optimizations (compile apps, animation speed, cache and background-app tools) arrive in milestone 4.")
            .size(12)
            .color(theme::TEXT_DISABLED),
    )
    .padding([4, 16]);

    scrollable(column![panel, later].spacing(8).padding(4)).into()
}
