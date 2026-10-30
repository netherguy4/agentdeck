//! Shared visual language: colour tokens (dark and light), a few layout
//! parameters, text roles, buttons, pills, cards and lucide icons. Only
//! presentation: no behaviour lives here.
//!
//! shortcut: three design variants (`AGENTDECK_UI_VARIANT=a|b|c`) live side
//! by side until the owner picks one; then drop the losing `Look`/`Colors`
//! constants and the `Variant` switch.

use std::sync::atomic::{AtomicU8, Ordering};

use iced::widget::{Space, button, column, container, progress_bar, row, svg, text};
use iced::{Background, Border, Color, Element, Font, Length, Shadow, Theme, Vector, color};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// a: near-black, hairline borders, flat, compact.
    Minimal,
    /// b: translucent surfaces, frosted sidebar and cards, more air.
    Glass,
    /// c: table-first, tight rows, monospace-heavy.
    Console,
}

static VARIANT: AtomicU8 = AtomicU8::new(u8::MAX);

pub fn variant() -> Variant {
    match VARIANT.load(Ordering::Relaxed) {
        0 => Variant::Minimal,
        1 => Variant::Glass,
        2 => Variant::Console,
        _ => {
            let v = match std::env::var("AGENTDECK_UI_VARIANT").as_deref() {
                Ok("b") => Variant::Glass,
                Ok("c") => Variant::Console,
                _ => Variant::Minimal,
            };
            set_variant(v);
            v
        }
    }
}

/// Test seam (and the env default above).
pub fn set_variant(v: Variant) {
    VARIANT.store(v as u8, Ordering::Relaxed);
}

/// Layout parameters that differ between variants.
pub struct Look {
    pub radius: f32,
    pub card_pad: f32,
    /// Between cards / sections.
    pub gap: f32,
    /// Between lines inside a card.
    pub row_gap: f32,
    pub title: f32,
    pub heading: f32,
    pub body: f32,
    pub small: f32,
    pub sidebar_width: f32,
    pub page_pad: f32,
    pub content_max: f32,
    pub bar: f32,
    pub button_pad: [f32; 2],
    /// Window background alpha: below 1 lets compositor glass show through.
    pub window_alpha: f32,
    /// Console: lists render as one table instead of cards.
    pub table: bool,
    /// Console: data lines in monospace.
    pub mono: bool,
}

const MINIMAL: Look = Look {
    radius: 8.0,
    card_pad: 12.0,
    gap: 8.0,
    row_gap: 6.0,
    title: 17.0,
    heading: 14.0,
    body: 13.0,
    small: 12.0,
    sidebar_width: 208.0,
    page_pad: 20.0,
    content_max: 980.0,
    bar: 4.0,
    button_pad: [4.0, 10.0],
    window_alpha: 0.96,
    table: false,
    mono: false,
};

const GLASS: Look = Look {
    radius: 14.0,
    card_pad: 18.0,
    gap: 14.0,
    row_gap: 10.0,
    title: 22.0,
    heading: 15.0,
    body: 13.0,
    small: 12.0,
    sidebar_width: 232.0,
    page_pad: 28.0,
    content_max: 940.0,
    bar: 8.0,
    button_pad: [7.0, 14.0],
    window_alpha: 0.72,
    table: false,
    mono: false,
};

const CONSOLE: Look = Look {
    radius: 3.0,
    card_pad: 8.0,
    gap: 0.0,
    row_gap: 2.0,
    title: 15.0,
    heading: 12.0,
    body: 12.0,
    small: 11.0,
    sidebar_width: 176.0,
    page_pad: 14.0,
    content_max: 1200.0,
    bar: 3.0,
    button_pad: [2.0, 8.0],
    window_alpha: 0.97,
    table: true,
    mono: true,
};

pub fn look() -> &'static Look {
    match variant() {
        Variant::Minimal => &MINIMAL,
        Variant::Glass => &GLASS,
        Variant::Console => &CONSOLE,
    }
}

