// SPDX-License-Identifier: GPL-3.0-or-later
//! Rich PR truth folded in frozen stream order, independent of beams and styles.
//! Plain/L0 changes never enter this table. Provider truth remains a producer contract.
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;

use crate::events::{
    ChangeEvent, EventLog, PrCompleteness, PrLifecycle, PrLogCoverage, PrPayload, PrState, StrId,
};
use crate::file::FileId;

pub const PR_TERMINAL_SECONDS: f32 = 2.0;
/// A bounded imported model, above the supported 10k-proposal stress case.
const MAX_PR_NODES: usize = 100_000;
pub(crate) type PrKey = (StrId, u64);

/// Wire tables may contain duplicate text at distinct IDs. Normalize typed
/// semantic identity only, without rewriting the immutable imported log.
pub(crate) fn canonical_id(log: &EventLog, id: StrId) -> StrId {
    log.lookup(log.str(id)).unwrap_or(id)
}

/// Explicit producer receipt only. The core cannot establish provider truth.
#[derive(Debug, Default)]
pub(crate) struct PrReceipts {
    bindings: BTreeMap<PrKey, Option<(StrId, StrId)>>,
    files: BTreeMap<(FileId, PrKey), (StrId, StrId)>,
}

impl PrReceipts {
    pub(crate) fn clear(&mut self) {
        self.bindings.clear();
        self.files.clear();
    }

    pub(crate) fn register(&mut self, file: FileId, key: PrKey, repository: StrId, sha: StrId) {
        let qualifier = (repository, sha);
        if self.bindings.len() >= MAX_PR_NODES && !self.bindings.contains_key(&key) {
            return;
        }
        let binding = self.bindings.entry(key).or_insert(Some(qualifier));
        if *binding != Some(qualifier) {
            *binding = None; // conflicting repository/SHA evidence stays ambiguous
            return;
        }
        if self.files.len() < MAX_PR_NODES {
            self.files.insert((file, key), qualifier);
        }
    }

    pub(crate) fn age(&self, file: FileId, table: &PrTable, runtime: f32) -> Option<f32> {
        self.files
            .range((file, (StrId(0), 0))..=(file, (StrId(u32::MAX), u64::MAX)))
            .filter_map(|((_, key), qualifier)| {
                if self.bindings.get(key) != Some(&Some(*qualifier)) {
                    return None;
                }
                let node = table.node(*key)?;
                if node.phase != PrPhase::Merged {
                    return None;
                }
                node.terminal_age(runtime)
                    .filter(|age| *age < PR_TERMINAL_SECONDS)
            })
            .min_by(|a, b| a.total_cmp(b))
    }

