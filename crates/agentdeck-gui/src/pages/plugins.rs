//! Plugins page: one card per built-in plugin with an on/off toggle, its
//! technical status behind an expander, and a confirm step showing the [`Change`] preview before anything is applied.

use std::collections::HashSet;

use i18n_embed_fl::fl;
use iced::widget::{button, column, row, toggler};
use iced::{Alignment, Element};

use crate::app::Message;
use crate::data::Data;
use crate::i18n::loader;
use crate::style as st;

/// A confirmed-but-not-yet-applied enable/disable with its change preview.
#[derive(Debug, Clone)]
pub struct Pending {
    pub name: String,
    pub enable: bool,
    pub preview: Vec<String>,
}

pub fn modal(p: &Pending) -> crate::overlay::Modal<'_, Message> {
    let title = if p.enable {
        fl!(loader(), "plugins-preview-enable", name = p.name.clone())
    } else {
        fl!(loader(), "plugins-preview-disable", name = p.name.clone())
    };
    crate::overlay::confirm(
        title,
        &p.preview,
        (
            fl!(loader(), "plugins-confirm"),
            Some(Message::ConfirmPlugin),
        ),
        (fl!(loader(), "plugins-cancel"), Message::CancelPlugin),
    )
}

pub fn view<'a>(data: &'a Data, open: &'a HashSet<String>) -> Element<'a, Message> {
    let look = st::look();
    let mut col = column![].spacing(look.gap.max(st::SPACE_S));
    if let Some(error) = &data.error {
        col = col.push(st::card(st::muted(fl!(
            loader(),
            "load-error",
            what = error
        ))));
    }
    if data.plugins.is_empty() {
        col = col.push(st::card(st::muted(fl!(loader(), "plugins-empty"))));
    }
    for plugin in &data.plugins {
        let (state, tone) = if !plugin.available {
            (fl!(loader(), "plugins-unavailable"), st::Tone::Neutral)
        } else if plugin.enabled {
            (fl!(loader(), "plugins-enabled"), st::Tone::Ok)
        } else {
            (fl!(loader(), "plugins-disabled"), st::Tone::Neutral)
        };
        let name = plugin.name.clone();
        let switch = toggler(plugin.enabled)
            .size(16)
            .on_toggle_maybe(plugin.available.then_some(move |on| {
                if on {
                    Message::EnablePlugin(name.clone())
                } else {
                    Message::DisablePlugin(name.clone())
                }
            }));
        let expanded = open.contains(&plugin.name);
        let head = row![
            st::fit(plugin.name.as_str(), st::Role::Heading, st::Cut::End),
            st::pill(state, tone),
            switch,
        ]
        .spacing(st::SPACE_M)
        .align_y(Alignment::Center);
        let expander = button(
            row![
                st::icon(
                    if expanded {
                        st::Icon::ChevronDown
                    } else {
                        st::Icon::ChevronRight
                    },
                    look.body
                ),
                st::nowrap(st::caption(fl!(loader(), "plugins-details"))),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        )
        .padding([2, 4])
        .style(st::row_style)
        .on_press(Message::PluginDetail(plugin.name.clone()));
        let mut card = column![head, expander].spacing(look.row_gap);
        if expanded {
            let mut detail = column![].spacing(2).padding(iced::Padding {
                left: 20.0,
                ..iced::Padding::ZERO
            });
            if plugin.detail.is_empty() {
                detail = detail.push(st::muted(fl!(loader(), "plugins-no-details")));
            }
            for line in &plugin.detail {
                detail = detail.push(st::mono_muted(line.as_str()));
            }
            card = card.push(detail);
        }
        col = col.push(st::card(card));
    }
    col.into()
}
