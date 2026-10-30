//! Harnesses page: detected version and install method per harness, with
//! Update / Install behind a confirmation that shows the exact official
//! command. Detection and the command itself run off the UI thread; the
//! outcome is plain result text.

use agentdeck_core::harness::{self, HarnessName, Status};
use i18n_embed_fl::fl;
use iced::widget::{button, column, row};
use iced::{Element, Task};

use crate::i18n::loader;
use crate::style as st;

#[derive(Debug, Clone)]
pub struct Pending {
    pub name: String,
    pub argv: Vec<String>,
    /// The exact command line shown in the panel.
    pub lines: Vec<String>,
}

#[derive(Default)]
pub struct State {
    pub rows: Vec<Status>,
    pub pending: Option<Pending>,
    pub busy: bool,
    pub result: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Loaded(Vec<Status>),
    Ask { name: String, install: bool },
    Confirm,
    Cancel,
    Done(String),
}

/// Runs `<bin> --version` per harness: call off the UI thread.
pub async fn load() -> Msg {
    Msg::Loaded(
        tokio::task::spawn_blocking(|| harness::list(&harness::real_runner))
            .await
            .unwrap_or_default(),
    )
}

pub fn update(state: &mut State, msg: Msg) -> Task<Msg> {
    match msg {
        Msg::Loaded(rows) => {
            state.rows = rows;
            Task::none()
        }
        Msg::Ask { name, install } => {
            state.result = None;
            let Ok(h) = HarnessName::parse(&name) else {
                return Task::none();
            };
            let binary = state
                .rows
                .iter()
                .find(|r| r.name == name)
                .and_then(|r| r.binary.as_ref())
                .map_or_else(|| name.clone(), |b| b.display().to_string());
            let argv = if install {
                h.install_argv()
            } else {
                h.update_argv(&binary)
            };
            state.pending = Some(Pending {
                name,
                lines: vec![
                    format!("+ {}", argv.join(" ")),
                    fl!(loader(), "harness-note"),
                ],
                argv,
            });
            Task::none()
        }
        Msg::Confirm => match state.pending.take() {
            Some(p) => {
                state.busy = true;
                Task::perform(
                    async move {
                        let name = p.name.clone();
                        let out = tokio::task::spawn_blocking(move || {
                            harness::real_runner(&p.argv[0], &p.argv[1..])
                        })
                        .await;
                        let what = match out {
                            Ok(Ok(text)) => last_lines(&text),
                            Ok(Err(e)) => format!("{e:#}"),
                            Err(e) => e.to_string(),
                        };
                        fl!(loader(), "harness-result", name = name, what = what)
                    },
                    Msg::Done,
                )
            }
            None => Task::none(),
        },
        Msg::Cancel => {
            state.pending = None;
            Task::none()
        }
        Msg::Done(text) => {
            state.busy = false;
            state.result = Some(text);
            Task::perform(load(), |m| m)
        }
    }
}

/// Installers are chatty: keep the last few lines.
fn last_lines(text: &str) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(4)..].join("\n")
}

pub fn view(state: &State) -> Element<'_, Msg> {
    let mut col = column![st::title(fl!(loader(), "harness-title"))].spacing(st::SPACE_M);
    for (i, r) in state.rows.iter().enumerate() {
        if i > 0 {
            col = col.push(st::divider());
        }
        let installed = r.binary.is_some();
        let (label, install) = if installed {
            (fl!(loader(), "harness-update"), false)
        } else {
            (fl!(loader(), "harness-install"), true)
        };
        let version = match (&r.version, installed) {
            (Some(v), _) => v.clone(),
            (None, true) => "—".to_string(),
            (None, false) => fl!(loader(), "harness-not-installed"),
        };
        col = col.push(
            row![
                column![
                    row![st::heading(r.name.clone()), st::muted(version)].spacing(st::SPACE_S),
                    st::muted(r.install_method.clone()),
                ]
                .spacing(st::SPACE_XS)
                .width(iced::Length::Fill),
                button(st::body(label))
                    .style(button::secondary)
                    .on_press_maybe((!state.busy).then(|| Msg::Ask {
                        name: r.name.clone(),
                        install,
                    })),
            ]
            .align_y(iced::Alignment::Center),
        );
    }
    if let Some(p) = &state.pending {
        col = col.push(st::confirm_panel(
            fl!(loader(), "harness-run-title"),
            &p.lines,
            (fl!(loader(), "action-apply"), Some(Msg::Confirm)),
            (fl!(loader(), "action-cancel"), Msg::Cancel),
        ));
        col = col.push(st::muted(fl!(loader(), "harness-note")));
    }
    if state.busy {
        col = col.push(st::muted(fl!(loader(), "action-working")));
    }
    if let Some(r) = &state.result {
        col = col.push(st::muted(r.clone()));
    }
    col.into()
}
