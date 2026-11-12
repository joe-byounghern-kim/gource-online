// SPDX-License-Identifier: GPL-3.0-or-later

//! AI attribution in the simulation (AI-30 to AI-36, AI-47, AI-59, AI-60):
//! the per-set display tables, the played-commit counts behind the agent key
//! and `attr_stats`, and the per-user style resolution that turns a user's
//! assist state into [`UserDecor`].
//!
//! Extraction does not see the [`EventLog`], so `process_commit` (which
//! does) copies what the scene needs the first time a set id is played:
//! [`SetInfo`] per set id (kinds, agent ids, models, tints and badges) and
//! the agent id and model strings in `attr_names`. These tables, the played
//! counts and the per-agent counts only grow when a new set or agent first
//! appears, so steady-state frames allocate nothing.
//!
//! The played counts describe the history up to the playback position: a
//! seek resets them and counts again over the commits before the new
//! cursor (one pass over the log, nothing simulated), so they equal a
//! straight playthrough to the same cursor.

use std::collections::BTreeMap;
use std::fmt::Write;

use glam::Vec3;

use super::Simulation;
use crate::agents;
use crate::events::{AttrKind, AttrSetId, EventLog, MAX_SET_ENTRIES, StrId};
use crate::style::{ResolvedStyle, StyleLayers, Subject, SubjectEntry, Tint};
use crate::user::RUser;
use crate::view::{AGENT_KEY_ROWS, AgentKeyRow, DecoKind, SceneView, TextRange, UserDecor};

/// One entry of a played attribution set.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct SetEntry {
    pub(crate) kind: AttrKind,
    /// Agent id (key into `attr_names`).
    pub(crate) agent: StrId,
    /// Model text (key into `attr_names`).
    pub(crate) model: Option<StrId>,
    pub(crate) tint: [u8; 3],
    pub(crate) badge: u8,
}

impl SetEntry {
    fn is_ai(&self) -> bool {
        matches!(self.kind, AttrKind::Assist | AttrKind::Agent)
    }
}

const NO_ENTRY: SetEntry = SetEntry {
    kind: AttrKind::Assist,
    agent: StrId(0),
    model: None,
    tint: [0; 3],
    badge: 0,
};

/// Display data of one attribution set, owned by the simulation.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct SetInfo {
    /// Filled from the log (an unplayed set id is not).
    known: bool,
    len: u8,
    entries: [SetEntry; MAX_SET_ENTRIES],
}

impl Default for SetInfo {
    fn default() -> Self {
        SetInfo {
            known: false,
            len: 0,
            entries: [NO_ENTRY; MAX_SET_ENTRIES],
        }
    }
}

impl SetInfo {
    pub(crate) fn entries(&self) -> &[SetEntry] {
        &self.entries[..self.len as usize]
    }
}

/// The set's entries as the style resolver sees them, in `out`; returns
/// how many.
fn subject_entries<'a>(
    info: Option<&SetInfo>,
    names: &'a BTreeMap<StrId, Box<str>>,
    out: &mut [SubjectEntry<'a>; MAX_SET_ENTRIES],
) -> usize {
    let Some(info) = info else {
        return 0;
    };
    for (slot, e) in out.iter_mut().zip(info.entries()) {
        *slot = SubjectEntry {
            kind: e.kind,
            agent: names.get(&e.agent).map_or("", |s| s),
            tint: e.tint,
            badge: e.badge,
        };
    }
    info.entries().len()
}

/// The cascade for set `set` (0 = no attribution).
pub(crate) fn resolve_style(
    style: &StyleLayers,
    infos: &[SetInfo],
    names: &BTreeMap<StrId, Box<str>>,
    set: u32,
) -> ResolvedStyle {
    let blank = SubjectEntry {
        kind: AttrKind::Assist,
        agent: "",
        tint: [0; 3],
        badge: 0,
    };
    let mut entries = [blank; MAX_SET_ENTRIES];
    let info = infos.get(set as usize).filter(|i| i.known);
    let n = subject_entries(info, names, &mut entries);
    style.resolve(&Subject {
        entries: &entries[..n],
    })
}

fn to_rgb(c: Vec3) -> [u8; 3] {
    let b = |x: f32| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    [b(c.x), b(c.y), b(c.z)]
}

