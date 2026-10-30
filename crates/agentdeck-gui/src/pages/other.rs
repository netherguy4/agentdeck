//! Placeholder pages; later units fill them in.

use i18n_embed_fl::fl;
use iced::widget::{column, text};
use iced::Element;

use crate::app::Message;
use crate::i18n::loader;

fn placeholder<'a>(title: String) -> Element<'a, Message> {
    column![
        text(title).size(22),
        text(fl!(loader(), "coming-soon")).size(13),
    ]
    .spacing(12)
    .into()
}

pub fn drift<'a>() -> Element<'a, Message> {
    placeholder(fl!(loader(), "drift-title"))
}

pub fn plugins<'a>() -> Element<'a, Message> {
    placeholder(fl!(loader(), "plugins-title"))
}

pub fn processes<'a>() -> Element<'a, Message> {
    placeholder(fl!(loader(), "processes-title"))
}
