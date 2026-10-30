//! Plugins page: one card per built-in plugin with enable/disable and a
//! confirm step showing the [`Change`] preview before anything is applied.

use i18n_embed_fl::fl;
use iced::widget::{button, column, text};
use iced::{Element, Length};

use crate::app::Message;
use crate::data::Data;
use crate::i18n::loader;

/// A confirmed-but-not-yet-applied enable/disable with its change preview.
#[derive(Debug, Clone)]
pub struct Pending {
    pub name: String,
    pub enable: bool,
    pub preview: Vec<String>,
}

pub fn view<'a>(data: &'a Data, pending: Option<&'a Pending>) -> Element<'a, Message> {
    let mut col = column![text(fl!(loader(), "plugins-title")).size(22)].spacing(12);
    if let Some(error) = &data.error {
        col = col.push(text(fl!(loader(), "load-error", what = error)).size(13));
    }
    if data.plugins.is_empty() {
        col = col.push(text(fl!(loader(), "plugins-empty")).size(13));
    }
    for plugin in &data.plugins {
        let state = if !plugin.available {
            fl!(loader(), "plugins-unavailable")
        } else if plugin.enabled {
            fl!(loader(), "plugins-enabled")
        } else {
            fl!(loader(), "plugins-disabled")
        };
        let mut card = column![
            iced::widget::row![text(&plugin.name).size(16), text(state).size(12),].spacing(8),
        ]
        .spacing(4);
        if plugin.available {
            for line in &plugin.detail {
                card = card.push(text(line).size(12));
            }
            let (label, msg) = if plugin.enabled {
                (
                    format!("{} {}", fl!(loader(), "plugins-disable"), plugin.name),
                    Message::DisablePlugin(plugin.name.clone()),
                )
            } else {
                (
                    format!("{} {}", fl!(loader(), "plugins-enable"), plugin.name),
                    Message::EnablePlugin(plugin.name.clone()),
                )
            };
            card = card.push(
                button(text(label).size(13))
                    .style(button::secondary)
                    .on_press(msg),
            );
        }
        col = col.push(card);
    }
    if let Some(p) = pending {
        let title = if p.enable {
            fl!(loader(), "plugins-preview-enable", name = p.name.clone())
        } else {
            fl!(loader(), "plugins-preview-disable", name = p.name.clone())
        };
        let mut confirm = column![text(title).size(16)].spacing(4);
        for line in &p.preview {
            confirm = confirm.push(text(line).size(12));
        }
        confirm = confirm.push(
            iced::widget::row![
                button(text(fl!(loader(), "plugins-confirm")).size(13))
                    .style(button::primary)
                    .on_press(Message::ConfirmPlugin),
                button(text(fl!(loader(), "plugins-cancel")).size(13))
                    .style(button::secondary)
                    .on_press(Message::CancelPlugin),
            ]
            .spacing(8),
        );
        col = col.push(confirm.width(Length::Fill));
    }
    col.into()
}