/// [`UserDecor`] of `user` from its cached style (default when nothing is
/// drawn).
pub(crate) fn user_decor(user: &RUser, motion: bool) -> UserDecor {
    let style = &user.style;
    let mask = style.mask();
    if mask == 0 {
        return UserDecor::default();
    }
    let mut tint = [[0; 3]; DecoKind::COUNT];
    for kind in DecoKind::ALL {
        tint[kind.index()] = match style.tints[kind.index()] {
            Tint::Off => [0; 3],
            Tint::Fixed(c) => c,
            Tint::Contributor => to_rgb(user.usercol),
        };
    }
    UserDecor {
        mask,
        dynamic: style.dynamic,
        inverse: 0,
        badge: style.badge,
        tint,
        envelope: user.attr_env,
        scale: 1.0,
        vel: user.pawn.accel,
        phase: user.deco_phase(),
        motion: if motion { 1.0 } else { 0.0 },
    }
}

impl Simulation {
    /// Copy the display data of set `set` from the log, once.
    pub(super) fn note_set(&mut self, log: &EventLog, set: AttrSetId) {
        let i = set.0 as usize;
        if i == 0 {
            return;
        }
        if i >= self.attr_infos.len() {
            self.attr_infos.resize(i + 1, SetInfo::default());
        }
        if self.attr_infos[i].known {
            return;
        }
        let mut info = SetInfo {
            known: true,
            ..SetInfo::default()
        };
        for (slot, e) in info.entries.iter_mut().zip(log.attr_set(set)) {
            let id = log.str(e.id);
            let display = agents::resolve(id);
            self.attr_names.entry(e.id).or_insert_with(|| id.into());
            if let Some(m) = e.model {
                self.attr_names
                    .entry(m)
                    .or_insert_with(|| log.str(m).into());
            }
            *slot = SetEntry {
                kind: e.kind,
                agent: e.id,
                model: e.model,
                tint: display.tint,
                badge: display.badge as u8,
            };
            info.len += 1;
        }
        self.attr_infos[i] = info;
    }

    /// Count one played commit of set `set` (after [`Simulation::note_set`]).
    pub(super) fn count_played(&mut self, set: AttrSetId) {
        let i = set.0 as usize;
        if i >= self.played_set_counts.len() {
            self.played_set_counts.resize(i + 1, 0);
        }
        self.played_set_counts[i] = self.played_set_counts[i].saturating_add(1);
        if i == 0 {
            return;
        }
        let info = self.attr_infos[i];
        let entries = info.entries();
        for (n, e) in entries.iter().enumerate() {
            // each credited agent once per commit
            if !e.is_ai() || entries[..n].iter().any(|p| p.is_ai() && p.agent == e.agent) {
                continue;
            }
            let count = self.agent_counts.entry(e.agent).or_insert(0);
            *count = count.saturating_add(1);
            self.agent_rows_dirty = true;
        }
    }

    /// Forget the played counts (`Gource::reset`).
    pub(super) fn reset_played(&mut self) {
        self.played_set_counts.clear();
        self.agent_counts.clear();
        self.agent_rows.clear();
        self.agents_more = 0;
        self.agent_rows_dirty = false;
    }

    /// Count the commits before `cursor` that a straight playthrough would
    /// have played (after a seek).
    pub(super) fn seed_played(&mut self, log: &EventLog, cursor: usize) {
        let stop = self.settings.stop_timestamp;
        for index in 0..cursor.min(log.commit_count()) {
            if !self.filters.commit_is_valid_cached(log, index) {
                continue;
            }
            let commit = log.commit(index);
            if stop != 0 && commit.timestamp > stop {
                break;
            }
            self.note_set(log, commit.attrs);
            self.count_played(commit.attrs);
        }
    }

    /// Played commits per set id (index 0: commits without attribution).
    pub fn played_set_counts(&self) -> &[u32] {
        &self.played_set_counts
    }

    /// Rebuild the agent key rows when a count changed: the top
    /// [`AGENT_KEY_ROWS`] agents by count (descending), then id.
    fn rebuild_agent_rows(&mut self) {
        if !self.agent_rows_dirty {
            return;
        }
        self.agent_rows_dirty = false;
        let names = &self.attr_names;
        let id = |a: StrId| names.get(&a).map_or("", |s| &**s);
        let rows = &mut self.agent_rows;
        rows.clear();
        for (&agent, &count) in &self.agent_counts {
            let at =
                rows.partition_point(|&(a, c)| c > count || (c == count && id(a) <= id(agent)));
            if at < AGENT_KEY_ROWS {
                rows.insert(at, (agent, count));
                rows.truncate(AGENT_KEY_ROWS);
            }
        }
        self.agents_more = (self.agent_counts.len() - rows.len()) as u32;
    }

