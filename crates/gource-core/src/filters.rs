// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! The commit filters of `formats/commitlog.cpp` (`RCommit::addFile` and
//! `RCommit::isValid`): `--file-filter` / `--file-show-filter` drop file
//! changes, `--user-filter` / `--user-show-filter` drop whole commits, and a
//! commit left without files is invalid.
//!
//! Rules kept from the C++:
//! - a path or user matching any `*-filter` is rejected;
//! - with `*-show-filter`s, a path or user must match **every** one of them
//!   (the C++ rejects on the first show filter that does not match);
//! - matching is unanchored (`pcre2_match` from the start of the string).
//!
//! Differences:
//! - regular expressions use `regex-lite` instead of PCRE2 (the common
//!   syntax is the same; look-around and backreferences are not available);
//! - file filters see the path without its leading `/`. Gource filters the
//!   path as the log reader produced it, before `RCommitFile` roots it; for
//!   git (the source of the web MVP) that is the repository-relative path.
//!   Custom logs that spell out a leading `/` were matched with it in Gource.
//!
//! Patterns are compiled once per filter change ([`Filters::sync`]) and the
//! verdict for each interned string is cached per [`StrId`].
//!
//! Port addition, `ai-filter` (AI-34): `only-assisted` keeps only commits
//! whose attribution set has an Assist or Agent entry, `exclude-assisted`
//! drops them (Bot entries count as neither). It composes with the other
//! filters by AND and reads the commit itself, so nothing is cached.

use regex_lite::Regex;

use crate::events::{AttrKind, AttrSetId, EventLog, StrId};
use crate::settings::{AiFilter, Settings};

const UNKNOWN: u8 = 0;
const PASS: u8 = 1;
const REJECT: u8 = 2;

#[derive(Clone, Debug, Default)]
struct FilterSet {
    sources: Vec<String>,
    show_sources: Vec<String>,
    filters: Vec<Regex>,
    show_filters: Vec<Regex>,
    cache: Vec<u8>,
}

impl FilterSet {
    fn sync(&mut self, filters: &[String], show_filters: &[String]) {
        if self.sources == filters && self.show_sources == show_filters {
            return;
        }
        self.sources = filters.to_vec();
        self.show_sources = show_filters.to_vec();
        // Invalid patterns were rejected by `Settings::apply_option`; any
        // that slip through (direct field edits) are ignored.
        self.filters = filters.iter().filter_map(|f| Regex::new(f).ok()).collect();
        self.show_filters = show_filters
            .iter()
            .filter_map(|f| Regex::new(f).ok())
            .collect();
        self.cache.clear();
    }

    fn is_empty(&self) -> bool {
        self.filters.is_empty() && self.show_filters.is_empty()
    }

    fn matches(&self, s: &str) -> bool {
        if self.filters.iter().any(|r| r.is_match(s)) {
            return false;
        }
        self.show_filters.iter().all(|r| r.is_match(s))
    }

    /// [`FilterSet::passes`] without filling the cache.
    fn passes_ro(&self, id: StrId, s: &str) -> bool {
        if self.is_empty() {
            return true;
        }
        match self.cache.get(id.0 as usize).copied().unwrap_or(UNKNOWN) {
            PASS => true,
            REJECT => false,
            _ => self.matches(s),
        }
    }

    fn passes(&mut self, id: StrId, s: &str) -> bool {
        if self.is_empty() {
            return true;
        }
        let i = id.0 as usize;
        if i >= self.cache.len() {
            self.cache.resize(i + 1, UNKNOWN);
        }
        match self.cache[i] {
            PASS => true,
            REJECT => false,
            _ => {
                let pass = self.matches(s);
                self.cache[i] = if pass { PASS } else { REJECT };
                pass
            }
        }
    }
}

/// Compiled filters with per-string verdict caches.
#[derive(Clone, Debug, Default)]
pub(crate) struct Filters {
    files: FilterSet,
    users: FilterSet,
    ai: AiFilter,
}

impl Filters {
    /// Recompile when the settings' filter lists changed.
    pub(crate) fn sync(&mut self, settings: &Settings) {
        self.files
            .sync(&settings.file_filters, &settings.file_show_filters);
        self.users
            .sync(&settings.user_filters, &settings.user_show_filters);
        self.ai = settings.ai_filter;
    }

    /// The `ai-filter` test of a commit's attribution set.
    fn ai_passes(&self, log: &EventLog, attrs: AttrSetId) -> bool {
        let credited = || {
            log.attr_set(attrs)
                .iter()
                .any(|e| matches!(e.kind, AttrKind::Assist | AttrKind::Agent))
        };
        match self.ai {
            AiFilter::All => true,
            AiFilter::OnlyAssisted => credited(),
            AiFilter::ExcludeAssisted => !credited(),
        }
    }

    /// `RCommit::addFile`'s filter test for a path.
    pub(crate) fn file_passes(&mut self, log: &EventLog, path: StrId) -> bool {
        if self.files.is_empty() {
            return true;
        }
        let s = log.str(path);
        let s = s.strip_prefix('/').unwrap_or(s);
        self.files.passes(path, s)
    }

