//! Processes page: agent-process tree grouped by owning session, with
//! resources and an owner-approved stop (confirmation lists every process).

use agentdeck_core::processes::{ProcInfo, SessionGroup, StopOutcome};
use i18n_embed_fl::fl;
use iced::widget::{button, checkbox, column, text};
use iced::{Element, Length};

use crate::app::Message;
use crate::data::Data;
use crate::i18n::loader;

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
    let mut col = column![text(fl!(loader(), "processes-title")).size(22)].spacing(12);
    if let Some(result) = &ui.result {
        col = col.push(text(result).size(13));
    }
    if data.processes.is_empty() {
        col = col.push(text(fl!(loader(), "processes-empty")).size(13));
    }
    for (i, g) in data.processes.iter().enumerate() {
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
            iced::widget::row![
                text(&g.title).size(16),
                text(kind_label(g)).size(12),
                text(totals).size(12).width(Length::Fill),
            ]
            .spacing(8)
        ]
        .spacing(4);
        for p in &g.processes {
            card = card.push(text(member_line(p)).size(12));
        }
        if g.can_stop {
            card = card.push(
                button(text(fl!(loader(), "processes-stop")).size(13))
                    .style(button::danger)
                    .on_press(Message::ProcStop(i)),
            );
        } else if !g.can_stop {
            card = card.push(text(fl!(loader(), "processes-t3-note")).size(12));
        }
        col = col.push(card);
    }
    if let Some(pending) = &ui.pending {
        let mut dlg = column![
            text(fl!(
                loader(),
                "processes-confirm-title",
                title = pending.title.as_str()
            ))
            .size(16),
            text(fl!(loader(), "processes-confirm-will")).size(13),
        ]
        .spacing(6);
        for p in &pending.members {
            dlg = dlg.push(text(format!("{}  {}", p.pid, p.display_cmd())).size(12));
        }
        dlg = dlg.push(
            checkbox(ui.force)
                .label(fl!(loader(), "processes-force"))
                .on_toggle(Message::ProcForce),
        );
        dlg = dlg.push(
            iced::widget::row![
                button(text(fl!(loader(), "processes-confirm-ok")).size(13))
                    .style(button::danger)
                    .on_press(Message::ProcConfirm),
                button(text(fl!(loader(), "processes-cancel")).size(13))
                    .style(button::secondary)
                    .on_press(Message::ProcCancel),
            ]
            .spacing(8),
        );
        col = col.push(dlg);
    }
    col.into()
}
