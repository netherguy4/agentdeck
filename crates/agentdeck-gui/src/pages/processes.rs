//! Processes page: agent-process tree grouped by owning session, with
//! resources and an owner-approved stop (confirmation lists every process).

use agentdeck_core::processes::{ProcInfo, SessionGroup, StopOutcome};
use i18n_embed_fl::fl;
use iced::Element;
use iced::widget::{button, checkbox, column};

use crate::app::Message;
use crate::data::Data;
use crate::i18n::loader;
use crate::style as st;

#[derive(Debug, Clone, Default)]
pub struct PendingStop {
    pub group: usize,
    pub title: String,
    pub members: Vec<ProcInfo>,
}

#[derive(Debug, Clone, Default)]
pub struct ProcUi {
    pub pending: Option<PendingStop>,
    pub force: bool,
    pub result: Option<String>,
}

fn mega(bytes: u64) -> String {
    format!("{:.0}M", bytes as f64 / 1_048_576.0)
}

fn maybe_mega(v: Option<u64>) -> String {
    v.map(mega).unwrap_or_else(|| "—".to_string())
}

fn kind_label(g: &SessionGroup) -> String {
    match &g.kind {
        agentdeck_core::processes::GroupKind::Agent { .. } => {
            fl!(loader(), "processes-kind-agent")
        }
        agentdeck_core::processes::GroupKind::T3 { .. } => fl!(loader(), "processes-kind-t3"),
        agentdeck_core::processes::GroupKind::Harness { .. } => {
            fl!(loader(), "processes-kind-harness")
        }
    }
}

pub fn describe_outcome(title: &str, outcome: &StopOutcome) -> String {
    let what = match outcome {
        StopOutcome::Terminated => fl!(loader(), "processes-stop-terminated"),
        StopOutcome::Killed => fl!(loader(), "processes-stop-killed"),
        StopOutcome::AlreadyExited => fl!(loader(), "processes-stop-exited"),
        StopOutcome::StillRunning => fl!(loader(), "processes-stop-running"),
    };
    fl!(
        loader(),
        "processes-stop-result",
        title = title,
        what = what
    )
}

fn member_line(p: &ProcInfo) -> String {
    format!(
        "{}  CPU {:.1}%  RSS {}  IO {}/{}  {}",
        p.pid,
        p.cpu_percent,
        mega(p.rss_bytes),
        maybe_mega(p.read_bytes),
        maybe_mega(p.write_bytes),
        p.display_cmd(),
    )
}

pub fn view<'a>(data: &'a Data, ui: &'a ProcUi) -> Element<'a, Message> {
    let mut col = column![st::title(fl!(loader(), "processes-title"))].spacing(st::SPACE_M);
    if let Some(result) = &ui.result {
        col = col.push(st::body(result.clone()));
    }
    if data.processes.is_empty() {
        col = col.push(st::muted(fl!(loader(), "processes-empty")));
    }
    for (i, g) in data.processes.iter().enumerate() {
        if i > 0 {
            col = col.push(st::divider());
        }
        let mut totals = format!(
            "CPU {:.1}% · {} · {}/{}",
            g.cpu_percent,
            mega(g.rss_bytes),
            maybe_mega(g.read_bytes),
            maybe_mega(g.write_bytes),
        );
        if let Some(mem) = g.scope_mem_bytes {
            totals.push_str(&format!(" · scope {}", mega(mem)));
        }
        let mut card = column![
            iced::widget::row![st::heading(g.title.clone()), st::muted(kind_label(g)),]
                .spacing(st::SPACE_S),
            st::muted(totals),
        ]
        .spacing(st::SPACE_XS);
        for p in &g.processes {
            card = card.push(st::muted(member_line(p)));
        }
        if g.can_stop {
            // Destructive: the only danger button on this page.
            card = card.push(
                button(st::body(fl!(loader(), "processes-stop")))
                    .style(button::danger)
                    .on_press(Message::ProcStop(i)),
            );
        } else if !g.can_stop {
            card = card.push(st::muted(fl!(loader(), "processes-t3-note")));
        }
        col = col.push(card);
    }
    if let Some(pending) = &ui.pending {
        let mut lines: Vec<Element<'_, Message>> =
            vec![st::body(fl!(loader(), "processes-confirm-will")).into()];
        for p in &pending.members {
            lines.push(st::muted(format!("{}  {}", p.pid, p.display_cmd())).into());
        }
        lines.push(
            checkbox(ui.force)
                .label(fl!(loader(), "processes-force"))
                .on_toggle(Message::ProcForce)
                .into(),
        );
        col = col.push(st::panel(
            fl!(
                loader(),
                "processes-confirm-title",
                title = pending.title.as_str()
            ),
            lines,
            vec![
                button(st::body(fl!(loader(), "processes-confirm-ok")))
                    .style(button::danger)
                    .on_press(Message::ProcConfirm)
                    .into(),
                button(st::body(fl!(loader(), "processes-cancel")))
                    .style(button::secondary)
                    .on_press(Message::ProcCancel)
                    .into(),
            ],
        ));
    }
    col.into()
}
