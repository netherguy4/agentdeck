//! agentdeck GUI shell (iced 0.14): sidebar with Instances, Drift, Plugins and
//! Processes pages. Drift shows per-instance-group drift with Apply / Import /
//! Fix / Re-enable actions behind a preview-then-confirm flow. All data reads
//! are read-only; refresh ticks and actions run off the UI thread via
//! `Task::perform`. Never opens a window in tests: see tests/.

pub mod app;
pub mod data;
pub mod i18n;
pub mod pages;

pub use app::{Message, Page, State};

/// Open the GUI. Without a display the backend returns an error (no panic).
pub fn run() -> iced::Result {
    iced::application(State::new, app::update, app::view)
        .title(app::title)
        .theme(app::theme)
        .subscription(app::subscription)
        .window_size(iced::Size::new(1100.0, 700.0))
        .run()
}
