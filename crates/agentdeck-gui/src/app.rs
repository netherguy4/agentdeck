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
}

pub struct State {
    page: Page,
    data: Data,
    paths: Option<agentdeck_core::Paths>,
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
            },
            match first {
                Some(paths) => {
                    Task::perform(async move { data::load(&paths) }, Message::Loaded)
                }
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
    }
}

pub fn subscription(_state: &State) -> Subscription<Message> {
    iced::time::every(Duration::from_secs(5)).map(|_| Message::Tick)
}

fn sidebar(state: &State) -> Element<'_, Message> {
    let mut col = column![].spacing(4).width(180);
    for page in [
        Page::Instances,
        Page::Drift,
        Page::Plugins,
        Page::Processes,
    ] {
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
