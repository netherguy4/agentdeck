//! Instances: one card per registry instance (identity, T3 pill, threads,
//! aligned quota bars) or, in the Console variant, one dense table.

use i18n_embed_fl::fl;
use iced::widget::{Space, column, row};
use iced::{Alignment, Element, Length};

use crate::app::{Message, RemoveAsk};
use crate::data::{Data, InstanceRow, QuotaBar, UsageQuotas};
use crate::i18n::loader;
use crate::overlay::Modal;
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

fn account_tone(status: &str) -> st::Tone {
    match status {
        "ok" => st::Tone::Ok,
        "error" => st::Tone::Danger,
        "stale" | "waiting" => st::Tone::Warn,
        _ => st::Tone::Neutral,
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
    // Fixed side columns keep every bar the same length across cards.
    let reset = bar.resets_in_mins.map(reset_label).unwrap_or_default();
    row![
        st::muted(label).width(150),
        st::quota_bar(bar.used_percent as f32).length(Length::Fill),
        st::data(format!("{:.0}%", bar.used_percent))
            .width(44)
            .align_x(Alignment::End),
        st::caption(reset).width(150),
    ]
    .spacing(st::SPACE_M)
    .align_y(Alignment::Center)
    .into()
}

fn quotas_view(u: &UsageQuotas) -> Element<'_, Message> {
    let look = st::look();
    let mut head = row![st::body(u.display.as_str())]
        .spacing(st::SPACE_S)
        .align_y(Alignment::Center);
    if !u.status.is_empty() {
        head = head.push(st::pill(account_status(&u.status), account_tone(&u.status)));
    }
    let mut col = column![head].spacing(look.row_gap);
    if u.bars.is_empty() {
        col = col.push(st::muted(match &u.error {
            Some(error) => error.clone(),
            None => fl!(loader(), "quotas-no-data"),
        }));
    }
    for bar in &u.bars {
        col = col.push(quota_row(bar));
    }
    col.into()
}

fn t3_pill(inst: &InstanceRow) -> Element<'_, Message> {
    match &inst.t3 {
        Some(info) => {
            let status = info.status.as_deref().unwrap_or("—");
            let tone = match status {
                "ready" => st::Tone::Ok,
                "disabled" | "—" => st::Tone::Neutral,
                _ => st::Tone::Warn,
            };
            st::pill(format!("{} {status}", fl!(loader(), "t3-label")), tone)
        }
        None => st::pill(fl!(loader(), "t3-unavailable"), st::Tone::Neutral),
    }
}

fn threads(inst: &InstanceRow) -> String {
    inst.t3
        .as_ref()
        .and_then(|i| i.threads)
        .map(|n| fl!(loader(), "threads-count", count = n))
        .unwrap_or_default()
}

fn remove_btn(inst: &InstanceRow) -> Element<'_, Message> {
    st::icon_btn(st::Icon::Trash)
        .on_press(Message::RemoveAsk(inst.id.clone()))
        .into()
}

fn instance_card(inst: &InstanceRow) -> Element<'_, Message> {
    let look = st::look();
    let mut head = row![
        st::heading(inst.display_name.as_str()),
        st::mono_muted(inst.id.as_str()),
        st::pill(inst.harness.as_str(), st::Tone::Accent),
    ]
    .spacing(st::SPACE_S)
    .align_y(Alignment::Center);
    if !inst.enabled {
        head = head.push(st::pill(fl!(loader(), "status-disabled"), st::Tone::Warn));
    }
    let head = head
        .push(Space::new().width(Length::Fill))
        .push(st::caption(threads(inst)))
        .push(t3_pill(inst))
        .push(remove_btn(inst));
    let mut col = column![head].spacing(look.row_gap + 4.0);
    for u in &inst.quotas {
        col = col.push(quotas_view(u));
    }
    st::card(col)
}

/// Console: one table, a line per instance with its quota lines below.
fn instance_table(instances: &[InstanceRow]) -> Element<'_, Message> {
    let header = row![
        st::caption(fl!(loader(), "col-name").to_uppercase()).width(Length::FillPortion(3)),
        st::caption(fl!(loader(), "col-id").to_uppercase()).width(Length::FillPortion(3)),
        st::caption(fl!(loader(), "col-harness").to_uppercase()).width(Length::FillPortion(2)),
        st::caption(fl!(loader(), "t3-label").to_uppercase()).width(Length::FillPortion(3)),
        st::caption(fl!(loader(), "col-threads").to_uppercase()).width(Length::FillPortion(2)),
        Space::new().width(28),
    ]
    .spacing(st::SPACE_S);
    let mut col = column![header, st::hairline()].spacing(4);
    for (i, inst) in instances.iter().enumerate() {
        if i > 0 {
            col = col.push(st::hairline());
        }
        col = col.push(
            row![
                st::data(inst.display_name.as_str()).width(Length::FillPortion(3)),
                st::mono_muted(inst.id.as_str()).width(Length::FillPortion(3)),
                st::mono(inst.harness.as_str()).width(Length::FillPortion(2)),
                iced::widget::container(t3_pill(inst)).width(Length::FillPortion(3)),
                st::mono_muted(threads(inst)).width(Length::FillPortion(2)),
                remove_btn(inst),
            ]
            .spacing(st::SPACE_S)
            .align_y(Alignment::Center),
        );
        for u in &inst.quotas {
            col = col.push(iced::widget::container(quotas_view(u)).padding([2, 16]));
        }
    }
    st::card(col)
}

pub fn view(data: &Data) -> Element<'_, Message> {
    let look = st::look();
    let mut col = column![].spacing(look.gap.max(st::SPACE_S));
    if let Some(error) = &data.error {
        col = col.push(st::muted(fl!(loader(), "load-error", what = error)));
    }
    if !data.loaded {
        col = col.push(st::muted(fl!(loader(), "instances-loading")));
    } else if data.instances.is_empty() {
        col = col.push(st::card(st::muted(fl!(loader(), "instances-empty"))));
    } else if look.table {
        col = col.push(instance_table(&data.instances));
    } else {
        for inst in &data.instances {
            col = col.push(instance_card(inst));
        }
    }
    if !data.standalone.is_empty() {
        let mut list = column![].spacing(look.row_gap + 4.0);
        for u in &data.standalone {
            list = list.push(quotas_view(u));
        }
        col = col.push(
            column![
                st::caption(fl!(loader(), "instances-standalone")),
                st::card(list)
            ]
            .spacing(st::SPACE_S),
        );
    }
    col.into()
}

pub fn remove_modal(ask: &RemoveAsk) -> Modal<'_, Message> {
    let mut body: Vec<Element<'_, Message>> = Vec::new();
    if let Some(n) = ask.threads.filter(|&n| n > 0) {
        body.push(st::body(fl!(loader(), "remove-threads", count = n)).into());
    }
    body.push(st::muted(fl!(loader(), "remove-keep-home")).into());
    if let Some(error) = &ask.error {
        body.push(st::body(fl!(loader(), "load-error", what = error)).into());
    }
    Modal {
        title: fl!(loader(), "remove-ask", id = ask.id.clone()),
        body,
        actions: vec![
            st::btn(st::Btn::Quiet, fl!(loader(), "remove-cancel"))
                .on_press(Message::RemoveCancel)
                .into(),
            // Destructive: the only red thing on screen.
            st::btn(st::Btn::Danger, fl!(loader(), "remove-confirm"))
                .on_press_maybe((!ask.working).then_some(Message::RemoveConfirm))
                .into(),
        ],
        on_cancel: Message::RemoveCancel,
        on_submit: None,
    }
}