/// Colour tokens; alpha in `sidebar`/`surface` is the glass.
pub struct Colors {
    pub bg: Color,
    pub sidebar: Color,
    pub surface: Color,
    pub raised: Color,
    pub hover: Color,
    pub border: Color,
    pub text: Color,
    pub muted: Color,
    pub faint: Color,
    pub accent: Color,
    pub on_accent: Color,
    pub danger: Color,
    pub ok: Color,
    pub warn: Color,
    pub track: Color,
    pub scrim: Color,
}

const MINIMAL_DARK: Colors = Colors {
    bg: color!(0x0a0a0a),
    sidebar: color!(0x0a0a0a),
    surface: color!(0x121212),
    raised: color!(0x161616),
    hover: color!(0xffffff, 0.06),
    border: color!(0xffffff, 0.08),
    text: color!(0xf1f3f7),
    muted: color!(0xa3a3a3),
    faint: color!(0x737373),
    accent: color!(0x5b74f0),
    on_accent: color!(0xffffff),
    danger: color!(0xe5484d),
    ok: color!(0x3fb950),
    warn: color!(0xd29922),
    track: color!(0xffffff, 0.08),
    scrim: color!(0x000000, 0.6),
};

const MINIMAL_LIGHT: Colors = Colors {
    bg: color!(0xfcfcfd),
    sidebar: color!(0xf6f6f7),
    surface: color!(0xffffff),
    raised: color!(0xffffff),
    hover: color!(0x000000, 0.05),
    border: color!(0x000000, 0.09),
    text: color!(0x18181b),
    muted: color!(0x5f5f68),
    faint: color!(0x8e8e96),
    accent: color!(0x4459e0),
    on_accent: color!(0xffffff),
    danger: color!(0xd92d32),
    ok: color!(0x1f883d),
    warn: color!(0x9a6700),
    track: color!(0x000000, 0.07),
    scrim: color!(0x000000, 0.35),
};

const GLASS_DARK: Colors = Colors {
    bg: color!(0x10131b),
    sidebar: color!(0xffffff, 0.035),
    surface: color!(0xffffff, 0.055),
    raised: color!(0x1a1e29, 0.98),
    hover: color!(0xffffff, 0.08),
    border: color!(0xffffff, 0.11),
    text: color!(0xeef1f8),
    muted: color!(0xa9b1c3),
    faint: color!(0x7a8296),
    accent: color!(0x8fb4ff),
    on_accent: color!(0x0b1020),
    danger: color!(0xff6b6b),
    ok: color!(0x5dd39e),
    warn: color!(0xf2c14e),
    track: color!(0xffffff, 0.1),
    scrim: color!(0x05070c, 0.55),
};

const GLASS_LIGHT: Colors = Colors {
    bg: color!(0xe9edf4),
    sidebar: color!(0xffffff, 0.45),
    surface: color!(0xffffff, 0.68),
    raised: color!(0xffffff, 0.98),
    hover: color!(0x0f172a, 0.06),
    border: color!(0x0f172a, 0.1),
    text: color!(0x0f172a),
    muted: color!(0x4f5d74),
    faint: color!(0x8590a6),
    accent: color!(0x3866d6),
    on_accent: color!(0xffffff),
    danger: color!(0xd33a3a),
    ok: color!(0x198754),
    warn: color!(0xa16207),
    track: color!(0x0f172a, 0.08),
    scrim: color!(0x0f172a, 0.3),
};

const CONSOLE_DARK: Colors = Colors {
    bg: color!(0x0d1013),
    sidebar: color!(0x090b0e),
    surface: color!(0x11151a),
    raised: color!(0x151a20),
    hover: color!(0xffffff, 0.04),
    border: color!(0x262c34),
    text: color!(0xd7dde5),
    muted: color!(0x8b949e),
    faint: color!(0x5c6570),
    accent: color!(0x3ddc97),
    on_accent: color!(0x06120c),
    danger: color!(0xf85149),
    ok: color!(0x3ddc97),
    warn: color!(0xe3b341),
    track: color!(0x20262e),
    scrim: color!(0x000000, 0.6),
};

