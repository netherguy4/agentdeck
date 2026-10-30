//! Plugins page: one card per built-in plugin with enable/disable and a
//! confirm step showing the [`Change`] preview before anything is applied.

use i18n_embed_fl::fl;
use iced::Element;
use iced::widget::{button, column};

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

pub fn view<'a>(data: &'a Data, pending: Option<&'a Pending>) -> Element<'a, Message> {
    let mut col = column![st::title(fl!(loader(), "plugins-title"))].spacing(st::SPACE_M);
    if let Some(error) = &data.error {
        col = col.push(st::muted(fl!(loader(), "load-error", what = error)));
    }
    if data.plugins.is_empty() {
        col = col.push(st::muted(fl!(loader(), "plugins-empty")));
    }
    for (i, plugin) in data.plugins.iter().enumerate() {
        if i > 0 {
            col = col.push(st::divider());
        }
        let state = if !plugin.available {
            fl!(loader(), "plugins-unavailable")
        } else if plugin.enabled {
            fl!(loader(), "plugins-enabled")
        } else {
            fl!(loader(), "plugins-disabled")
        };
        let mut card = column![
            iced::widget::row![st::heading(plugin.name.clone()), st::muted(state),]
                .spacing(st::SPACE_S),
        ]
        .spacing(st::SPACE_XS);
        if plugin.available {
            for line in &plugin.detail {
                card = card.push(st::muted(line.clone()));
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
                button(st::body(label))
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
        let lines: Vec<Element<'_, Message>> = p
            .preview
            .iter()
            .map(|l| st::body(l.clone()).into())
            .collect();
        col = col.push(st::panel(
            title,
            lines,
            vec![
                button(st::body(fl!(loader(), "plugins-confirm")))
                    .style(button::primary)
                    .on_press(Message::ConfirmPlugin)
                    .into(),
                button(st::body(fl!(loader(), "plugins-cancel")))
                    .style(button::secondary)
                    .on_press(Message::CancelPlugin)
                    .into(),
            ],
        ));
    }
    col.into()
}
