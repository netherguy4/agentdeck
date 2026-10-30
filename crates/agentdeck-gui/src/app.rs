//! Elm shell: sidebar pages, 5 s refresh tick, reads off the UI thread.

use std::time::Duration;

use i18n_embed_fl::fl;
use iced::widget::{button, column, container, scrollable, text};
use iced::{Element, Length, Subscription, Task, Theme};

use crate::data::{self, Data};
use crate::i18n::loader;
use crate::pages::add_instance::{self, AddMsg};
use crate::pages::drift::{self, Ctx as DriftCtx};
use crate::pages::plugins::Pending;
use crate::style as st;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    Instances,
    AddInstance,
    Drift,
    Plugins,
    Processes,
}

impl Page {
    fn label(self) -> String {
        match self {
            Self::Instances => fl!(loader(), "nav-instances"),
            Self::AddInstance => fl!(loader(), "add-title"),
            Self::Drift => fl!(loader(), "nav-drift"),
            Self::Plugins => fl!(loader(), "nav-plugins"),
            Self::Processes => fl!(loader(), "nav-processes"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    Loaded(Data),
    SelectPage(Page),
    Add(AddMsg),
    RemoveAsk(String),
    RemoveCancel,
    RemoveConfirm,
    RemoveDone(Result<RemoveOutcome, String>),
    Noop,
    EnablePlugin(String),
    DisablePlugin(String),
    CancelPlugin,
    ConfirmPlugin,
    PluginDone(Result<String, String>),
    Drift(drift::Message),
    ProcStop(usize),
    ProcConfirm,
    ProcCancel,
    ProcForce(bool),
    ProcDone { title: String, result: String },
}

#[derive(Debug, Clone)]
pub struct RemoveOutcome {
    pub id: String,
    pub lines: Vec<String>,
    pub home: String,
}

#[derive(Debug, Clone, Default)]
pub struct RemoveAsk {
    pub id: String,
    pub threads: Option<u64>,
    pub error: Option<String>,
    pub working: bool,
}

fn drift_ctx(paths: &agentdeck_core::Paths) -> DriftCtx {
    DriftCtx {
        paths: paths.clone(),
        home: std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_default(),
        t3_root: agentdeck_core::t3::t3_root().ok(),
    }
}

pub struct State {
    page: Page,
    data: Data,
    drift: drift::State,
    paths: Option<agentdeck_core::Paths>,
    add: add_instance::Wizard,
    remove: Option<RemoveAsk>,
    pending: Option<Pending>,
    proc_ui: crate::pages::processes::ProcUi,
}

impl State {
    /// Production state; tests build their own via [`state_for_tests`].
    pub fn new() -> (Self, Task<Message>) {
        let paths = agentdeck_core::Paths::from_env().ok();
        let first = paths.clone();
        (
            Self {
                page: Page::Instances,
                data: Data::default(),
                drift: drift::State::default(),
                paths,
                add: add_instance::Wizard::default(),
                remove: None,
                pending: None,
                proc_ui: crate::pages::processes::ProcUi::default(),
            },
            match first {
                Some(paths) => Task::perform(async move { data::load(&paths) }, Message::Loaded),
                None => Task::none(),
            },
        )
    }

    /// Test seam: fixed paths and preloaded data, no async involved.
    pub fn for_tests(paths: agentdeck_core::Paths, data: Data) -> Self {
        Self {
            page: Page::Instances,
            data,
            drift: drift::State::default(),
            paths: Some(paths),
            add: add_instance::Wizard::default(),
            remove: None,
            pending: None,
            proc_ui: crate::pages::processes::ProcUi::default(),
        }
    }

    pub fn page(&self) -> Page {
        self.page
    }

    pub fn proc_ui(&self) -> &crate::pages::processes::ProcUi {
        &self.proc_ui
    }
}

pub fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::SelectPage(page) => {
            state.page = page;
            state.pending = None;
            state.remove = None;
            if page == Page::AddInstance {
                state.add = add_instance::Wizard::default();
            }
            Task::none()
        }
        Message::Tick => match state.paths.clone() {
            Some(paths) => {
                let drift_paths = paths.clone();
                Task::batch([
                    Task::perform(async move { data::load(&paths) }, Message::Loaded),
                    Task::perform(
                        async move {
                            let ctx = drift_ctx(&drift_paths);
                            drift::load(ctx)
                        },
                        Message::Drift,
                    ),
                ])
            }
            None => Task::none(),
        },
        Message::Loaded(data) => {
            state.data = data;
            Task::none()
        }
        Message::Add(msg) => match state.paths.clone() {
            Some(paths) => add_instance::update_wizard(&mut state.add, msg, &paths),
            None => Task::none(),
        },
        Message::RemoveAsk(id) => {
            // Read-only thread count for the confirmation; core refuses the
            // removal itself when pool/usage entries still reference the id.
            let threads = agentdeck_core::t3::t3_root()
                .ok()
                .and_then(|root| agentdeck_core::t3::thread_count(&root, &id).unwrap_or(None));
            state.remove = Some(RemoveAsk {
                id,
                threads,
                error: None,
                working: false,
            });
            Task::none()
        }
        Message::RemoveCancel => {
            state.remove = None;
            Task::none()
        }
        Message::RemoveConfirm => {
            let Some(ask) = state.remove.clone() else {
                return Task::none();
            };
            let id = ask.id.clone();
            match state.paths.clone() {
                Some(paths) => {
                    if let Some(current) = state.remove.as_mut() {
                        current.working = true;
                    }
                    Task::perform(
                        async move {
                            agentdeck_core::registry::remove_instance(&paths, &id, false)
                                .map(|(changes, home)| RemoveOutcome {
                                    id: id.clone(),
                                    lines: changes.iter().map(|c| c.describe()).collect(),
                                    home,
                                })
                                .map_err(|e| format!("{e:#}"))
                        },
                        Message::RemoveDone,
                    )
                }
                None => Task::none(),
            }
        }
        Message::RemoveDone(result) => {
            match result {
                Ok(outcome) => {
                    state.data.instances.retain(|i| i.id != outcome.id);
                    state.remove = None;
                }
                Err(e) => {
                    if let Some(ask) = state.remove.as_mut() {
                        ask.working = false;
                        ask.error = Some(e);
                    }
                }
            }
            match state.paths.clone() {
                Some(paths) => Task::perform(async move { data::load(&paths) }, Message::Loaded),
                None => Task::none(),
            }
        }
        Message::ProcStop(i) => {
            // Fresh identity-checked member list for the confirmation; the
            // snapshot rows are the fallback (e.g. the group just exited).
            let pending = state.data.processes.get(i).map(|g| {
                let members = agentdeck_core::processes::group_members(
                    &agentdeck_core::processes::proc_root(),
                    g.pid,
                    g.start_ticks,
                )
                .unwrap_or_else(|_| g.processes.clone());
                crate::pages::processes::PendingStop {
                    group: i,
                    title: g.title.clone(),
                    members,
                }
            });
            state.proc_ui.pending = pending;
            state.proc_ui.force = false;
            state.proc_ui.result = None;
            Task::none()
        }
        Message::ProcConfirm => match state.proc_ui.pending.clone() {
            // The owner's click is the approval: stop off the UI thread.
            Some(pending) => {
                let group = state.data.processes.get(pending.group).cloned();
                let force = state.proc_ui.force;
                state.proc_ui.pending = None;
                match group.filter(|g| g.can_stop) {
                    Some(g) => Task::perform(
                        async move {
                            let outcome = tokio::task::spawn_blocking(move || {
                                agentdeck_core::processes::stop_group(
                                    g.pid,
                                    g.start_ticks,
                                    std::time::Duration::from_secs(5),
                                    force,
                                )
                            })
                            .await
                            .map_err(|e| anyhow::anyhow!("{e}"))
                            .and_then(|r| r);
                            match outcome {
                                Ok(o) => {
                                    let result = crate::pages::processes::describe_outcome(
                                        &g.title, &o.outcome,
                                    );
                                    Message::ProcDone {
                                        title: g.title,
                                        result,
                                    }
                                }
                                Err(e) => Message::ProcDone {
                                    title: g.title.clone(),
                                    result: format!("{}: {e:#}", g.title),
                                },
                            }
                        },
                        |m| m,
                    ),
                    None => Task::none(),
                }
            }
            None => Task::none(),
        },
        Message::ProcCancel => {
            state.proc_ui.pending = None;
            Task::none()
        }
        Message::ProcForce(force) => {
            state.proc_ui.force = force;
            Task::none()
        }
        Message::ProcDone { title: _, result } => {
            state.proc_ui.result = Some(result);
            // Refresh behind the result line.
            match state.paths.clone() {
                Some(paths) => Task::perform(async move { data::load(&paths) }, Message::Loaded),
                None => Task::none(),
            }
        }
        Message::Noop => Task::none(),
        Message::Drift(inner) => match state.paths.clone() {
            Some(paths) => {
                drift::update(&mut state.drift, drift_ctx(&paths), inner).map(Message::Drift)
            }
            None => Task::none(),
        },
        Message::EnablePlugin(name) => {
            preview_plugin(state, &name, true);
            Task::none()
        }
        Message::DisablePlugin(name) => {
            preview_plugin(state, &name, false);
            Task::none()
        }
        Message::CancelPlugin => {
            state.pending = None;
            Task::none()
        }
        Message::ConfirmPlugin => match (state.paths.clone(), state.pending.clone()) {
            (Some(paths), Some(pending)) => {
                state.pending = None;
                Task::perform(
                    async move { apply_plugin(&paths, &pending) },
                    Message::PluginDone,
                )
            }
            _ => Task::none(),
        },
        Message::PluginDone(result) => {
            if let Some(paths) = state.paths.clone() {
                state.data = data::load(&paths);
            }
            if let Err(error) = result {
                state.data.error = Some(error);
            }
            Task::none()
        }
    }
}

/// Read-only preview of the enable/disable changes; failures surface as a
/// one-line pending preview instead of a popup.
fn preview_plugin(state: &mut State, name: &str, enable: bool) {
    let Some(paths) = state.paths.clone() else {
        return;
    };
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let exe = std::env::current_exe().unwrap_or_default();
    let files = agentdeck_core::plugins::default_instruction_files(&home);
    let planned = if enable {
        agentdeck_core::plugins::plan_enable(&paths, &home, &files, &exe, name, false)
    } else {
        agentdeck_core::plugins::plan_disable(&paths, &home, &files, &exe, name)
    };
    state.pending = Some(match planned {
        Ok(changes) => Pending {
            name: name.to_string(),
            enable,
            preview: changes.iter().map(|c| c.describe()).collect(),
        },
        Err(error) => Pending {
            name: name.to_string(),
            enable,
            preview: vec![format!("{error:#}")],
        },
    });
}

/// Apply a confirmed plugin change off the UI thread.
fn apply_plugin(paths: &agentdeck_core::Paths, pending: &Pending) -> Result<String, String> {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let exe = std::env::current_exe().unwrap_or_default();
    let files = agentdeck_core::plugins::default_instruction_files(&home);
    let planned = if pending.enable {
        agentdeck_core::plugins::plan_enable(paths, &home, &files, &exe, &pending.name, false)
    } else {
        agentdeck_core::plugins::plan_disable(paths, &home, &files, &exe, &pending.name)
    }
    .map_err(|e| format!("{e:#}"))?;
    agentdeck_core::plugins::apply_all(paths, &pending.name, pending.enable, &planned)
        .map_err(|e| format!("{e:#}"))?;
    Ok(if pending.enable {
        format!("enabled {}", pending.name)
    } else {
        format!("disabled {}", pending.name)
    })
}

pub fn subscription(state: &State) -> Subscription<Message> {
    // The Processes page shows live resources; everything else keeps 5 s.
    let secs = if state.page == Page::Processes { 2 } else { 5 };
    Subscription::batch([
        iced::time::every(Duration::from_secs(secs)).map(|_| Message::Tick),
        add_instance::subscription(&state.add),
    ])
}

fn sidebar(state: &State) -> Element<'_, Message> {
    // Light sidebar: plain text buttons, the selected page in accent fill.
    let mut col = column![].spacing(st::SPACE_XS).width(st::SIDEBAR_WIDTH);
    for page in [Page::Instances, Page::Drift, Page::Plugins, Page::Processes] {
        let mut btn = button(text(page.label()).size(st::BODY).width(Length::Fill))
            .width(Length::Fill)
            .style(button::text)
            .on_press(Message::SelectPage(page));
        if state.page == page {
            btn = btn.style(button::primary);
        }
        col = col.push(btn);
    }
    container(col).padding(st::SPACE_L).into()
}

pub fn view(state: &State) -> Element<'_, Message> {
    let content: Element<'_, Message> = match state.page {
        Page::Instances => crate::pages::instances::view(&state.data, state.remove.as_ref()),
        Page::AddInstance => crate::pages::add_instance::view(&state.add),
        Page::Drift => crate::pages::drift::view(&state.drift).map(Message::Drift),
        Page::Plugins => crate::pages::plugins::view(&state.data, state.pending.as_ref()),
        Page::Processes => crate::pages::processes::view(&state.data, &state.proc_ui),
    };
    iced::widget::row![
        sidebar(state),
        scrollable(container(content).padding(st::SPACE_L)).width(Length::Fill),
    ]
    .into()
}

/// `None` lets iced match the system color scheme (dark/light); it falls back
/// to its own default when the OS exposes nothing.
pub fn theme(_state: &State) -> Option<Theme> {
    None
}

pub fn title(_state: &State) -> String {
    fl!(loader(), "app-title")
}