const CONSOLE_LIGHT: Colors = Colors {
    bg: color!(0xf6f8fa),
    sidebar: color!(0xeef1f4),
    surface: color!(0xffffff),
    raised: color!(0xffffff),
    hover: color!(0x000000, 0.04),
    border: color!(0xd0d7de),
    text: color!(0x1f2328),
    muted: color!(0x59636e),
    faint: color!(0x8c959f),
    accent: color!(0x1a7f37),
    on_accent: color!(0xffffff),
    danger: color!(0xcf222e),
    ok: color!(0x1a7f37),
    warn: color!(0x9a6700),
    track: color!(0xe6eaef),
    scrim: color!(0x000000, 0.3),
};

fn colors_for(dark: bool) -> &'static Colors {
    match (variant(), dark) {
        (Variant::Minimal, true) => &MINIMAL_DARK,
        (Variant::Minimal, false) => &MINIMAL_LIGHT,
        (Variant::Glass, true) => &GLASS_DARK,
        (Variant::Glass, false) => &GLASS_LIGHT,
        (Variant::Console, true) => &CONSOLE_DARK,
        (Variant::Console, false) => &CONSOLE_LIGHT,
    }
}

pub fn colors(theme: &Theme) -> &'static Colors {
    colors_for(theme.extended_palette().is_dark)
}

/// The app theme for the current variant; built-in widgets (inputs,
/// checkboxes, scrollbars) derive from this palette.
pub fn theme(dark: bool) -> Theme {
    let c = colors_for(dark);
    Theme::custom(
        format!(
            "agentdeck-{:?}-{}",
            variant(),
            if dark { "dark" } else { "light" }
        ),
        iced::theme::Palette {
            background: c.bg,
            text: c.text,
            primary: c.accent,
            success: c.ok,
            warning: c.warn,
            danger: c.danger,
        },
    )
}

/// Window background: translucent so COSMIC glass/blur shows through; the
/// opaque `bg` is the fallback headless snapshots and other desktops get.
pub fn app_style(theme: &Theme) -> iced::theme::Style {
    let c = colors(theme);
    iced::theme::Style {
        background_color: Color {
            a: look().window_alpha,
            ..c.bg
        },
        text_color: c.text,
    }
}

/// Spacing scale (px) kept for pages not redesigned yet.
pub const SPACE_XS: f32 = 4.0;
pub const SPACE_S: f32 = 8.0;
pub const SPACE_M: f32 = 12.0;
pub const SPACE_L: f32 = 20.0;

const SEMIBOLD: Font = Font {
    weight: iced::font::Weight::Semibold,
    ..Font::DEFAULT
};

pub fn title<'a>(label: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(label).size(look().title).font(SEMIBOLD)
}

pub fn heading<'a>(label: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(label).size(look().heading).font(SEMIBOLD)
}

pub fn body<'a>(label: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(label).size(look().body)
}

/// Secondary line in the muted token colour.
pub fn muted<'a>(label: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(label)
        .size(look().body)
        .style(|t: &Theme| text::Style {
            color: Some(colors(t).muted),
        })
}

/// Small faint label: column headers, group captions, units.
pub fn caption<'a>(label: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(label)
        .size(look().small)
        .style(|t: &Theme| text::Style {
            color: Some(colors(t).faint),
        })
}

/// Ids, paths, commands.
pub fn mono<'a>(label: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(label).size(look().small).font(Font::MONOSPACE)
}

pub fn mono_muted<'a>(label: impl text::IntoFragment<'a>) -> text::Text<'a> {
    mono(label).style(|t: &Theme| text::Style {
        color: Some(colors(t).muted),
    })
}

