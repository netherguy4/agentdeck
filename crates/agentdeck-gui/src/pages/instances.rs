//! One row per registry instance: identity, T3 health, quota bars.

use i18n_embed_fl::fl;
use iced::widget::{button, column};
use iced::{Element, Length};

use crate::app::{Message, Page, RemoveAsk};
use crate::data::{Data, QuotaBar, UsageQuotas};
use crate::i18n::loader;
use crate::style as st;

fn account_status(status: &str) -> String {
    match status {
        "ok" => fl!(loader(), "account-ok"),
        "stale" => fl!(loader(), "account-stale"),
        "error" => fl!(loader(), "account-error"),
        "waiting" => fl!(loader(), "account-waiting"),
        _ => status.to_string(),
    }
}

/// "resets in 2h 15m" / "resets in 40m"; the countdown math lives in data.
fn reset_label(mins: i64) -> String {
    if mins >= 24 * 60 {
        let d: i64 = mins / (24 * 60);
        let h: i64 = mins % (24 * 60) / 60;
        fl!(loader(), "quota-in-days", d = d, h = h)
    } else if mins >= 60 {
        let h: i64 = mins / 60;
        let m: i64 = mins % 60;
        fl!(loader(), "quota-in-hours", h = h, m = m)
    } else {
        fl!(loader(), "quota-in-mins", m = mins.max(0))
    }
}

fn quota_row(bar: &QuotaBar) -> Element<'_, Message> {
    // Window labels stay as the provider data gives them; the scope marks a
    // per-model window so it is not confused with the shared one.
    let label = match &bar.scope {
        Some(scope) => format!("{} · {scope}", bar.label),
        None => bar.label.clone(),
    };
    // Fixed side columns keep every bar the same length.
    let reset = bar.resets_in_mins.map(reset_label).unwrap_or_default();
    iced::widget::row![
        st::muted(label).width(150),
        st::quota_bar(bar.used_percent as f32).length(Length::Fill),
        st::body(format!("{:.0}%", bar.used_percent)).width(44),
        st::muted(reset).width(170),
    ]
    .spacing(st::SPACE_S)
    .into()
}

fn quotas_view(u: &UsageQuotas) -> Element<'_, Message> {
    // Account header carries the name plus status; window labels belong to
    // the bar rows below, not mixed into this line.
    let mut col = column![].spacing(st::SPACE_XS);
    let header = if u.status.is_empty() {
        u.display.clone()
    } else {
        format!("{} · {}", u.display, account_status(&u.status))
    };
    col = col.push(st::body(header));
    if u.bars.is_empty() {
        match &u.error {
            Some(error) => col = col.push(st::muted(error.clone())),
            None => col = col.push(st::muted(fl!(loader(), "quotas-no-data"))),
        }
    } else {
        for bar in &u.bars {
            col = col.push(quota_row(bar));
        }
    }
    col.into()
}

fn instance_row(inst: &crate::data::InstanceRow) -> Element<'_, Message> {
    // Name and status share the headline; identity details stay muted below.
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
    let mut col = column![
        iced::widget::row![
            st::heading(inst.display_name.clone()),
            st::muted(state),
            button(st::body(fl!(loader(), "instances-remove")))
                .style(button::secondary)
                .on_press(Message::RemoveAsk(inst.id.clone())),
        ]
        .spacing(st::SPACE_S),
        st::muted(format!("{} · {} · {t3}", inst.id, inst.harness)),
    ]
    .spacing(st::SPACE_XS);
    for u in &inst.quotas {
        col = col.push(quotas_view(u));
    }
    col.into()
}

pub fn view<'a>(data: &'a Data, remove: Option<&'a RemoveAsk>) -> Element<'a, Message> {
    let mut col = column![
        iced::widget::row![
            st::title(fl!(loader(), "instances-title")),
            button(st::body(fl!(loader(), "instances-add")))
                .style(button::secondary)
                .on_press(Message::SelectPage(Page::AddInstance)),
        ]
        .spacing(st::SPACE_M),
    ]
    .spacing(st::SPACE_M);
    if let Some(error) = &data.error {
        col = col.push(st::muted(fl!(loader(), "load-error", what = error)));
    }
    if !data.loaded {
        col = col.push(st::muted(fl!(loader(), "instances-loading")));
    } else if data.instances.is_empty() {
        col = col.push(st::muted(fl!(loader(), "instances-empty")));
    }
    for (i, inst) in data.instances.iter().enumerate() {
        if i > 0 {
            col = col.push(st::divider());
        }
        col = col.push(instance_row(inst));
    }
    if !data.standalone.is_empty() {
        if !data.instances.is_empty() {
            col = col.push(st::divider());
        }
        col = col.push(st::heading(fl!(loader(), "instances-standalone")));
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
    let mut lines: Vec<Element<'_, Message>> = Vec::new();
    if let Some(n) = ask.threads.filter(|&n| n > 0) {
        lines.push(st::muted(fl!(loader(), "remove-threads", count = n)).into());
    }
    lines.push(st::muted(fl!(loader(), "remove-keep-home")).into());
    if let Some(error) = &ask.error {
        lines.push(st::muted(fl!(loader(), "load-error", what = error)).into());
    }
    st::panel(
        fl!(loader(), "remove-ask", id = ask.id.clone()),
        lines,
        vec![
            // Destructive: the only danger button on this page.
            button(st::body(fl!(loader(), "remove-confirm")))
                .style(button::danger)
                .on_press_maybe((!ask.working).then_some(Message::RemoveConfirm))
                .into(),
            button(st::body(fl!(loader(), "remove-cancel")))
                .style(button::secondary)
                .on_press(Message::RemoveCancel)
                .into(),
        ],
    )
}
