// SPDX-License-Identifier: GPL-3.0-or-later

//! Immutable, repository-scoped rich PR anchors (RPA-12).
//!
//! This public GPL adapter consumes already-sanitised snapshot data, not provider
//! objects, identities or events. Supply every ordinary code path in the entire
//! frozen log, including future additions and deletions. A trailing `/` marks a
//! directory operation, not a real file. Changed paths are file paths and their
//! completeness must be explicit. Placement from a snapshot diff is not a claim
//! about the historically exact opening diff.
//!
//! Build once per immutable log, then look up the same owned path/title for every
//! edit, close and reopen. A new snapshot may construct a different table. This
//! module never mutates code, emits events, resolves identities or changes L0
//! [`crate::pulls::node_path`]. Numbers are local to one repository/log.

use std::collections::{BTreeMap, BTreeSet};

use gource_core::events::PrCompleteness;

use crate::pulls::MAX_TITLE_CHARS;

/// Maximum PR definitions in one frozen log, checked before building the table.
pub const MAX_PULL_REQUESTS: usize = 100_000;
/// Maximum code path entries, before deduplication, from the whole frozen log.
pub const MAX_CODE_PATHS: usize = 1_000_000;
/// Maximum changed path entries per PR, before deduplication.
/// This is a local safety bound, not a provider completeness guarantee.
pub const MAX_CHANGED_PATHS: usize = 100_000;
/// Maximum changed path entries across all PR definitions, before deduplication.
pub const MAX_TOTAL_CHANGED_PATHS: usize = 1_000_000;
/// Maximum bytes in each incoming path, including any leading/trailing slashes.
pub const MAX_PATH_BYTES: usize = 4096;
/// Maximum bytes in each incoming title, before the 60-character sanitisation.
pub const MAX_TITLE_INPUT_BYTES: usize = 16_384;
/// Maximum frozen node path bytes, matching the typed PR label limit.
pub const MAX_NODE_PATH_BYTES: usize = 512;

const REGION: &str = "/pull requests";
const SYNTHETIC_DIR: &str = ".pull-requests";

/// One immutable PR definition, without actors, provider fields or lifecycle.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PrAnchorInput<'a> {
    /// Positive repository-local PR number. Duplicate definitions are rejected.
    pub number: u64,
    /// Display title. Delimiter/control safety is enforced again at this seam.
    pub title: &'a str,
    /// Relative or slash-rooted file paths. Equivalent leading slashes dedupe.
    pub changed_paths: &'a [&'a str],
    /// Partial/unknown lists never influence the selected code directory.
    pub completeness: PrCompleteness,
}

/// Why a frozen node is in its code area or in the fallback region.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AnchorPlacement {
    StrictMajority,
    /// Common directory of all files, if no strict majority was found.
    /// Counting all ancestors normally makes a non-root common directory itself
    /// a strict majority, but the common-ancestor fallback remains explicit.
    CommonAncestor,
    RootFallback,
    IncompleteFallback,
    UnknownFallback,
    /// A real code file at/above the selected anchor prevents descendants.
    CodeFileFallback,
}

/// Owned frozen display data. The table only exposes shared references.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrozenPrAnchor {
    pub number: u64,
    /// Selected code directory, or the actual (possibly suffixed) fallback region.
    /// No trailing slash. A suffixed synthetic child does not change this anchor.
    pub anchor: String,
    /// Sanitised title, at most 60 Unicode scalar values including any ellipsis.
    pub title: String,
    pub path: String,
    pub placement: AnchorPlacement,
}

/// An immutable table scoped to one repository's entire frozen log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrozenPrAnchors {
    nodes: BTreeMap<u64, FrozenPrAnchor>,
}

/// Safe failures contain only bounded counts/indices or numeric PR identifiers,
/// never raw paths, titles, provider data or credentials. Construction is atomic.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PrAnchorError {
    InvalidNumber,
    DuplicateNumber { number: u64 },
    TooManyPullRequests,
    TooManyCodePaths,
    TooManyChangedPaths { number: u64 },
    TooManyTotalChangedPaths,
    TitleTooLong { number: u64 },
    InvalidCodePath { index: usize },
    InvalidChangedPath { number: u64, index: usize },
    NodePathTooLong { number: u64 },
    SyntheticDirectoryExhausted { number: u64 },
}

