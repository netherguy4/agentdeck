//! Elm shell: sidebar pages, 5 s refresh tick, reads off the UI thread.

use std::time::Duration;

use i18n_embed_fl::fl;
use iced::widget::{button, column, container, scrollable, text};
use iced::{Element, Length, Subscription, Task, Theme};

use crate::data::{self, Data};
use crate::i18n::loader;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    Instances,
    Drift,
    Plugins,
    Processes,
}

impl Page {
    fn label(self) -> String {
        match self {
            Self::Instances => fl!(loader(), "nav-instances"),
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
    ProcStop(usize),
    ProcConfirm,
    ProcCancel,
    ProcForce(bool),
    ProcDone { title: String, result: String },
}

pub struct State {
    page: Page,
    data: Data,
    paths: Option<agentdeck_core::Paths>,
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
                paths,
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
            paths: Some(paths),
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
            Task::none()
        }
        Message::Tick => match state.paths.clone() {
            Some(paths) => Task::perform(async move { data::load(&paths) }, Message::Loaded),
            None => Task::none(),
        },
        Message::Loaded(data) => {
            state.data = data;
            Task::none()
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
    }
}

pub fn subscription(state: &State) -> Subscription<Message> {
    // The Processes page shows live resources; everything else keeps 5 s.
    let secs = if state.page == Page::Processes { 2 } else { 5 };
    iced::time::every(Duration::from_secs(secs)).map(|_| Message::Tick)
}

fn sidebar(state: &State) -> Element<'_, Message> {
    let mut col = column![].spacing(4).width(180);
    for page in [Page::Instances, Page::Drift, Page::Plugins, Page::Processes] {
        // The default button style is the accent fill, so non-selected pages
        // need the quiet variant explicitly.
        let mut btn = button(text(page.label()).size(14))
            .width(Length::Fill)
            .style(button::secondary)
            .on_press(Message::SelectPage(page));
        if state.page == page {
            btn = btn.style(button::primary);
        }
        col = col.push(btn);
    }
    container(col).padding(16).into()
}

pub fn view(state: &State) -> Element<'_, Message> {
    let content: Element<'_, Message> = match state.page {
        Page::Instances => crate::pages::instances::view(&state.data),
        Page::Drift => crate::pages::other::drift(),
        Page::Plugins => crate::pages::other::plugins(),
        Page::Processes => crate::pages::processes::view(&state.data, &state.proc_ui),
    };
    iced::widget::row![
        sidebar(state),
        scrollable(container(content).padding(20)).width(Length::Fill),
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
