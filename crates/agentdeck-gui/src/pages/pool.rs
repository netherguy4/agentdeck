//! Pool page: `[[pool]]` entries with add / enable / remove and the T3
//! favorites import. Every change shows the core preview first; the apply
//! runs off the UI thread through the same core functions as `agentdeck pool`.

use std::path::{Path, PathBuf};

use agentdeck_core::{Paths, PoolEntry, Registry, pool};
use i18n_embed_fl::fl;
use iced::widget::{button, column, pick_list, row, text_input};
use iced::{Element, Task};

use crate::i18n::loader;
use crate::style as st;

/// Explicit roots for every read/write (tests pass temp dirs).
#[derive(Debug, Clone)]
pub struct Ctx {
    pub paths: Paths,
    pub t3_root: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub enum Op {
    Add(PoolEntry),
    Remove(PoolEntry),
    Toggle(PoolEntry),
    Import,
}

#[derive(Debug, Clone)]
pub struct Pending {
    /// `None` for a refusal: the panel then has no Apply.
    pub op: Option<Op>,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Loaded {
    pub entries: Vec<PoolEntry>,
    pub instances: Vec<String>,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct State {
    pub data: Loaded,
    /// Model slugs from the T3 cache of the chosen instance.
    pub models: Vec<String>,
    pub instance: Option<String>,
    pub model: String,
    pub options: String,
    pub pending: Option<Pending>,
    pub busy: bool,
    pub result: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Loaded(Loaded),
    Instance(String),
    Model(String),
    Options(String),
    AskAdd,
    AskRemove(usize),
    AskToggle(usize),
    AskImport,
    Confirm,
    Cancel,
    Done(Result<String, String>),
}

pub fn load(ctx: &Ctx) -> Msg {
    Msg::Loaded(match Registry::load(&ctx.paths) {
        Ok(reg) => Loaded {
            entries: reg.pool.clone(),
            instances: reg.instances.iter().map(|i| i.id.clone()).collect(),
            error: None,
        },
        Err(e) => Loaded {
            error: Some(format!("{e:#}")),
            ..Loaded::default()
        },
    })
}

/// Model slugs T3 cached for `instance` (empty when there is no cache).
fn cached_models(t3_root: Option<&Path>, instance: &str) -> Vec<String> {
    let Some(root) = t3_root else {
        return Vec::new();
    };
    std::fs::read(root.join("caches").join(format!("{instance}.json")))
        .ok()
        .and_then(|raw| serde_json::from_slice::<serde_json::Value>(&raw).ok())
        .and_then(|v| {
            Some(
                v.get("models")?
                    .as_array()?
                    .iter()
                    .filter_map(|m| m.get("slug")?.as_str().map(str::to_string))
                    .collect(),
            )
        })
        .unwrap_or_default()
}

fn form_entry(state: &State) -> anyhow::Result<PoolEntry> {
    let instance = state
        .instance
        .clone()
        .ok_or_else(|| anyhow::anyhow!("choose an instance"))?;
    let args: Vec<String> = state
        .options
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    Ok(PoolEntry {
        instance,
        model: state.model.trim().to_string(),
        options: pool::parse_options(&args)?,
        enabled: true,
    })
}

fn lines_of(changes: &[agentdeck_core::Change]) -> Vec<String> {
    changes.iter().map(|c| c.describe()).collect()
}

/// Read-only preview of `op` through the core dry-run.
fn preview(ctx: &Ctx, op: &Op) -> anyhow::Result<Vec<String>> {
    match op {
        Op::Add(e) => {
            let (changes, warn) = pool::add_pool_entry(&ctx.paths, e.clone(), true)?;
            let mut lines = lines_of(&changes);
            let warn =
                warn.or_else(|| pool::check_model(ctx.t3_root.as_deref(), &e.instance, &e.model));
            lines.extend(warn.map(|w| fl!(loader(), "pool-warning", what = w)));
            Ok(lines)
        }
        Op::Remove(e) => Ok(lines_of(&pool::remove_pool_entry(
            &ctx.paths,
            &e.instance,
            &e.model,
            &e.options,
            true,
        )?)),
        Op::Toggle(e) => Ok(lines_of(&pool::set_pool_enabled(
            &ctx.paths,
            &e.instance,
            &e.model,
            &e.options,
            !e.enabled,
            true,
        )?)),
        Op::Import => {
            let root = ctx
                .t3_root
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("T3 directory unknown"))?;
            let (changes, added, skipped) = pool::import_favorites(&ctx.paths, root, true)?;
            let mut lines = vec![fl!(
                loader(),
                "pool-import-count",
                count = i64::try_from(added).unwrap_or(0)
            )];
            lines.extend(lines_of(&changes));
            lines.extend(skipped);
            Ok(lines)
        }
    }
}

pub fn apply(ctx: &Ctx, op: &Op) -> anyhow::Result<String> {
    let changes = match op {
        Op::Add(e) => pool::add_pool_entry(&ctx.paths, e.clone(), false)?.0,
        Op::Remove(e) => {
            pool::remove_pool_entry(&ctx.paths, &e.instance, &e.model, &e.options, false)?
        }
        Op::Toggle(e) => pool::set_pool_enabled(
            &ctx.paths,
            &e.instance,
            &e.model,
            &e.options,
            !e.enabled,
            false,
        )?,
        Op::Import => {
            let root = ctx
                .t3_root
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("T3 directory unknown"))?;
            pool::import_favorites(&ctx.paths, root, false)?.0
        }
    };
    Ok(lines_of(&changes).join("\n"))
}

pub fn update(state: &mut State, ctx: Ctx, msg: Msg) -> Task<Msg> {
    match msg {
        Msg::Loaded(data) => {
            state.data = data;
            Task::none()
        }
        Msg::Instance(id) => {
            state.models = cached_models(ctx.t3_root.as_deref(), &id);
            state.instance = Some(id);
            state.model.clear();
            Task::none()
        }
        Msg::Model(m) => {
            state.model = m;
            Task::none()
        }
        Msg::Options(o) => {
            state.options = o;
            Task::none()
        }
        Msg::AskAdd => {
            ask(state, &ctx, form_entry(state).map(Op::Add));
            Task::none()
        }
        Msg::AskRemove(i) => {
            let op = state.data.entries.get(i).cloned().map(Op::Remove);
            ask(
                state,
                &ctx,
                op.ok_or_else(|| anyhow::anyhow!("entry vanished")),
            );
            Task::none()
        }
        Msg::AskToggle(i) => {
            let op = state.data.entries.get(i).cloned().map(Op::Toggle);
            ask(
                state,
                &ctx,
                op.ok_or_else(|| anyhow::anyhow!("entry vanished")),
            );
            Task::none()
        }
        Msg::AskImport => {
            ask(state, &ctx, Ok(Op::Import));
            Task::none()
        }
        Msg::Confirm => match state.pending.take().and_then(|p| p.op) {
            Some(op) => {
                state.busy = true;
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            apply(&ctx, &op).map_err(|e| format!("{e:#}"))
                        })
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|r| r)
                    },
                    Msg::Done,
                )
            }
            None => Task::none(),
        },
        Msg::Cancel => {
            state.pending = None;
            Task::none()
        }
        Msg::Done(result) => {
            state.busy = false;
            state.result = Some(result.unwrap_or_else(|e| e));
            Task::done(load(&ctx))
        }
    }
}

