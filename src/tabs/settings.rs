//! Settings: quick settings with their current values, a browser for every
//! system/secure/global setting on the TV, and shortcuts that open the TV's
//! own settings screens. Every change is recorded in the History.

use iced::widget::{button, checkbox, column, container, pick_list, row, scrollable, space, text_input, tooltip};
use iced::{Alignment, Element, Length, Padding, Task};

use super::{Ctx, View};
use crate::ui::text;
use crate::android::actions::{self, Action, Current};
use crate::android::changes::{self, NAMESPACES, Outcome};
use crate::android::settings_catalog::{self as catalog, Kind, Known};
use crate::design::{self as ds, style};
use crate::journal::{self, Change};
use crate::i18n::tr;
use crate::ui::{self, ButtonStyle};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Quick,
    All,
    Screens,
}

impl Mode {
    const ALL: [Mode; 3] = [Mode::Quick, Mode::All, Mode::Screens];

    fn label(self) -> &'static str {
        match self {
            Mode::Quick => "Quick Settings",
            Mode::All => "All Settings",
            Mode::Screens => "Open on TV",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Editing {
    namespace: String,
    key: String,
    draft: String,
}

#[derive(Debug, Default)]
pub struct State {
    mode: Mode,
    now: Option<Current>,
    density: String,
    busy: bool,
    /// Every setting on the TV, for the browser.
    all: Option<journal::State>,
    loading_all: bool,
    /// `None` shows every namespace.
    namespace: Option<&'static str>,
    search: String,
    known_only: bool,
    editing: Option<Editing>,
    /// New key form: namespace, key, value.
    new_key: (String, String),
}

#[derive(Debug, Clone)]
pub enum Msg {
    Mode(Mode),
    Load,
    Loaded(Result<Current, String>),
    Act(Action),
    Confirmed(Action, bool),
    Done(Result<Outcome, String>),
    DensityInput(String),
    ApplyDensity,
    LoadAll,
    LoadedAll(Result<journal::State, String>),
    Namespace(Option<&'static str>),
    Search(String),
    KnownOnly(bool),
    Edit(String, String, String),
    Draft(String),
    CancelEdit,
    /// Set `namespace/key` to a value (`None` deletes it).
    Set(String, String, Option<String>),
    SetConfirmed(String, String, Option<String>, bool),
    Saved(Result<Change, String>),
    NewKey(String),
    NewValue(String),
    AddKey,
    Open(&'static str),
}

/// One row of the browser.
struct Item<'a> {
    namespace: &'a str,
    key: &'a str,
    value: Option<&'a str>,
    known: Option<&'static Known>,
}

impl State {
    pub fn reset(&mut self) {
        *self = State::default();
    }

    /// Show live values whenever the tab is opened.
    pub fn on_open(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        match self.mode {
            Mode::All => self.load_all(ctx),
            _ => self.load(ctx),
        }
    }

    fn load(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        let Some(conn) = ctx.conn() else { return Task::none() };
        Task::perform(actions::read_current(conn), Msg::Loaded)
    }

    fn load_all(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        let Some(conn) = ctx.conn() else { return Task::none() };
        self.loading_all = true;
        Task::perform(changes::capture(conn), Msg::LoadedAll)
    }

    pub fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::Mode(mode) => {
                self.mode = mode;
                return match mode {
                    Mode::Quick => self.load(ctx),
                    Mode::All => self.load_all(ctx),
                    Mode::Screens => Task::none(),
                };
            }
            Msg::Load => return self.load(ctx),
            Msg::Loaded(result) => match result {
                Ok(now) if ctx.session.is_some() => self.now = Some(now),
                Ok(_) => {}
                Err(e) => ctx.log(format!("Could not read settings: {e}")),
            },
            Msg::Act(action) => match action.confirmation() {
                Some((title, question)) => return ui::confirm(title, question, move |yes| Msg::Confirmed(action, yes)),
                None => return self.update(Msg::Confirmed(action, true), ctx),
            },
            Msg::Confirmed(_, false) => {}
            Msg::Confirmed(action, true) => {
                let Some(conn) = ctx.conn() else { return Task::none() };
                self.busy = true;
                return Task::perform(actions::run(conn, action), Msg::Done);
            }
            Msg::Done(result) => {
                self.busy = false;
                match result {
                    Ok(outcome) => ctx.apply(outcome),
                    Err(e) => ctx.log(format!("Failed: {e}")),
                }
                return self.load(ctx);
            }
            Msg::DensityInput(v) => self.density = v,
            Msg::ApplyDensity => match self.density.trim().parse::<u32>() {
                Ok(d) if (120..=640).contains(&d) => return self.update(Msg::Act(Action::Density(d)), ctx),
                _ => ctx.log("Enter a density between 120 and 640 (TVs usually use 213, 240 or 320)."),
            },
            Msg::LoadAll => return self.load_all(ctx),
            Msg::LoadedAll(result) => {
                self.loading_all = false;
                match result {
                    Ok(state) if ctx.session.is_some() => self.all = Some(state),
                    Ok(_) => {}
                    Err(e) => ctx.log(format!("Could not read settings: {e}")),
                }
            }
            Msg::Namespace(ns) => self.namespace = ns,
            Msg::Search(q) => self.search = q,
            Msg::KnownOnly(on) => self.known_only = on,
            Msg::Edit(namespace, key, draft) => self.editing = Some(Editing { namespace, key, draft }),
            Msg::Draft(d) => {
                if let Some(e) = &mut self.editing {
                    e.draft = d;
                }
            }
            Msg::CancelEdit => self.editing = None,
            Msg::Set(namespace, key, value) => {
                let risky = catalog::is_risky(&namespace, &key);
                if risky || value.is_none() {
                    let what = match &value {
                        Some(v) => format!("Set {namespace}/{key} to \"{v}\"?"),
                        None => format!("Delete {namespace}/{key}?\n\nThe TV goes back to its built-in default for it."),
                    };
                    let warning = if risky { "\n\nThis setting can disconnect this app or affect the TV's setup." } else { "" };
                    let title = if value.is_some() { "Change setting" } else { "Delete setting" };
                    return ui::confirm(title, format!("{what}{warning}"), move |yes| Msg::SetConfirmed(namespace, key, value, yes));
                }
                return self.update(Msg::SetConfirmed(namespace, key, value, true), ctx);
            }
            Msg::SetConfirmed(_, _, _, false) => {}
            Msg::SetConfirmed(namespace, key, value, true) => {
                let Some(conn) = ctx.conn() else { return Task::none() };
                self.busy = true;
                self.editing = None;
                ctx.log.cmd(match &value {
                    Some(v) => format!("settings put {namespace} {key} {v}"),
                    None => format!("settings delete {namespace} {key}"),
                });
                return Task::perform(
                    async move { changes::put_setting(&conn, &namespace, &key, value.as_deref()).await },
                    Msg::Saved,
                );
            }
            Msg::Saved(result) => {
                self.busy = false;
                match result {
                    Ok(change) => ctx.apply(Outcome { lines: vec![change.describe()], changes: vec![change] }),
                    Err(e) => ctx.log(format!("Couldn't change the setting: {e}")),
                }
                return self.load_all(ctx);
            }
            Msg::NewKey(k) => self.new_key.0 = k,
            Msg::NewValue(v) => self.new_key.1 = v,
            Msg::AddKey => {
                let (key, value) = (self.new_key.0.trim().to_owned(), self.new_key.1.trim().to_owned());
                let namespace = self.namespace.unwrap_or("global").to_owned();
                if !changes::is_setting_key(&key) || value.is_empty() {
                    ctx.log("Enter a key (letters, digits and _ . - :) and a value.");
                    return Task::none();
                }
                self.new_key = Default::default();
                return self.update(Msg::Set(namespace, key, Some(value)), ctx);
            }
            Msg::Open(intent) => {
                let Some(conn) = ctx.conn() else { return Task::none() };
                return Task::perform(actions::run(conn, Action::OpenSettings(intent)), Msg::Done);
            }
        }
        Task::none()
    }

    pub fn view<'a>(&'a self, v: &View<'a>) -> Element<'a, Msg> {
        let modes = row(Mode::ALL.iter().map(|&m| {
            let active = m == self.mode;
            let caption = if active { ds::medium(m.label(), ds::TEXT_BASE, ds::TEXT) } else { ds::label(m.label(), ds::TEXT_BASE, ds::TEXT_SECONDARY) };
            button(caption).height(24).padding([0, 10]).style(style::pill(active)).on_press(Msg::Mode(m)).into()
        }))
        .spacing(4)
        .align_y(Alignment::Center);
        let mut bar = row![modes, space::horizontal()].spacing(8).align_y(Alignment::Center);
        if self.mode == Mode::All {
            bar = bar.push(self.all_toolbar());
        }
        let toolbar = container(column![container(bar).padding([0, 12]).center_y(Length::Fill), ds::hline(ds::BORDER)])
            .height(ds::TOOLBAR_HEIGHT)
            .style(style::fill(ds::SIDEBAR));
        let body = match self.mode {
            Mode::Quick => self.quick(),
            Mode::All => self.browser(v.journal, v.session.map(|s| s.address()).unwrap_or("")),
            Mode::Screens => self.screens(),
        };
        column![toolbar, container(body).height(Length::Fill)].into()
    }

    // ── Quick settings ───────────────────────────────────────────────────────

    fn quick(&self) -> Element<'_, Msg> {
        let busy = self.busy;
        let now = |f: fn(&Current) -> &String| self.now.as_ref().map(|c| f(c).as_str());
        let rotation_labels: Vec<&'static str> = actions::ROTATIONS.iter().map(|(l, _)| tr(l)).collect();
        let timeout_labels: Vec<&'static str> = actions::TIMEOUTS.iter().map(|(l, _)| tr(l)).collect();
        let pick_rotation = pick_list(rotation_labels, None::<&'static str>, |label| {
            let v = actions::ROTATIONS.iter().find(|(l, _)| tr(l) == label).map(|(_, v)| *v).unwrap_or("0");
            Msg::Act(Action::Rotation(v))
        })
        .style(style::pick_list)
        .menu_style(style::menu)
        .placeholder("Change…")
        .text_size(ds::TEXT_BASE)
        .width(200);
        let pick_timeout = pick_list(timeout_labels, None::<&'static str>, |label| {
            let v = actions::TIMEOUTS.iter().find(|(l, _)| tr(l) == label).map(|(_, v)| *v).unwrap_or("600000");
            Msg::Act(Action::ScreenTimeout(v))
        })
        .style(style::pick_list)
        .menu_style(style::menu)
        .placeholder("Change…")
        .text_size(ds::TEXT_BASE)
        .width(200);
        let density_controls = row![
            text_input("e.g. 320", &self.density)
                .on_input(Msg::DensityInput)
                .on_submit(Msg::ApplyDensity)
                .size(ds::TEXT_BASE)
                .padding([5, 8])
                .style(style::input(ds::INSET))
                .width(90),
            ui::small_button("Apply", style::standard, (!busy).then_some(Msg::ApplyDensity)),
            act("Reset", Action::ResetDensity, busy),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        let pair = |on: Action, off: Action, labels: (&'static str, &'static str)| -> Element<'_, Msg> {
            row![act(labels.0, on, busy), act(labels.1, off, busy)].spacing(6).into()
        };
        let onoff = ("Turn on", "Turn off");

        let display = ui::card(
            "Display",
            column![
                setting_row("Rotation", now(|c| &c.rotation), pick_rotation.into()),
                setting_row("Screen timeout", now(|c| &c.timeout), pick_timeout.into()),
                setting_row("Density", now(|c| &c.density), density_controls.into()),
            ]
            .spacing(10)
            .into(),
        );
        let system = ui::card(
            "System",
            column![
                setting_row(
                    "Developer options",
                    now(|c| &c.developer_options),
                    pair(Action::DeveloperOptions(true), Action::DeveloperOptions(false), ("Show", "Hide"))
                ),
                setting_row("Location", now(|c| &c.location), pair(Action::Location(true), Action::Location(false), onoff)),
                setting_row("Play Protect", now(|c| &c.play_protect), pair(Action::PlayProtect(true), Action::PlayProtect(false), onoff)),
                setting_row("Ambient display", now(|c| &c.ambient), pair(Action::AmbientDisplay(true), Action::AmbientDisplay(false), onoff)),
                setting_row("Time server", now(|c| &c.ntp), act("Use pool.ntp.org", Action::RepairNtp, busy)),
            ]
            .spacing(10)
            .into(),
        );
        let refresh = ds::icon_button(Some((ds::Icon::RefreshCw, ds::TEXT_SECONDARY)), "Refresh", None, style::standard, ds::CONTROL_HEIGHT, Some(Msg::Load));
        ui::page(column![display, system, refresh].spacing(12))
    }

    // ── All settings ─────────────────────────────────────────────────────────

    fn all_toolbar(&self) -> Element<'_, Msg> {
        let counts = |ns: Option<&str>| -> usize {
            self.all.as_ref().map_or(0, |a| a.settings.iter().filter(|(n, _)| ns.is_none_or(|x| x == n.as_str())).map(|(_, m)| m.len()).sum())
        };
        let pill = |label: &'static str, ns: Option<&'static str>| -> Element<'_, Msg> {
            let active = self.namespace == ns;
            let caption = if active { ds::medium(label, ds::TEXT_BASE, ds::TEXT) } else { ds::label(label, ds::TEXT_BASE, ds::TEXT_SECONDARY) };
            button(row![caption, ds::mono(counts(ns).to_string(), ds::TEXT_XS, ds::TEXT_TERTIARY)].spacing(4).align_y(Alignment::Center))
                .height(24)
                .padding([0, 10])
                .style(style::pill(active))
                .on_press(Msg::Namespace(ns))
                .into()
        };
        let search = container(
            row![
                ds::icon_any(ds::Icon::Search, 14.0, ds::TEXT_TERTIARY),
                ds::input("Search settings…", &self.search, false, ds::INSET).on_input(Msg::Search).style(style::bare_input).padding(Padding::from([5, 0])),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding(Padding::from([0, 10]))
        .width(200)
        .height(ds::CONTROL_HEIGHT)
        .style(style::panel(ds::INSET, ds::BORDER, ds::RADIUS));
        row![
            pill("All", None),
            pill("System", Some("system")),
            pill("Secure", Some("secure")),
            pill("Global", Some("global")),
            checkbox(self.known_only).label("Known only").on_toggle(Msg::KnownOnly).size(14).text_size(ds::TEXT_BASE).style(style::checkbox),
            ds::square_button(ds::Icon::RefreshCw, ds::TEXT_SECONDARY, ds::CONTROL_HEIGHT, style::ghost, (!self.loading_all).then_some(Msg::LoadAll)),
            search,
        ]
        .spacing(6)
        .align_y(Alignment::Center)
        .into()
    }

    /// Settings matching the filters: what the TV has, plus known keys it
    /// doesn't have yet (so they can be set).
    fn items(&self) -> Vec<Item<'_>> {
        let Some(all) = &self.all else { return Vec::new() };
        let q = self.search.trim().to_lowercase();
        let mut items: Vec<Item<'_>> = Vec::new();
        for ns in NAMESPACES {
            if self.namespace.is_some_and(|n| n != ns) {
                continue;
            }
            let values = all.settings.get(ns);
            for (key, value) in values.into_iter().flatten() {
                items.push(Item { namespace: ns, key, value: Some(value), known: catalog::find(ns, key) });
            }
            for k in catalog::KNOWN.iter().filter(|k| k.namespace == ns) {
                if values.is_none_or(|m| !m.contains_key(k.key)) {
                    items.push(Item { namespace: ns, key: k.key, value: None, known: Some(k) });
                }
            }
        }
        items.retain(|i| !self.known_only || i.known.is_some());
        if !q.is_empty() {
            items.retain(|i| {
                i.key.to_lowercase().contains(&q)
                    || i.value.is_some_and(|v| v.to_lowercase().contains(&q))
                    || i.known.is_some_and(|k| k.name.to_lowercase().contains(&q) || k.about.to_lowercase().contains(&q))
            });
        }
        // Known settings first (by name), then the rest by key.
        items.sort_by(|a, b| match (a.known, b.known) {
            (Some(x), Some(y)) => x.name.cmp(y.name),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => (a.namespace, a.key).cmp(&(b.namespace, b.key)),
        });
        items
    }

    fn browser<'a>(&'a self, journal: &'a journal::Journal, serial: &str) -> Element<'a, Msg> {
        if self.all.is_none() {
            let title = if self.loading_all { "Reading settings…" } else { "Settings not loaded" };
            return ui::empty_state(title, "Every system, secure and global setting on the TV.");
        }
        // Keys this app changed on this TV, to mark them in the list.
        let touched: Vec<String> = journal.touched_settings_since(serial, "");
        let items = self.items();
        let shown = items.len();
        let mut rows = column![];
        for (i, item) in items.into_iter().enumerate() {
            if i > 0 {
                rows = rows.push(ds::hline(ds::DIVIDER));
            }
            let changed = touched.contains(&format!("{}/{}", item.namespace, item.key));
            rows = rows.push(self.setting_item(item, changed));
        }
        let ns = self.namespace.unwrap_or("global");
        let add = row![
            ds::label(format!("Add a {ns} setting:"), ds::TEXT_SM, ds::TEXT_SECONDARY),
            ds::input("key", &self.new_key.0, true, ds::INSET).on_input(Msg::NewKey).width(240),
            ds::input("value", &self.new_key.1, true, ds::INSET).on_input(Msg::NewValue).on_submit(Msg::AddKey).width(160),
            ui::small_button("Add", style::standard, (!self.busy).then_some(Msg::AddKey)),
            space::horizontal(),
            ds::label(format!("{shown} shown"), ds::TEXT_SM, ds::TEXT_TERTIARY),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        column![
            scrollable(rows).direction(ds::thin_scrollbar()).style(style::scrollbar).height(Length::Fill),
            container(column![ds::hline(ds::BORDER), container(add).padding([8, 12])]).style(style::fill(ds::SIDEBAR)),
        ]
        .into()
    }

    fn setting_item<'a>(&'a self, item: Item<'a>, changed: bool) -> Element<'a, Msg> {
        let (ns, key) = (item.namespace.to_owned(), item.key.to_owned());
        let title: Element<'a, Msg> = match item.known {
            Some(k) => column![
                row![ds::medium(k.name, ds::TEXT_BASE, ds::TEXT), ds::mono(format!("{}/{}", item.namespace, item.key), ds::TEXT_XS, ds::TEXT_TERTIARY)]
                    .spacing(8)
                    .align_y(Alignment::Center),
                ds::label(k.about, ds::TEXT_SM, ds::TEXT_SECONDARY),
            ]
            .spacing(2)
            .into(),
            None => row![
                ds::mono(item.key, ds::TEXT_SM, ds::TEXT),
                container(ds::mono(item.namespace, ds::TEXT_XS, ds::TEXT_TERTIARY)).padding([0, 5]).style(style::count_badge(ds::BADGE)),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into(),
        };
        let editing = self.editing.as_ref().filter(|e| e.namespace == item.namespace && e.key == item.key);
        let busy = self.busy;
        let control: Element<'a, Msg> = match (editing, item.known.map(|k| k.kind)) {
            (Some(e), _) => row![
                ds::input("value", &e.draft, true, ds::INSET)
                    .on_input(Msg::Draft)
                    .on_submit(Msg::Set(ns.clone(), key.clone(), Some(e.draft.clone())))
                    .width(240),
                ui::small_button("Save", style::primary, (!busy).then(|| Msg::Set(ns.clone(), key.clone(), Some(e.draft.clone())))),
                ui::small_button("Cancel", style::standard, Some(Msg::CancelEdit)),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into(),
            (None, Some(Kind::Switch)) => {
                let on = item.value == Some("1");
                let off = item.value == Some("0");
                row![
                    toggle_button("On", on, (!busy && !on).then(|| Msg::Set(ns.clone(), key.clone(), Some("1".into())))),
                    toggle_button("Off", off, (!busy && !off).then(|| Msg::Set(ns.clone(), key.clone(), Some("0".into())))),
                ]
                .spacing(4)
                .into()
            }
            (None, Some(Kind::Choice(options))) => {
                let labels: Vec<&'static str> = options.iter().map(|(l, _)| tr(l)).collect();
                let current = options.iter().find(|(_, v)| Some(*v) == item.value).map(|(l, _)| tr(l));
                let (ns2, key2) = (ns.clone(), key.clone());
                pick_list(labels, current, move |label| {
                    let value = options.iter().find(|(l, _)| tr(l) == label).map(|(_, v)| (*v).to_owned());
                    Msg::Set(ns2.clone(), key2.clone(), value)
                })
                .placeholder(item.value.unwrap_or("not set"))
                .text_size(ds::TEXT_BASE)
                .style(style::pick_list)
                .menu_style(style::menu)
                .width(200)
                .into()
            }
            (None, _) => {
                let shown = catalog::describe(item.known, item.value);
                row![
                    ds::mono(shown, ds::TEXT_SM, if item.value.is_some() { ds::TEXT } else { ds::TEXT_TERTIARY })
                        .wrapping(iced::widget::text::Wrapping::None)
                        .width(Length::Fill),
                    ui::small_button("Edit", style::standard, (!busy).then(|| Msg::Edit(ns.clone(), key.clone(), item.value.unwrap_or("").to_owned()))),
                ]
                .spacing(8)
                .align_y(Alignment::Center)
                .width(300)
                .into()
            }
        };
        let mut right = row![].spacing(6).align_y(Alignment::Center);
        if changed {
            right = right.push(tooltip(
                ds::icon_any(ds::Icon::FileClock, 14.0, ds::ACCENT),
                container(ds::label("Changed by this app; see History to undo", ds::TEXT_SM, ds::TEXT)).padding([4, 8]).style(style::flyout),
                tooltip::Position::Left,
            ));
        }
        right = right.push(control);
        let delete = ds::square_button(
            ds::Icon::Trash,
            ds::TEXT_TERTIARY,
            24.0,
            style::ghost,
            (item.value.is_some() && !busy && editing.is_none()).then(|| Msg::Set(ns.clone(), key.clone(), None)),
        );
        row![container(title).width(Length::Fill).clip(true), right, delete]
            .spacing(10)
            .padding([8, 12])
            .align_y(Alignment::Center)
            .into()
    }

    // ── Open on TV ───────────────────────────────────────────────────────────

    fn screens(&self) -> Element<'_, Msg> {
        let mut grid = column![].spacing(8);
        for chunk in catalog::SCREENS.chunks(4) {
            let mut line = row![].spacing(8);
            for &(title, intent) in chunk {
                line = line.push(
                    button(
                        column![
                            ds::medium(title, ds::TEXT_BASE, ds::TEXT),
                            ds::mono(intent.trim_start_matches("android.settings."), ds::TEXT_XS, ds::TEXT_TERTIARY),
                        ]
                        .spacing(4),
                    )
                    .padding([10, 12])
                    .width(Length::Fill)
                    .style(style::standard)
                    .on_press(Msg::Open(intent)),
                );
            }
            for _ in chunk.len()..4 {
                line = line.push(space::horizontal());
            }
            grid = grid.push(line);
        }
        ui::page(
            column![
                ds::label(
                    "Opens the screen on the TV itself; use the Remote tab or the TV's remote to move around it. \
                     Some TVs don't have every screen.",
                    ds::TEXT_BASE,
                    ds::TEXT_SECONDARY,
                ),
                grid,
            ]
            .spacing(12),
        )
    }
}

fn toggle_button(label: &str, active: bool, msg: Option<Msg>) -> Element<'_, Msg> {
    button(container(text(label).size(ds::TEXT_SM)).center_y(Length::Fill))
        .height(24)
        .padding([0, 10])
        .style(style::pill(active))
        .on_press_maybe(msg)
        .into()
}

fn act(label: &str, action: Action, busy: bool) -> Element<'_, Msg> {
    let style: ButtonStyle = if action.confirmation().is_some() { style::danger } else { style::standard };
    ui::small_button(label, style, (!busy).then_some(Msg::Act(action)))
}

fn setting_row<'a>(label: &'a str, now: Option<&'a str>, controls: Element<'a, Msg>) -> Element<'a, Msg> {
    row![
        ds::label(label, ds::TEXT_BASE, ds::TEXT).width(150),
        ds::label(format!("Now: {}", now.unwrap_or("…")), ds::TEXT_BASE, ds::TEXT_SECONDARY).width(170),
        controls,
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .into()
}