/// Data text: monospace in the Console variant, body elsewhere.
pub fn data<'a>(label: impl text::IntoFragment<'a>) -> text::Text<'a> {
    if look().mono {
        mono(label).size(look().body)
    } else {
        body(label)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Tone {
    Neutral,
    Accent,
    Ok,
    Warn,
    Danger,
}

fn tone_color(c: &Colors, tone: Tone) -> Color {
    match tone {
        Tone::Neutral => c.muted,
        Tone::Accent => c.accent,
        Tone::Ok => c.ok,
        Tone::Warn => c.warn,
        Tone::Danger => c.danger,
    }
}

/// Small status badge.
pub fn pill<'a, M: 'a>(label: impl text::IntoFragment<'a>, tone: Tone) -> Element<'a, M> {
    let console = look().table;
    container(
        text(label)
            .size(look().small)
            .font(if console {
                Font::MONOSPACE
            } else {
                Font::DEFAULT
            })
            .style(move |t: &Theme| text::Style {
                color: Some(tone_color(colors(t), tone)),
            }),
    )
    .padding([1.0, if console { 5.0 } else { 8.0 }])
    .style(move |t: &Theme| {
        let c = colors(t);
        let fg = tone_color(c, tone);
        container::Style {
            background: Some(Color { a: 0.13, ..fg }.into()),
            border: Border {
                color: Color { a: 0.28, ..fg },
                width: 1.0,
                radius: if console { 2.0 } else { 999.0 }.into(),
            },
            ..container::Style::default()
        }
    })
    .into()
}

pub fn card_style(t: &Theme) -> container::Style {
    let c = colors(t);
    container::Style {
        background: Some(c.surface.into()),
        border: Border {
            color: c.border,
            width: 1.0,
            radius: look().radius.into(),
        },
        ..container::Style::default()
    }
}

/// Bordered card around a block of rows.
pub fn card<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Element<'a, M> {
    container(content)
        .padding(look().card_pad)
        .width(Length::Fill)
        .style(card_style)
        .into()
}

pub fn sidebar_style(t: &Theme) -> container::Style {
    container::Style {
        background: Some(colors(t).sidebar.into()),
        ..container::Style::default()
    }
}

/// Mostly opaque dialog surface with a soft shadow.
pub fn dialog_style(t: &Theme) -> container::Style {
    let c = colors(t);
    container::Style {
        background: Some(c.raised.into()),
        border: Border {
            color: c.border,
            width: 1.0,
            radius: (look().radius + 2.0).into(),
        },
        shadow: Shadow {
            color: color!(0x000000, 0.4),
            offset: Vector::new(0.0, 8.0),
            blur_radius: 32.0,
        },
        ..container::Style::default()
    }
}

pub fn scrim_style(t: &Theme) -> container::Style {
    container::Style {
        background: Some(colors(t).scrim.into()),
        ..container::Style::default()
    }
}

/// 1 px hairline, horizontal.
pub fn hairline<'a, M: 'a>() -> Element<'a, M> {
    container(Space::new())
        .width(Length::Fill)
        .height(1)
        .style(|t: &Theme| container::Style {
            background: Some(colors(t).border.into()),
            ..container::Style::default()
        })
        .into()
}

/// 1 px hairline, vertical.
pub fn hairline_v<'a, M: 'a>() -> Element<'a, M> {
    container(Space::new())
        .width(1)
        .height(Length::Fill)
        .style(|t: &Theme| container::Style {
            background: Some(colors(t).border.into()),
            ..container::Style::default()
        })
        .into()
}

/// Kept for pages not redesigned yet.
pub fn divider<'a, M: 'a>() -> Element<'a, M> {
    hairline()
}

