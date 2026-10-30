//! Elm shell: sidebar pages, 5 s refresh tick, reads off the UI thread.

use std::time::Duration;

use i18n_embed_fl::fl;
use iced::widget::{button, column, container, scrollable, text};
use iced::{Element, Length, Subscription, Task, Theme};

use crate::data::{self, Data};
use crate::i18n::loader;
use crate::pages::plugins::Pending;

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
    EnablePlugin(String),
    DisablePlugin(String),
    CancelPlugin,
    ConfirmPlugin,
    PluginDone(Result<String, String>),
}

pub struct State {
    page: Page,
    data: Data,
    paths: Option<agentdeck_core::Paths>,
    pending: Option<Pending>,
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
                pending: None,
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
            pending: None,
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
            state.pending = None;
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

pub fn subscription(_state: &State) -> Subscription<Message> {
    iced::time::every(Duration::from_secs(5)).map(|_| Message::Tick)
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
        Page::Plugins => crate::pages::plugins::view(&state.data, state.pending.as_ref()),
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