    /// `RCommit::isValid`'s user test.
    pub(crate) fn user_passes(&mut self, log: &EventLog, user: StrId) -> bool {
        if self.users.is_empty() {
            return true;
        }
        self.users.passes(user, log.str(user))
    }

    /// `nextCommit(commit, validate = true)` succeeds: the user passes and
    /// at least one file survives the file filters.
    pub(crate) fn commit_is_valid(&mut self, log: &EventLog, index: usize) -> bool {
        let commit = log.commit(index);
        if !self.ai_passes(log, commit.attrs) || !self.user_passes(log, commit.user) {
            return false;
        }
        if self.files.is_empty() {
            return !commit.changes.is_empty();
        }
        commit.changes.iter().any(|c| self.file_passes(log, c.path))
    }
}

impl Filters {
    /// [`Filters::commit_is_valid`] for `&self` callers: uses cached
    /// verdicts but does not add new ones.
    pub(crate) fn commit_is_valid_cached(&self, log: &EventLog, index: usize) -> bool {
        let commit = log.commit(index);
        if !self.ai_passes(log, commit.attrs)
            || !self.users.passes_ro(commit.user, log.str(commit.user))
        {
            return false;
        }
        commit.changes.iter().any(|c| {
            let s = log.str(c.path);
            self.files
                .passes_ro(c.path, s.strip_prefix('/').unwrap_or(s))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{ChangeEvent, ChangeKind, FileChange};

    fn log() -> EventLog {
        let mut log = EventLog::new();
        let andrew = log.intern("Andrew");
        let bob = log.intern("bob");
        let cpp = log.intern("/src/main.cpp");
        let h = log.intern("/src/main.h");
        let ch = |p| FileChange {
            path: p,
            kind: ChangeKind::Add,
            colour: None,
            event: ChangeEvent::Commit,
        };
        log.push_commit(1, andrew, [ch(cpp)], 0);
        log.push_commit(2, bob, [ch(h)], 10);
        log.push_commit(3, bob, [ch(cpp), ch(h)], 20);
        log
    }

    #[test]
    fn user_and_file_filters_follow_commitlog_rules() {
        let log = log();
        let mut s = Settings::default();
        let mut f = Filters::default();
        f.sync(&s);
        assert!((0..3).all(|i| f.commit_is_valid(&log, i)));

        s.user_filters = vec!["Andrew".into()];
        f.sync(&s);
        assert!(!f.commit_is_valid(&log, 0));
        assert!(f.commit_is_valid(&log, 1));

        s.user_filters.clear();
        s.file_filters = vec![r"\.cpp".into()];
        f.sync(&s);
        assert!(!f.commit_is_valid(&log, 0));
        assert!(f.commit_is_valid(&log, 2));
        assert!(!f.file_passes(&log, log.lookup("/src/main.cpp").unwrap()));

        // show filters must all match; the path is seen without its '/'.
        s.file_filters.clear();
        s.file_show_filters = vec![r"^src/".into(), r"\.h$".into()];
        f.sync(&s);
        assert!(!f.commit_is_valid(&log, 0));
        assert!(f.commit_is_valid(&log, 1));
        assert!(f.file_passes(&log, log.lookup("/src/main.h").unwrap()));
        assert!(!f.file_passes(&log, log.lookup("/src/main.cpp").unwrap()));
    }

    #[test]
    fn ai_filter_reads_the_attribution_set() {
        use crate::events::Attribution;
        let mut log = log();
        let u = log.intern("jane");
        let p = log.lookup("/src/main.h").unwrap();
        let claude = log.intern("claude");
        let bot = log.intern("dependabot");
        let entry = |kind, id| Attribution {
            kind,
            id,
            model: None,
        };
        let ch = [FileChange {
            path: p,
            kind: ChangeKind::Modify,
            colour: None,
            event: ChangeEvent::Commit,
        }];
        let assist = log.intern_attr_set(&[entry(AttrKind::Assist, claude)]);
        let agent = log.intern_attr_set(&[entry(AttrKind::Agent, claude)]);
        let botset = log.intern_attr_set(&[entry(AttrKind::Bot, bot)]);
        log.push_commit_attr(4, u, ch, 30, assist);
        log.push_commit_attr(5, u, ch, 40, agent);
        log.push_commit_attr(6, u, ch, 50, botset);

        let mut s = Settings::default();
        let mut f = Filters::default();
        let verdicts = |f: &mut Filters| -> Vec<bool> {
            (0..6)
                .map(|i| {
                    let v = f.commit_is_valid(&log, i);
                    assert_eq!(v, f.commit_is_valid_cached(&log, i));
                    v
                })
                .collect()
        };
        f.sync(&s);
        assert_eq!(verdicts(&mut f), [true; 6]);
        s.ai_filter = AiFilter::OnlyAssisted;
        f.sync(&s);
        assert_eq!(verdicts(&mut f), [false, false, false, true, true, false]);
        s.ai_filter = AiFilter::ExcludeAssisted;
        s.user_filters = vec!["bob".into()];
        f.sync(&s);
        assert_eq!(verdicts(&mut f), [true, false, false, false, false, true]);
    }
}