/// Thin quota bar; warn colour from 90 % so red stays for destructive buttons.
pub fn quota_bar(percent: f32) -> progress_bar::ProgressBar<'static> {
    progress_bar(0.0..=100.0, percent)
        .girth(Length::Fixed(look().bar))
        .style(move |t: &Theme| {
            let c = colors(t);
            progress_bar::Style {
                background: c.track.into(),
                bar: if percent >= 90.0 { c.warn } else { c.accent }.into(),
                border: Border {
                    radius: (look().bar / 2.0).into(),
                    ..Border::default()
                },
            }
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Btn {
    Primary,
    /// Bordered, transparent: secondary actions.
    Quiet,
    /// No border: icon buttons, row toggles.
    Ghost,
    Danger,
}

fn btn_style(kind: Btn) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |t, status| {
        let c = colors(t);
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let (bg, fg, border) = match kind {
            Btn::Primary => (
                Some(Color {
                    a: if hovered { 0.88 } else { 1.0 },
                    ..c.accent
                }),
                c.on_accent,
                Color::TRANSPARENT,
            ),
            Btn::Danger => (
                Some(Color {
                    a: if hovered { 0.88 } else { 1.0 },
                    ..c.danger
                }),
                Color::WHITE,
                Color::TRANSPARENT,
            ),
            Btn::Quiet => (hovered.then_some(c.hover), c.text, c.border),
            Btn::Ghost => (hovered.then_some(c.hover), c.text, Color::TRANSPARENT),
        };
        let disabled = matches!(status, button::Status::Disabled);
        button::Style {
            background: bg.map(Background::Color),
            text_color: if disabled {
                Color { a: 0.45, ..fg }
            } else {
                fg
            },
            border: Border {
                color: border,
                width: 1.0,
                radius: look().radius.min(8.0).into(),
            },
            ..button::Style::default()
        }
    }
}

pub fn btn<'a, M: 'a>(kind: Btn, label: impl text::IntoFragment<'a>) -> button::Button<'a, M> {
    button(text(label).size(look().body))
        .padding(look().button_pad)
        .style(btn_style(kind))
}

/// Icon plus label, e.g. the page's primary action.
pub fn btn_icon<'a, M: 'a>(
    kind: Btn,
    i: Icon,
    label: impl text::IntoFragment<'a>,
) -> button::Button<'a, M> {
    let size = look().body;
    button(
        row![
            icon_colored(i, size, move |c| match kind {
                Btn::Primary => c.on_accent,
                Btn::Danger => Color::WHITE,
                _ => c.text,
            }),
            text(label).size(size)
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center),
    )
    .padding(look().button_pad)
    .style(btn_style(kind))
}

/// Square icon-only button.
pub fn icon_btn<'a, M: 'a>(i: Icon) -> button::Button<'a, M> {
    button(icon(i, look().body + 1.0))
        .padding(look().button_pad[0] + 1.0)
        .style(btn_style(Btn::Ghost))
}

/// Sidebar item: selected gets a quiet fill, never the accent block.
pub fn nav_style(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |t, status| {
        let c = colors(t);
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: (selected || hovered).then_some(Background::Color(if selected {
                Color {
                    a: c.hover.a * 1.8,
                    ..c.hover
                }
            } else {
                c.hover
            })),
            text_color: if selected { c.text } else { c.muted },
            border: Border {
                radius: look().radius.min(8.0).into(),
                ..Border::default()
            },
            ..button::Style::default()
        }
    }
}

