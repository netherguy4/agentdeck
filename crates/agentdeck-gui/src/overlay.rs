//! Centered modal over a dimmed scrim and corner toasts. Esc/Enter routing
//! lives in app.rs: it reads `on_cancel`/`on_submit` off the open modal.

use std::time::{Duration, Instant};

use iced::widget::{Space, center, column, container, mouse_area, opaque, row, scrollable, stack};
use iced::{Alignment, Element, Length, Theme};

use crate::style as st;

pub struct Modal<'a, M> {
    pub title: String,
    pub body: Vec<Element<'a, M>>,
    pub actions: Vec<Element<'a, M>>,
    /// Esc, the scrim and the close icon.
    pub on_cancel: M,
    /// Enter; `None` for destructive dialogs so they need a click.
    pub on_submit: Option<M>,
}

impl<'a, M: 'a> Modal<'a, M> {
    pub fn map<N: 'a>(self, f: impl Fn(M) -> N + Clone + 'a) -> Modal<'a, N> {
        Modal {
            title: self.title,
            body: self.body.into_iter().map(|e| e.map(f.clone())).collect(),
            actions: self.actions.into_iter().map(|e| e.map(f.clone())).collect(),
            on_cancel: f(self.on_cancel),
            on_submit: self.on_submit.map(&f),
        }
    }
}

/// Preview/confirm dialog with a primary Apply (Enter) and Cancel (Esc).
pub fn confirm<'a, M: Clone + 'a>(
    title: String,
    lines: &'a [String],
    (apply_label, on_apply): (String, Option<M>),
    (cancel_label, on_cancel): (String, M),
) -> Modal<'a, M> {
    Modal {
        title,
        body: lines.iter().map(|l| st::body(l.as_str()).into()).collect(),
        actions: vec![
            st::btn(st::Btn::Quiet, cancel_label)
                .on_press(on_cancel.clone())
                .into(),
            st::btn(st::Btn::Primary, apply_label)
                .on_press_maybe(on_apply.clone())
                .into(),
        ],
        on_cancel,
        on_submit: on_apply,
    }
}

fn dialog<'a, M: Clone + 'a>(modal: Modal<'a, M>) -> Element<'a, M> {
    let look = st::look();
    let head = row![
        st::heading(modal.title).width(Length::Fill),
        st::icon_btn(st::Icon::X).on_press(modal.on_cancel.clone()),
    ]
    .align_y(Alignment::Center);
    let body = container(
        scrollable(column(modal.body).spacing(look.row_gap).padding([0, 12]))
            .spacing(4)
            .style(st::scroll_style),
    )
    .max_height(380);
    let actions = row![Space::new().width(Length::Fill)]
        .extend(modal.actions)
        .spacing(st::SPACE_S)
        .align_y(Alignment::Center);
    container(
        column![head, body, actions]
            .spacing(look.gap.max(12.0))
            .width(Length::Fill),
    )
    .padding(look.card_pad.max(14.0) + 2.0)
    .max_width(560)
    .style(st::dialog_style)
    .into()
}

/// Base content, then toasts, then the modal (if any) on top.
pub fn layer<'a, M: Clone + 'a>(
    base: Element<'a, M>,
    toasts: Element<'a, M>,
    modal: Option<Modal<'a, M>>,
) -> Element<'a, M> {
    let mut layers = stack![base, toasts]
        .width(Length::Fill)
        .height(Length::Fill);
    if let Some(modal) = modal {
        let cancel = modal.on_cancel.clone();
        // Outer opaque blocks the page; inner opaque keeps clicks inside the
        // dialog from reaching the scrim's cancel.
        layers = layers.push(opaque(
            mouse_area(
                center(opaque(dialog(modal)))
                    .padding(24)
                    .style(st::scrim_style),
            )
            .on_press(cancel),
        ));
    }
    layers.into()
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub id: u64,
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

/// Results fade after 5 s; errors stay until closed.
pub const TOAST_TTL: Duration = Duration::from_secs(5);

pub fn expired(t: &Toast, now: Instant) -> bool {
    !t.error && now.duration_since(t.at) >= TOAST_TTL
}

pub fn toasts<'a, M: Clone + 'a>(list: &'a [Toast], on_close: fn(u64) -> M) -> Element<'a, M> {
    let items = list.iter().map(|t| {
        let tone = if t.error {
            st::Tone::Danger
        } else {
            st::Tone::Ok
        };
        container(
            row![
                container(Space::new().width(3).height(18)).style(move |th: &Theme| {
                    let c = st::colors(th);
                    container::Style {
                        background: Some(
                            if matches!(tone, st::Tone::Danger) {
                                c.danger
                            } else {
                                c.ok
                            }
                            .into(),
                        ),
                        border: iced::Border {
                            radius: 2.0.into(),
                            ..iced::Border::default()
                        },
                        ..container::Style::default()
                    }
                }),
                st::body(t.text.as_str()).width(Length::Fill),
                st::icon_btn(st::Icon::X).on_press(on_close(t.id)),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .padding([8, 12])
        .width(360)
        .style(st::dialog_style)
        .into()
    });
    container(column(items).spacing(8).align_x(Alignment::End))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_right(Length::Fill)
        .align_bottom(Length::Fill)
        .padding(16)
        .into()
}