    pub(crate) fn remove_file(&mut self, file: FileId) {
        // Removal happens at event/expiry boundaries, not in extraction.
        while let Some(key) = self
            .files
            .range((file, (StrId(0), 0))..=(file, (StrId(u32::MAX), u64::MAX)))
            .next()
            .map(|(key, _)| *key)
        {
            self.files.remove(&key);
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PrPhase {
    Open,
    Draft,
    Merged,
    Closed,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PrNode {
    pub path: StrId,
    pub number: u64,
    pub owner: StrId,
    pub created: i64,
    pub phase: PrPhase,
    pub review: Option<ChangeEvent>,
    pub actor: StrId,
    pub target: Option<StrId>,
    pub generation: u64,
    pub terminal_started: Option<f32>,
}

impl PrNode {
    pub fn age_days(&self, replay_time: i64) -> u64 {
        replay_time.saturating_sub(self.created).max(0) as u64 / 86_400
    }

    pub fn terminal_age(&self, animation_runtime: f32) -> Option<f32> {
        let start = self.terminal_started?;
        if !animation_runtime.is_finite() || !start.is_finite() || animation_runtime < start {
            return None;
        }
        let elapsed = animation_runtime - start;
        elapsed.is_finite().then_some(elapsed)
    }

    pub(crate) fn is_open(&self) -> bool {
        matches!(self.phase, PrPhase::Open | PrPhase::Draft)
    }

    pub(crate) fn visible(&self, runtime: f32) -> bool {
        self.is_open()
            || self
                .terminal_age(runtime)
                .is_some_and(|age| age < PR_TERMINAL_SECONDS)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PrFlowStats {
    pub open: u64,
    pub oldest_days: Option<u64>,
}

#[derive(Copy, Clone, Debug)]
struct Entry {
    node: PrNode,
    open_phase: PrPhase,
    blocked: bool,
}

#[derive(Debug)]
pub(crate) struct PrTable {
    nodes: BTreeMap<PrKey, Entry>,
    path_aliases: BTreeMap<StrId, StrId>,
    code_paths: BTreeSet<Box<str>>,
    scanned_commits: usize,
    causal_complete: bool,
    pub(crate) new_proposals: u64,
}

impl Default for PrTable {
    fn default() -> Self {
        Self {
            nodes: BTreeMap::new(),
            path_aliases: BTreeMap::new(),
            code_paths: BTreeSet::new(),
            scanned_commits: 0,
            causal_complete: true,
            new_proposals: 0,
        }
    }
}

fn code_collision(paths: &BTreeSet<Box<str>>, path: &str) -> bool {
    // A real file can be the marker itself, one of its ancestors, or a descendant.
    // Real code directories are not collisions: proposals are anchored under them.
    if paths.contains(path)
        || path
            .rmatch_indices('/')
            .any(|(at, _)| at != 0 && paths.contains(&path[..at]))
    {
        return true;
    }
    // Start at the exact directory prefix, not the marker: a lexical sibling
    // such as `marker-extra` can sort before `marker/real`.
    let prefix = format!("{path}/");
    paths
        .range::<str, _>((Bound::Included(prefix.as_str()), Bound::Unbounded))
        .next()
        .is_some_and(|p| p.starts_with(prefix.as_str()))
}

impl PrTable {
    pub(crate) fn clear(&mut self) {
        self.nodes.clear();
        self.path_aliases.clear();
        self.code_paths.clear();
        self.scanned_commits = 0;
        self.causal_complete = true;
        self.new_proposals = 0;
    }

    /// Inspect frozen path definitions, not future lifecycle state. Appends extend it.
    pub(crate) fn sync_code_paths(&mut self, log: &EventLog) {
        if self.scanned_commits == log.commit_count() {
            return;
        }
        for index in self.scanned_commits..log.commit_count() {
            let first = log.records()[index].first_change as usize;
            for (offset, change) in log.commit(index).changes.iter().enumerate() {
                if matches!(log.pr_change(first + offset), Some(PrPayload::Lifecycle(_))) {
                    continue;
                }
                let path = log.str(change.path);
                if !path.ends_with('/') {
                    self.code_paths.insert(path.into());
                }
            }
        }
        self.scanned_commits = log.commit_count();
        for entry in self.nodes.values_mut() {
            if code_collision(&self.code_paths, log.str(entry.node.path)) {
                entry.blocked = true;
                self.causal_complete = false;
            }
        }
    }

    pub(crate) fn nodes(&self) -> impl ExactSizeIterator<Item = &PrNode> {
        self.nodes.values().map(|entry| &entry.node)
    }

    pub(crate) fn by_path(&self, path: StrId) -> Option<&PrNode> {
        let path = self.path_aliases.get(&path).copied().unwrap_or(path);
        self.nodes
            .range((path, 0)..=(path, u64::MAX))
            .next()
            .map(|(_, entry)| &entry.node)
    }

    pub(crate) fn node(&self, key: PrKey) -> Option<&PrNode> {
        self.nodes
            .get(&key)
            .filter(|entry| !entry.blocked)
            .map(|entry| &entry.node)
    }

    /// Called only for a due consumed change, or explicitly for an actionless prefix.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn consume(
        &mut self,
        log: &EventLog,
        path: StrId,
        event: ChangeEvent,
        mut payload: PrLifecycle,
        actor: StrId,
        timestamp: i64,
        runtime: Option<f32>,
    ) -> Option<PrKey> {
        let canonical_path = canonical_id(log, path);
        if path != canonical_path && self.path_aliases.len() < MAX_PR_NODES {
            self.path_aliases.insert(path, canonical_path);
        }
        let path = canonical_path;
        payload.owner = canonical_id(log, payload.owner);
        payload.target = payload.target.map(|target| canonical_id(log, target));
        let actor = canonical_id(log, actor);
        let key = (path, payload.number);
        if payload.created > timestamp || code_collision(&self.code_paths, log.str(path)) {
            self.causal_complete = false;
            return None;
        }
        let initial_phase = match payload.state {
            PrState::Open => PrPhase::Open,
            PrState::Draft => PrPhase::Draft,
        };
        if !self.nodes.contains_key(&key) {
            if !matches!(
                event,
                ChangeEvent::PrOpened
                    | ChangeEvent::PrReopened
                    | ChangeEvent::PrMerged
                    | ChangeEvent::PrClosed
            ) || self.nodes.len() >= MAX_PR_NODES
                || self.by_path(path).is_some()
            {
                self.causal_complete = false;
                return None;
            }
            if event != ChangeEvent::PrOpened {
                // A valid typed reopen can restore current context, but not missing history.
                self.causal_complete = false;
            }
            self.nodes.insert(
                key,
                Entry {
                    node: PrNode {
                        path,
                        number: payload.number,
                        owner: payload.owner,
                        created: payload.created,
                        phase: initial_phase,
                        review: None,
                        actor,
                        target: None,
                        generation: 0,
                        terminal_started: None,
                    },
                    open_phase: initial_phase,
                    blocked: false,
                },
            );
            if event == ChangeEvent::PrOpened && !payload.initial && runtime.is_some() {
                self.new_proposals = self.new_proposals.saturating_add(1);
            }
        }
        let entry = self.nodes.get_mut(&key)?;
        if entry.blocked
            || entry.node.owner != payload.owner
            || entry.node.created != payload.created
        {
            self.causal_complete = false;
            return None;
        }
        // Provider merge transports can carry a close companion. Merged stays merged.
        if event == ChangeEvent::PrClosed && entry.node.phase == PrPhase::Merged {
            return Some(key);
        }
        if event == ChangeEvent::PrReopened && entry.node.phase == PrPhase::Merged {
            self.causal_complete = false;
            return None;
        }
        if event == ChangeEvent::Commit
            || (event == ChangeEvent::PrOpened && !entry.node.is_open() && !payload.initial)
            || (!entry.node.is_open()
                && matches!(
                    event,
                    ChangeEvent::PrReady
                        | ChangeEvent::PrReviewRequested
                        | ChangeEvent::ReviewApproved
                        | ChangeEvent::ReviewChangesRequested
                        | ChangeEvent::ReviewCommented
                ))
        {
            self.causal_complete = false;
            return None;
        }
        entry.node.generation = entry.node.generation.saturating_add(1);
        entry.node.actor = actor;
        entry.node.target = payload.target;
        match event {
            ChangeEvent::PrOpened => {
                entry.node.phase = if payload.initial {
                    initial_phase
                } else {
                    entry.open_phase
                };
                entry.open_phase = entry.node.phase;
                entry.node.terminal_started = None;
            }
            ChangeEvent::PrReopened => {
                entry.node.phase = entry.open_phase;
                entry.node.terminal_started = None;
            }
            ChangeEvent::PrReady => {
                if entry.node.is_open() {
                    entry.node.phase = PrPhase::Open;
                    entry.open_phase = PrPhase::Open;
                }
            }
            ChangeEvent::PrMerged | ChangeEvent::PrClosed => {
                entry.node.phase = if event == ChangeEvent::PrMerged {
                    PrPhase::Merged
                } else {
                    PrPhase::Closed
                };
                entry.node.terminal_started = runtime.filter(|t| t.is_finite() && *t >= 0.0);
            }
            ChangeEvent::ReviewApproved
            | ChangeEvent::ReviewChangesRequested
            | ChangeEvent::ReviewCommented => {
                entry.node.review = Some(event);
            }
            ChangeEvent::PrReviewRequested => {}
            ChangeEvent::Commit => return None,
        }
        Some(key)
    }

    pub(crate) fn flow_stats(&self, log: &EventLog, replay_time: i64) -> Option<PrFlowStats> {
        let PrLogCoverage::Known(coverage) = log.pr_coverage() else {
            return None;
        };
        if !log.has_pr_lifecycle()
            || !self.causal_complete
            || !coverage.is_valid()
            || coverage.lifecycle != PrCompleteness::Complete
            || replay_time < coverage.from
            || replay_time >= coverage.to
        {
            return None;
        }
        let mut stats = PrFlowStats {
            open: 0,
            oldest_days: None,
        };
        for node in self.nodes() {
            if node.is_open() {
                stats.open = stats.open.saturating_add(1);
                let age = node.age_days(replay_time);
                stats.oldest_days = Some(stats.oldest_days.map_or(age, |old| old.max(age)));
            }
        }
        Some(stats)
    }
}
