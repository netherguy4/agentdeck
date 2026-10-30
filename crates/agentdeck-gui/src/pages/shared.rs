//! Shared page: skills, instruction links and MCP servers across harnesses.
//! Status is read-only (MCP values stay hidden); sync/link show the core
//! change preview and apply exactly the previewed changes off the UI thread.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use agentdeck_core::shared::{self, SkillState};
use agentdeck_core::{Change, Paths};
use i18n_embed_fl::fl;
use iced::widget::{button, column};
use iced::{Element, Task};

use crate::i18n::loader;
use crate::style as st;

#[derive(Debug, Clone)]
pub struct Ctx {
    pub paths: Paths,
    pub home: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub skills: Vec<String>,
    pub instructions: Vec<String>,
    pub mcp_servers: Vec<String>,
    pub mcp_targets: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Skills,
    Instructions,
    Mcp,
}

#[derive(Debug, Clone, Default)]
pub struct Pending {
    pub lines: Vec<String>,
    pub changes: Vec<Change>,
}

#[derive(Default)]
pub struct State {
    pub snap: Snapshot,
    pub pending: Option<Pending>,
    pub busy: bool,
    pub result: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Loaded(Snapshot),
    Ask(Kind),
    Confirm,
    Cancel,
    Done(Result<String, String>),
}

/// `~`-shortened path text.
fn short(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

pub fn load(ctx: &Ctx) -> Msg {
    let mut snap = Snapshot::default();
    // Skills: one line per existing target dir (core manages only those).
    let mut per_dir: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for s in shared::skills_status(&ctx.home, &ctx.paths) {
        if matches!(s.state, SkillState::NoDir) {
            continue;
        }
        let entry = per_dir.entry(short(&s.target, &ctx.home)).or_default();
        match s.state {
            SkillState::Ok | SkillState::Foreign => entry.0 += 1,
            _ => entry.1 += 1,
        }
    }
    for (dir, (ok, todo)) in per_dir {
        snap.skills.push(fl!(
            loader(),
            "shared-skills-dir",
            dir = dir,
            ok = i64::try_from(ok).unwrap_or(0),
            todo = i64::try_from(todo).unwrap_or(0)
        ));
    }
    snap.instructions = shared::instructions_status(&ctx.home)
        .iter()
        .map(|s| format!("{}: {}", short(&s.path, &ctx.home), s.state.describe()))
        .collect();
    match shared::claude_secondaries(&ctx.paths, &ctx.home) {
        Ok(secondaries) => {
            let (redacted, targets) = shared::mcp_status(&ctx.home, &ctx.paths, &secondaries);
            let mut names: Vec<String> = redacted
                .as_object()
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default();
            names.sort();
            snap.mcp_servers = names;
            snap.mcp_targets = targets
                .iter()
                .map(|t| {
                    format!(
                        "{}: {}",
                        t.name.replace(&ctx.home.display().to_string(), "~"),
                        t.detail
                    )
                })
                .collect();
        }
        Err(e) => snap.error = Some(format!("{e:#}")),
    }
    Msg::Loaded(snap)
}

fn plan(ctx: &Ctx, kind: Kind) -> anyhow::Result<Pending> {
    let mut lines = Vec::new();
    let changes = match kind {
        Kind::Skills => shared::plan_skills_sync(&ctx.home, &ctx.paths)?,
        Kind::Instructions => {
            shared::plan_instructions_link(&ctx.home, &ctx.paths.state_dir, false)?
        }
        Kind::Mcp => {
            let secondaries = shared::claude_secondaries(&ctx.paths, &ctx.home)?;
            let (changes, skipped) =
                shared::plan_mcp_sync(&ctx.home, &ctx.paths.state_dir, &secondaries, false)?;
            lines.extend(
                skipped
                    .iter()
                    .map(|n| format!("skip {n}: unknown entry shape, left alone")),
            );
            changes
        }
    };
    lines.extend(changes.iter().map(shared::describe_mcp_change));
    Ok(Pending { lines, changes })
}

pub fn update(state: &mut State, ctx: Ctx, msg: Msg) -> Task<Msg> {
    match msg {
        Msg::Loaded(snap) => {
            state.snap = snap;
            Task::none()
        }
        Msg::Ask(kind) => {
            state.result = None;
            state.pending = match plan(&ctx, kind) {
                Ok(p) if p.changes.is_empty() && p.lines.is_empty() => {
                    state.result = Some(fl!(loader(), "action-none"));
                    None
                }
                Ok(p) => Some(p),
                // A refusal shows as a panel with no changes, hence no Apply.
                Err(e) => Some(Pending {
                    lines: vec![format!("{e:#}")],
                    changes: Vec::new(),
                }),
            };
            Task::none()
        }
        Msg::Confirm => match state.pending.take().filter(|p| !p.changes.is_empty()) {
            Some(p) => {
                state.busy = true;
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            agentdeck_core::apply_changes(&p.changes)
                                .map(|()| p.lines.join("\n"))
                                .map_err(|e| format!("{e:#}"))
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

fn section<'a>(heading: String, lines: &'a [String], action: (String, Kind)) -> Element<'a, Msg> {
    let mut col = column![st::heading(heading)].spacing(st::SPACE_XS);
    for line in lines {
        col = col.push(st::muted(line.clone()));
    }
    col.push(
        button(st::body(action.0))
            .style(button::secondary)
            .on_press(Msg::Ask(action.1)),
    )
    .into()
}

pub fn modal(state: &State) -> Option<crate::overlay::Modal<'_, Msg>> {
    state.pending.as_ref().map(|p| {
        crate::overlay::confirm(
            fl!(loader(), "shared-preview-title"),
            &p.lines,
            (
                fl!(loader(), "action-apply"),
                (!p.changes.is_empty()).then_some(Msg::Confirm),
            ),
            (fl!(loader(), "action-cancel"), Msg::Cancel),
        )
    })
}

pub fn view(state: &State) -> Element<'_, Msg> {
    let mut col = column![st::title(fl!(loader(), "shared-title"))].spacing(st::SPACE_M);
    if let Some(error) = &state.snap.error {
        col = col.push(st::muted(fl!(loader(), "load-error", what = error)));
    }
    col = col.push(section(
        fl!(loader(), "shared-skills"),
        &state.snap.skills,
        (fl!(loader(), "shared-skills-sync"), Kind::Skills),
    ));
    col = col.push(st::divider());
    col = col.push(section(
        fl!(loader(), "shared-instr"),
        &state.snap.instructions,
        (fl!(loader(), "shared-instr-link"), Kind::Instructions),
    ));
    col = col.push(st::divider());
    let mut mcp = column![st::heading(fl!(loader(), "shared-mcp"))].spacing(st::SPACE_XS);
    mcp = mcp.push(st::body(if state.snap.mcp_servers.is_empty() {
        fl!(loader(), "shared-mcp-none")
    } else {
        state.snap.mcp_servers.join(", ")
    }));
    mcp = mcp.push(st::muted(fl!(loader(), "shared-mcp-hidden")));
    for line in &state.snap.mcp_targets {
        mcp = mcp.push(st::muted(line.clone()));
    }
    col = col.push(
        mcp.push(
            button(st::body(fl!(loader(), "shared-mcp-sync")))
                .style(button::secondary)
                .on_press(Msg::Ask(Kind::Mcp)),
        ),
    );
    if state.busy {
        col = col.push(st::muted(fl!(loader(), "action-working")));
    }
    if let Some(r) = &state.result {
        col = col.push(st::muted(r.clone()));
    }
    col.into()
}
