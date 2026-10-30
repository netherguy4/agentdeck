//! Shared visual language: spacing scale, three text sizes, muted
//! secondary text, thin quota bars, a light sidebar and one confirm-panel
//! shape used by every page. Only presentation: no behaviour lives here.

use iced::widget::{column, container, progress_bar, rule, text};
use iced::{Element, Length};

/// Spacing scale (px). Pages use these instead of ad-hoc numbers.
pub const SPACE_XS: f32 = 4.0;
pub const SPACE_S: f32 = 8.0;
pub const SPACE_M: f32 = 12.0;
pub const SPACE_L: f32 = 20.0;

/// The only three text sizes: page titles, group headings, everything else.
/// Hierarchy below the heading level comes from the muted secondary style.
pub const TITLE: f32 = 22.0;
pub const HEADING: f32 = 16.0;
pub const BODY: f32 = 13.0;

/// Sidebar width and thin quota-bar girth (px).
pub const SIDEBAR_WIDTH: f32 = 160.0;
pub const QUOTA_BAR_HEIGHT: f32 = 7.0;

/// Page title, e.g. "Instances".
pub fn title<'a>(label: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(label).size(TITLE)
}

/// Group or dialog heading, e.g. an instance name or "Preview".
pub fn heading<'a>(label: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(label).size(HEADING)
}

/// Primary line: labels, buttons, data the user must read.
pub fn body<'a>(label: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(label).size(BODY)
}

/// Secondary line: same size, theme-muted colour (works dark and light).
pub fn muted<'a>(label: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(label).size(BODY).style(text::secondary)
}

/// Thin quota bar: the default progress bar reads as a banner.
pub fn quota_bar(percent: f32) -> progress_bar::ProgressBar<'static> {
    progress_bar(0.0..=100.0, percent).girth(Length::Fixed(QUOTA_BAR_HEIGHT))
}

/// Restrained divider between groups and rows.
pub fn divider<'a, Message: 'a>() -> Element<'a, Message> {
    rule::horizontal(1).into()
}

/// One confirm/preview shape for every page: bordered box, heading, lines,
/// action row. Callers pass their own message elements.
pub fn panel<'a, Message: 'a>(
    heading_label: String,
    lines: Vec<Element<'a, Message>>,
    actions: Vec<Element<'a, Message>>,
) -> Element<'a, Message> {
    let mut col = column![heading(heading_label)].spacing(SPACE_S);
    for line in lines {
        col = col.push(line);
    }
    if !actions.is_empty() {
        let mut row = iced::widget::row![].spacing(SPACE_S);
        for action in actions {
            row = row.push(action);
        }
        col = col.push(row);
    }
    container(col)
        .padding(SPACE_S)
        .style(container::bordered_box)
        .into()
}