/// Transparent full-width row button (expand/collapse).
pub fn row_style(t: &Theme, status: button::Status) -> button::Style {
    let c = colors(t);
    button::Style {
        background: matches!(status, button::Status::Hovered).then_some(Background::Color(c.hover)),
        text_color: c.text,
        border: Border {
            radius: look().radius.min(8.0).into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

/// Lucide icons (ISC licence, see the crate README), inner markup only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Users,
    Compare,
    Plug,
    Layers,
    Share,
    Wrench,
    Activity,
    Plus,
    Trash,
    Search,
    ChevronRight,
    ChevronDown,
    X,
    Square,
}

impl Icon {
    fn markup(self) -> &'static str {
        match self {
            Icon::Users => {
                r#"<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/>"#
            }
            Icon::Compare => {
                r#"<circle cx="18" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><path d="M13 6h3a2 2 0 0 1 2 2v7"/><path d="M11 18H8a2 2 0 0 1-2-2V9"/>"#
            }
            Icon::Plug => {
                r#"<path d="M12 22v-5"/><path d="M9 8V2"/><path d="M15 8V2"/><path d="M18 8v5a4 4 0 0 1-4 4h-4a4 4 0 0 1-4-4V8Z"/>"#
            }
            Icon::Layers => {
                r#"<path d="m12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83Z"/><path d="m22 17.65-9.17 4.16a2 2 0 0 1-1.66 0L2 17.65"/><path d="m22 12.65-9.17 4.16a2 2 0 0 1-1.66 0L2 12.65"/>"#
            }
            Icon::Share => {
                r#"<circle cx="18" cy="5" r="3"/><circle cx="6" cy="12" r="3"/><circle cx="18" cy="19" r="3"/><line x1="8.59" x2="15.42" y1="13.51" y2="17.49"/><line x1="15.41" x2="8.59" y1="6.51" y2="10.49"/>"#
            }
            Icon::Wrench => {
                r#"<path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/>"#
            }
            Icon::Activity => r#"<polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/>"#,
            Icon::Plus => r#"<path d="M5 12h14"/><path d="M12 5v14"/>"#,
            Icon::Trash => {
                r#"<path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/><line x1="10" x2="10" y1="11" y2="17"/><line x1="14" x2="14" y1="11" y2="17"/>"#
            }
            Icon::Search => r#"<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>"#,
            Icon::ChevronRight => r#"<path d="m9 18 6-6-6-6"/>"#,
            Icon::ChevronDown => r#"<path d="m6 9 6 6 6-6"/>"#,
            Icon::X => r#"<path d="M18 6 6 18"/><path d="m6 6 12 12"/>"#,
            Icon::Square => r#"<rect width="14" height="14" x="5" y="5" rx="2"/>"#,
        }
    }

    fn handle(self) -> svg::Handle {
        svg::Handle::from_memory(
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{}</svg>"#,
                self.markup()
            )
            .into_bytes(),
        )
    }
}

/// Icon in the muted token colour.
pub fn icon<'a>(i: Icon, size: f32) -> svg::Svg<'a> {
    icon_colored(i, size, |c| c.muted)
}

pub fn icon_colored<'a>(i: Icon, size: f32, pick: impl Fn(&Colors) -> Color + 'a) -> svg::Svg<'a> {
    svg(i.handle())
        .width(size)
        .height(size)
        .style(move |t: &Theme, _| svg::Style {
            color: Some(pick(colors(t))),
        })
}

/// Generic bordered block kept for inline sections (add wizard).
pub fn panel<'a, Message: 'a>(
    heading_label: String,
    lines: Vec<Element<'a, Message>>,
    actions: Vec<Element<'a, Message>>,
) -> Element<'a, Message> {
    let mut col = column![heading(heading_label)].spacing(look().row_gap);
    for line in lines {
        col = col.push(line);
    }
    if !actions.is_empty() {
        col = col.push(row(actions).spacing(SPACE_S));
    }
    card(col)
}

/// Buttons side by side.
pub fn actions<'a, Message: 'a>(items: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    row(items).spacing(SPACE_S).into()
}

/// Slim scrollbar: no rail, a hairline-coloured thumb.
pub fn scroll_style(
    t: &Theme,
    status: iced::widget::scrollable::Status,
) -> iced::widget::scrollable::Style {
    let c = colors(t);
    let mut s = iced::widget::scrollable::default(t, status);
    for rail in [&mut s.vertical_rail, &mut s.horizontal_rail] {
        rail.background = None;
        rail.border = Border::default();
        rail.scroller.background = Color {
            a: c.border.a * 2.5,
            ..c.border
        }
        .into();
        rail.scroller.border = Border {
            radius: 3.0.into(),
            ..Border::default()
        };
    }
    s
}