    /// The agent key rows and the filter line of the HUD.
    pub(super) fn extract_ai_hud(&mut self, view: &mut SceneView) {
        if self.settings.agent_key {
            self.rebuild_agent_rows();
            view.hud.show_agent_key = true;
            for &(agent, count) in &self.agent_rows {
                let id = self.attr_names.get(&agent).map_or("", |s| &**s);
                let display = agents::resolve(id);
                let name = view.push_text(display.name);
                let [r, g, b] = display.tint;
                view.hud.agents.push(AgentKeyRow {
                    name,
                    colour: Vec3::new(r as f32, g as f32, b as f32) / 255.0,
                    count,
                });
            }
            view.hud.agents_more = self.agents_more;
        }
        let note = match self.settings.ai_filter {
            crate::settings::AiFilter::All => None,
            crate::settings::AiFilter::OnlyAssisted => Some("AI commits: only assisted"),
            crate::settings::AiFilter::ExcludeAssisted => Some("AI commits: excluding assisted"),
        };
        if let Some(note) = note {
            view.hud.filter_note = Some(view.push_text(note));
        }
    }

    /// Re-resolve the style of every user whose state or rules changed.
    pub(super) fn update_user_styles(&mut self) {
        let epoch = self.style_epoch;
        for &u in self.user_map.values() {
            let user = &mut self.users[u];
            let key = (epoch, user.attr_set);
            if user.style_key != Some(key) {
                user.style = resolve_style(
                    &self.style,
                    &self.attr_infos,
                    &self.attr_names,
                    user.attr_set,
                );
                user.style_key = Some(key);
            }
        }
    }

    /// The hover line of a user's current state (AI-60): `AI agent: X` or
    /// `AI-assisted: X[ and others]`, X the model when known, else the agent
    /// name. Written straight into the view's text store.
    pub(super) fn write_assist_line(&self, set: u32, view: &mut SceneView) -> Option<TextRange> {
        let info = self.attr_infos.get(set as usize).filter(|i| i.known)?;
        let entries = info.entries();
        let label = |e: &SetEntry| -> &str {
            match e.model.and_then(|m| self.attr_names.get(&m)) {
                Some(model) => model,
                None => agents::resolve(self.attr_names.get(&e.agent).map_or("", |s| s)).name,
            }
        };
        let start = view.text.len() as u32;
        if let Some(agent) = entries.iter().find(|e| e.kind == AttrKind::Agent) {
            let _ = write!(view.text, "AI agent: {}", label(agent));
        } else {
            let first = entries.iter().find(|e| e.kind == AttrKind::Assist)?;
            let _ = write!(view.text, "AI-assisted: {}", label(first));
            if entries
                .iter()
                .any(|e| e.kind == AttrKind::Assist && e.agent != first.agent)
            {
                view.text.push_str(" and others");
            }
        }
        Some(TextRange {
            start,
            end: view.text.len() as u32,
        })
    }
}

/// Most agents listed per [`AttrTotals`].
pub const STATS_MAX_AGENTS: usize = 16;
/// Most models listed per [`AgentStats`].
pub const STATS_MAX_MODELS: usize = 8;

/// Commits credited to one model of an agent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelStats {
    pub model: String,
    pub commits: u32,
}

/// Commits credited to one agent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentStats {
    pub id: String,
    /// Display name ([`agents::resolve`]).
    pub name: String,
    pub colour: [u8; 3],
    /// Commits crediting the agent (assisted or authored).
    pub commits: u32,
    /// Commits the agent authored.
    pub as_author: u32,
    /// Top [`STATS_MAX_MODELS`] models by commits, then text.
    pub models: Vec<ModelStats>,
    /// Commits credited to models that are not listed.
    pub other_models: u32,
}