impl std::fmt::Display for PrAnchorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidNumber => f.write_str("PR number must be positive"),
            Self::DuplicateNumber { number } => write!(f, "duplicate PR definition #{number}"),
            Self::TooManyPullRequests => f.write_str("PR definition count exceeds limit"),
            Self::TooManyCodePaths => f.write_str("frozen code path count exceeds limit"),
            Self::TooManyChangedPaths { number } => {
                write!(f, "changed path count exceeds limit for PR #{number}")
            }
            Self::TooManyTotalChangedPaths => f.write_str("total changed path count exceeds limit"),
            Self::TitleTooLong { number } => {
                write!(f, "title input exceeds limit for PR #{number}")
            }
            Self::InvalidCodePath { index } => {
                write!(f, "invalid frozen code path at index {index}")
            }
            Self::InvalidChangedPath { number, index } => {
                write!(f, "invalid changed path at index {index} for PR #{number}")
            }
            Self::NodePathTooLong { number } => {
                write!(f, "node path exceeds limit for PR #{number}")
            }
            Self::SyntheticDirectoryExhausted { number } => {
                write!(f, "no unused synthetic directory for PR #{number}")
            }
        }
    }
}

impl std::error::Error for PrAnchorError {}

impl FrozenPrAnchors {
    /// Freeze one unambiguous definition per number using all code paths from the
    /// whole log. Reject malformed paths rather than guessing a placement.
    ///
    /// Path components must not be empty, `.` or `..`, or contain `|`, backslashes,
    /// control/bidi characters or Unicode line separators. Code directory entries
    /// may end in `/` (including the root `/`), but changed file paths may not.
    ///
    /// Each synthetic-directory search makes at most `deduped_code_paths + 1`
    /// attempts, with ordered-set lookups rather than rescanning the entire log.
    /// A real ancestor file triggers region fallback before that search.
    pub fn new(prs: &[PrAnchorInput<'_>], all_code_paths: &[&str]) -> Result<Self, PrAnchorError> {
        validate_bounds(prs, all_code_paths)?;
        let code = CodePaths::new(all_code_paths)?;
        let mut nodes = BTreeMap::new();
        for pr in prs {
            let mut changed = BTreeSet::new();
            for (index, path) in pr.changed_paths.iter().enumerate() {
                let path =
                    normalise_path(path, false).ok_or(PrAnchorError::InvalidChangedPath {
                        number: pr.number,
                        index,
                    })?;
                changed.insert(path);
            }
            let (mut anchor, mut placement) = choose_anchor(&changed, pr.completeness);
            let mut region = !matches!(
                placement,
                AnchorPlacement::StrictMajority | AnchorPlacement::CommonAncestor
            );
            if !region && code.file_ancestor(&anchor) {
                anchor = REGION.to_owned();
                placement = AnchorPlacement::CodeFileFallback;
                region = true;
            }
            let directory = code
                .unused_directory(&anchor, region)
                .ok_or(PrAnchorError::SyntheticDirectoryExhausted { number: pr.number })?;
            if region {
                anchor.clone_from(&directory);
            }
            let title = sanitise_title(pr.title);
            let mut path = format!("{directory}/#{}", pr.number);
            if !title.is_empty() {
                path.push(' ');
                path.push_str(&title);
            }
            if path.len() > MAX_NODE_PATH_BYTES {
                return Err(PrAnchorError::NodePathTooLong { number: pr.number });
            }
            nodes.insert(
                pr.number,
                FrozenPrAnchor {
                    number: pr.number,
                    anchor,
                    title,
                    path,
                    placement,
                },
            );
        }
        Ok(Self { nodes })
    }

    /// The frozen definition. No event/update operation can replace its title/path.
    pub fn get(&self, number: u64) -> Option<&FrozenPrAnchor> {
        self.nodes.get(&number)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

fn validate_bounds(prs: &[PrAnchorInput<'_>], code: &[&str]) -> Result<(), PrAnchorError> {
    if prs.len() > MAX_PULL_REQUESTS {
        return Err(PrAnchorError::TooManyPullRequests);
    }
    if code.len() > MAX_CODE_PATHS {
        return Err(PrAnchorError::TooManyCodePaths);
    }
    let mut numbers = BTreeSet::new();
    let mut total = 0usize;
    for pr in prs {
        if pr.number == 0 {
            return Err(PrAnchorError::InvalidNumber);
        }
        if !numbers.insert(pr.number) {
            return Err(PrAnchorError::DuplicateNumber { number: pr.number });
        }
        if pr.title.len() > MAX_TITLE_INPUT_BYTES {
            return Err(PrAnchorError::TitleTooLong { number: pr.number });
        }
        if pr.changed_paths.len() > MAX_CHANGED_PATHS {
            return Err(PrAnchorError::TooManyChangedPaths { number: pr.number });
        }
        total = total
            .checked_add(pr.changed_paths.len())
            .filter(|&n| n <= MAX_TOTAL_CHANGED_PATHS)
            .ok_or(PrAnchorError::TooManyTotalChangedPaths)?;
    }
    Ok(())
}

fn unsafe_character(c: char) -> bool {
    c.is_control()
        || matches!(c,
        '\u{61c}' | '\u{200e}' | '\u{200f}' | '\u{2028}' | '\u{2029}'
        | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

fn normalise_path(path: &str, directory: bool) -> Option<String> {
    if path.is_empty()
        || path.len() > MAX_PATH_BYTES
        || path
            .chars()
            .any(|c| unsafe_character(c) || matches!(c, '|' | '\\'))
    {
        return None;
    }
    let mut relative = path.trim_start_matches('/');
    if directory && path.ends_with('/') {
        if relative.is_empty() {
            return Some("/".to_owned());
        }
        relative = relative.strip_suffix('/')?;
    }
    if relative
        .split('/')
        .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return None;
    }
    Some(format!("/{relative}"))
}

fn sanitise_title(title: &str) -> String {
    let clean: String = title
        .chars()
        .filter(|&c| !unsafe_character(c))
        .map(|c| {
            if matches!(c, '|' | '/' | '\\') {
                '-'
            } else {
                c
            }
        })
        .collect();
    let clean = clean.trim();
    if clean.chars().count() <= MAX_TITLE_CHARS {
        return clean.to_owned();
    }
    let shortened: String = clean.chars().take(MAX_TITLE_CHARS - 1).collect();
    format!("{}…", shortened.trim_end())
}

fn choose_anchor(
    files: &BTreeSet<String>,
    completeness: PrCompleteness,
) -> (String, AnchorPlacement) {
    match completeness {
        PrCompleteness::Partial => return (REGION.to_owned(), AnchorPlacement::IncompleteFallback),
        PrCompleteness::Unknown => return (REGION.to_owned(), AnchorPlacement::UnknownFallback),
        PrCompleteness::Complete => {}
    }
    if files.is_empty() {
        return (REGION.to_owned(), AnchorPlacement::UnknownFallback);
    }
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for path in files {
        for (index, _) in path.match_indices('/').filter(|&(index, _)| index > 0) {
            *counts.entry(&path[..index]).or_default() += 1;
        }
    }
    if let Some((&directory, _)) = counts
        .iter()
        .filter(|&(_, &count)| count > files.len() / 2)
        .max_by_key(|&(directory, _)| directory.len())
    {
        return (directory.to_owned(), AnchorPlacement::StrictMajority);
    }
    // No non-root majority: explicitly compute the common directory ancestor.
    let mut common: Option<Vec<&str>> = None;
    for path in files {
        let parts: Vec<&str> = path[1..].split('/').collect();
        let directory = &parts[..parts.len() - 1];
        if let Some(common) = &mut common {
            let length = common
                .iter()
                .zip(directory)
                .take_while(|(a, b)| a == b)
                .count();
            common.truncate(length);
        } else {
            common = Some(directory.to_vec());
        }
    }
    match common.filter(|parts| !parts.is_empty()) {
        Some(parts) => (
            format!("/{}", parts.join("/")),
            AnchorPlacement::CommonAncestor,
        ),
        None => (REGION.to_owned(), AnchorPlacement::RootFallback),
    }
}

struct CodePaths {
    all: BTreeSet<String>,
    files: BTreeSet<String>,
}

impl CodePaths {
    fn new(paths: &[&str]) -> Result<Self, PrAnchorError> {
        let mut all = BTreeSet::new();
        let mut files = BTreeSet::new();
        for (index, path) in paths.iter().enumerate() {
            let directory = path.ends_with('/');
            let normalised =
                normalise_path(path, directory).ok_or(PrAnchorError::InvalidCodePath { index })?;
            if !directory {
                files.insert(normalised.clone());
            }
            all.insert(normalised);
        }
        Ok(Self { all, files })
    }

    fn file_ancestor(&self, path: &str) -> bool {
        self.files.contains(path)
            || path
                .match_indices('/')
                .filter(|&(index, _)| index > 0)
                .any(|(index, _)| self.files.contains(&path[..index]))
    }

    fn directory_used(&self, directory: &str) -> bool {
        if self.all.contains(directory) {
            return true;
        }
        let prefix = format!("{directory}/");
        self.all
            .range(prefix.clone()..)
            .next()
            .is_some_and(|path| path.starts_with(&prefix))
    }

    fn unused_directory(&self, anchor: &str, region: bool) -> Option<String> {
        let base = if region {
            REGION.to_owned()
        } else {
            format!("{anchor}/{SYNTHETIC_DIR}")
        };
        // Once real ancestor files are removed, each code entry can occupy at
        // most one candidate. N+1 attempts are therefore sufficient and finite.
        for suffix in 1..=self.all.len() + 1 {
            let directory = if suffix == 1 {
                base.clone()
            } else {
                format!("{base}-{suffix}")
            };
            if !self.file_ancestor(&directory) && !self.directory_used(&directory) {
                return Some(directory);
            }
        }
        None
    }
}