fn ask(state: &mut State, ctx: &Ctx, op: anyhow::Result<Op>) {
    state.result = None;
    state.pending = Some(match op.and_then(|op| Ok((preview(ctx, &op)?, op))) {
        Ok((lines, op)) => Pending {
            op: Some(op),
            lines,
        },
        // A refusal (duplicate, unknown instance, …) shows as a panel with no Apply.
        Err(e) => Pending {
            op: None,
            lines: vec![format!("{e:#}")],
        },
    });
}

pub fn modal(state: &State) -> Option<crate::overlay::Modal<'_, Msg>> {
    state.pending.as_ref().map(|p| {
        crate::overlay::confirm(
            fl!(loader(), "pool-preview-title"),
            &p.lines,
            (
                fl!(loader(), "action-apply"),
                p.op.as_ref().map(|_| Msg::Confirm),
            ),
            (fl!(loader(), "action-cancel"), Msg::Cancel),
        )
    })
}

pub fn view(state: &State) -> Element<'_, Msg> {
    let mut col = column![st::title(fl!(loader(), "pool-title"))].spacing(st::SPACE_M);
    if let Some(error) = &state.data.error {
        col = col.push(st::muted(fl!(loader(), "load-error", what = error)));
    }
    if state.data.entries.is_empty() {
        col = col.push(st::muted(fl!(loader(), "pool-empty")));
    }
    for (i, e) in state.data.entries.iter().enumerate() {
        let mut label = format!("{}  {}", e.instance, e.model);
        for (k, v) in &e.options {
            label.push_str(&format!("  {k}={v}"));
        }
        let (toggle, on) = if e.enabled {
            ("pool-disable", "pool-enabled")
        } else {
            ("pool-enable", "pool-disabled")
        };
        col = col.push(
            row![
                st::body(label),
                st::muted(loader().get(on)),
                button(st::body(loader().get(toggle)))
                    .style(button::secondary)
                    .on_press(Msg::AskToggle(i)),
                button(st::body(fl!(loader(), "pool-remove")))
                    .style(button::secondary)
                    .on_press(Msg::AskRemove(i)),
            ]
            .spacing(st::SPACE_S)
            .align_y(iced::Alignment::Center),
        );
    }
    col = col.push(st::divider());
    col = col.push(st::heading(fl!(loader(), "pool-add-title")));
    let model: Element<'_, Msg> = if state.models.is_empty() {
        column![
            text_input(&fl!(loader(), "pool-model"), &state.model).on_input(Msg::Model),
            st::muted(fl!(loader(), "pool-no-models")),
        ]
        .spacing(st::SPACE_XS)
        .into()
    } else {
        pick_list(
            state.models.clone(),
            Some(state.model.clone()).filter(|m| !m.is_empty()),
            Msg::Model,
        )
        .placeholder(fl!(loader(), "pool-model"))
        .into()
    };
    col = col.push(
        pick_list(
            state.data.instances.clone(),
            state.instance.clone(),
            Msg::Instance,
        )
        .placeholder(fl!(loader(), "pool-instance")),
    );
    col = col.push(model);
    col =
        col.push(text_input(&fl!(loader(), "pool-options"), &state.options).on_input(Msg::Options));
    col = col.push(st::actions(vec![
        button(st::body(fl!(loader(), "pool-preview-add")))
            .style(button::secondary)
            .on_press(Msg::AskAdd)
            .into(),
        button(st::body(fl!(loader(), "pool-import")))
            .style(button::secondary)
            .on_press(Msg::AskImport)
            .into(),
    ]));
    if state.busy {
        col = col.push(st::muted(fl!(loader(), "action-working")));
    }
    if let Some(r) = &state.result {
        col = col.push(st::muted(r.clone()));
    }
    col.into()
}
