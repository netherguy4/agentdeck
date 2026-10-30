//! Elm shell: sidebar pages, 5 s refresh tick, reads off the UI thread.

use std::time::Duration;

use i18n_embed_fl::fl;
use iced::widget::{button, column, container, scrollable, text};
use iced::{Element, Length, Subscription, Task, Theme};

use crate::data::{self, Data};
use crate::i18n::loader;
use crate::pages::add_instance::{self, AddMsg};

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

pub struct State {
    page: Page,
    data: Data,
    paths: Option<agentdeck_core::Paths>,
    add: add_instance::Wizard,
    remove: Option<RemoveAsk>,
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
                add: add_instance::Wizard::default(),
                remove: None,
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
            add: add_instance::Wizard::default(),
            remove: None,
        }
    }

    pub fn page(&self) -> Page {
        self.page
    }
}

pub fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::SelectPage(page) => {
            state.page = page;
            state.remove = None;
            if page == Page::AddInstance {
                state.add = add_instance::Wizard::default();
            }
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
        Message::Noop => Task::none(),
    }
}

pub fn subscription(state: &State) -> Subscription<Message> {
    Subscription::batch([
        iced::time::every(Duration::from_secs(5)).map(|_| Message::Tick),
        add_instance::subscription(&state.add),
    ])
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
        Page::Instances => crate::pages::instances::view(&state.data, state.remove.as_ref()),
        Page::AddInstance => crate::pages::add_instance::view(&state.add),
        Page::Drift => crate::pages::other::drift(),
        Page::Plugins => crate::pages::other::plugins(),
        Page::Processes => crate::pages::other::processes(),
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