/// Aggregate attribution numbers over a set of commits.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AttrTotals {
    pub commits: u32,
    /// Commits with an Assist entry.
    pub assisted: u32,
    /// Commits with an Agent entry.
    pub agent: u32,
    /// Commits with a Bot entry.
    pub bot: u32,
    /// Top [`STATS_MAX_AGENTS`] agents by commits, then id.
    pub agents: Vec<AgentStats>,
    /// Agents that are not listed.
    pub more_agents: u32,
}

/// `ai_stats()`: the whole loaded history and the played part (AI-33).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AttrStats {
    pub registry: u32,
    pub loaded: AttrTotals,
    /// Distinct non-empty attribution sets of the log.
    pub sets: u32,
    pub played: AttrTotals,
}

/// Totals over `counts` (commits per set id of `log`).
fn totals(log: &EventLog, counts: &[u32]) -> AttrTotals {
    #[derive(Default)]
    struct Agg {
        commits: u32,
        as_author: u32,
        models: BTreeMap<StrId, u32>,
    }
    let mut out = AttrTotals::default();
    let mut aggs: BTreeMap<StrId, Agg> = BTreeMap::new();
    for (i, &count) in counts.iter().enumerate() {
        out.commits = out.commits.saturating_add(count);
        if i == 0 || count == 0 {
            continue;
        }
        let entries = log.attr_set(AttrSetId(i as u32));
        let has = |k| entries.iter().any(|e| e.kind == k);
        let add = |v: &mut u32, yes: bool| {
            if yes {
                *v = v.saturating_add(count);
            }
        };
        add(&mut out.assisted, has(AttrKind::Assist));
        add(&mut out.agent, has(AttrKind::Agent));
        add(&mut out.bot, has(AttrKind::Bot));
        for (n, e) in entries.iter().enumerate() {
            if !matches!(e.kind, AttrKind::Assist | AttrKind::Agent) {
                continue;
            }
            let earlier = &entries[..n];
            let ai = |p: &&crate::events::Attribution| {
                matches!(p.kind, AttrKind::Assist | AttrKind::Agent) && p.id == e.id
            };
            let agg = aggs.entry(e.id).or_default();
            if !earlier.iter().any(|p| ai(&p)) {
                add(&mut agg.commits, true);
            }
            if e.kind == AttrKind::Agent
                && !earlier.iter().any(|p| ai(&p) && p.kind == AttrKind::Agent)
            {
                add(&mut agg.as_author, true);
            }
            if let Some(m) = e.model
                && !earlier.iter().any(|p| ai(&p) && p.model == Some(m))
            {
                let c = agg.models.entry(m).or_insert(0);
                *c = c.saturating_add(count);
            }
        }
    }

    let mut agents: Vec<AgentStats> = aggs
        .into_iter()
        .map(|(id, agg)| {
            let id = log.str(id);
            let display = agents::resolve(id);
            let mut models: Vec<ModelStats> = agg
                .models
                .into_iter()
                .map(|(m, commits)| ModelStats {
                    model: log.str(m).to_string(),
                    commits,
                })
                .collect();
            models.sort_by(|a, b| {
                b.commits
                    .cmp(&a.commits)
                    .then_with(|| a.model.cmp(&b.model))
            });
            let other_models = models
                .iter()
                .skip(STATS_MAX_MODELS)
                .fold(0u32, |s, m| s.saturating_add(m.commits));
            models.truncate(STATS_MAX_MODELS);
            AgentStats {
                id: id.to_string(),
                name: display.name.to_string(),
                colour: display.tint,
                commits: agg.commits,
                as_author: agg.as_author,
                models,
                other_models,
            }
        })
        .collect();
    agents.sort_by(|a, b| b.commits.cmp(&a.commits).then_with(|| a.id.cmp(&b.id)));
    out.more_agents = agents.len().saturating_sub(STATS_MAX_AGENTS) as u32;
    agents.truncate(STATS_MAX_AGENTS);
    out.agents = agents;
    out
}

impl Simulation {
    /// Attribution numbers of the loaded history and of the commits played
    /// up to the playback position (AI-33; not meant for every frame).
    pub fn attr_stats(&self, log: &EventLog) -> AttrStats {
        AttrStats {
            registry: agents::REGISTRY_VERSION,
            loaded: totals(log, log.attr_commit_counts()),
            sets: (log.attr_set_count() - 1) as u32,
            played: totals(log, &self.played_set_counts),
        }
    }
}

#[cfg(test)]
#[path = "attr_tests.rs"]
mod tests;
