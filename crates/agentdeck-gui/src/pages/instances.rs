//! One row per registry instance: identity, T3 health, quota bars.

use i18n_embed_fl::fl;
use iced::widget::{Space, button, column, progress_bar, text};
use iced::{Element, Length};

use crate::app::{Message, Page, RemoveAsk};
use crate::data::{Data, UsageQuotas};
use crate::i18n::loader;

fn account_status(status: &str) -> String {
    match status {
        "ok" => fl!(loader(), "account-ok"),
        "stale" => fl!(loader(), "account-stale"),
        "error" => fl!(loader(), "account-error"),
        "waiting" => fl!(loader(), "account-waiting"),
        _ => status.to_string(),
    }
}

fn quotas_view(u: &UsageQuotas) -> Element<'_, Message> {
    let mut col = column![text(&u.display).size(14)].spacing(4);
    if u.bars.is_empty() {
        match &u.error {
            Some(error) => col = col.push(text(error).size(12)),
            None => col = col.push(text(fl!(loader(), "quotas-no-data")).size(12)),
        }
    } else {
        if !u.status.is_empty() {
            col = col.push(text(account_status(&u.status)).size(12));
        }
        for bar in &u.bars {
            let label = match &bar.scope {
                Some(scope) => format!("{} · {scope}", bar.label),
                None => bar.label.clone(),
            };
            col = col.push(
                iced::widget::row![
                    text(label).size(12).width(160),
                    progress_bar(0.0..=100.0, bar.used_percent as f32).length(Length::Fill),
                    text(format!("{:.0}%", bar.used_percent)).size(12).width(44),
                ]
                .spacing(8),
            );
        }
    }
    col.into()
}

pub fn view<'a>(data: &'a Data, remove: Option<&'a RemoveAsk>) -> Element<'a, Message> {
    let mut col = column![
        iced::widget::row![
            text(fl!(loader(), "instances-title")).size(22),
            button(text(fl!(loader(), "instances-add")).size(13))
                .style(button::secondary)
                .on_press(Message::SelectPage(Page::AddInstance)),
        ]
        .spacing(12),
    ]
    .spacing(12);
    if let Some(error) = &data.error {
        col = col.push(text(fl!(loader(), "load-error", what = error)).size(13));
    }
    if data.instances.is_empty() {
        col = col.push(text(fl!(loader(), "instances-empty")).size(13));
    }
    for inst in &data.instances {
        let state = if inst.enabled {
            fl!(loader(), "status-enabled")
        } else {
            fl!(loader(), "status-disabled")
        };
        let t3 = match &inst.t3 {
            Some(info) => {
                let mut line = format!(
                    "{} {}",
                    fl!(loader(), "t3-label"),
                    info.status.as_deref().unwrap_or("—")
                );
                if let Some(n) = info.threads {
                    line.push_str(&format!(" · {}", fl!(loader(), "threads-count", count = n)));
                }
                line
            }
            None => fl!(loader(), "t3-unavailable"),
        };
        let mut card = column![
            iced::widget::row![
                text(&inst.display_name).size(16),
                text(&inst.id).size(12),
                text(&inst.harness).size(12),
                text(state).size(12),
                button(text(fl!(loader(), "instances-remove")).size(12))
                    .style(button::secondary)
                    .on_press(Message::RemoveAsk(inst.id.clone())),
            ]
            .spacing(8),
            text(t3).size(12),
        ]
        .spacing(4);
        if !inst.quotas.is_empty() {
            card = card.push(text(fl!(loader(), "instances-quotas")).size(13));
            for u in &inst.quotas {
                card = card.push(quotas_view(u));
            }
        }
        col = col.push(card);
        col = col.push(Space::new().height(Length::Fixed(8.0)));
    }
    if !data.standalone.is_empty() {
        col = col.push(text(fl!(loader(), "instances-standalone")).size(16));
        for u in &data.standalone {
            col = col.push(quotas_view(u));
        }
    }
    if let Some(ask) = remove {
        col = col.push(remove_panel(ask));
    }
    col.into()
}

fn remove_panel(ask: &RemoveAsk) -> Element<'_, Message> {
    let mut col =
        column![text(fl!(loader(), "remove-ask", id = ask.id.clone())).size(16)].spacing(6);
    if let Some(n) = ask.threads.filter(|&n| n > 0) {
        col = col.push(text(fl!(loader(), "remove-threads", count = n)).size(12));
    }
    col = col.push(text(fl!(loader(), "remove-keep-home")).size(12));
    if let Some(error) = &ask.error {
        col = col.push(text(fl!(loader(), "load-error", what = error)).size(12));
    }
    col = col.push(
        iced::widget::row![
            button(text(fl!(loader(), "remove-confirm")).size(13))
                .style(button::primary)
                .on_press_maybe((!ask.working).then_some(Message::RemoveConfirm)),
            button(text(fl!(loader(), "remove-cancel")).size(13))
                .style(button::secondary)
                .on_press(Message::RemoveCancel),
        ]
        .spacing(8),
    );
    col.into()
}
