// SPDX-License-Identifier: GPL-3.0-or-later

//! The AI agent table: display name, tint and badge shape per agent id.
//!
//! Attributions in the event stream name agents by id (`claude`, `codex`,
//! ...). This table turns an id into what the scene shows. The detection
//! patterns that produce the ids live in `gource-sources` (`classify.rs`),
//! because they are a source concern; the table is here so that the client
//! can resolve ids the classifier never saw, such as ids written by hand into
//! a custom log.
//!
//! Tints come from the Okabe-Ito colour-blind-safe set where possible and are
//! not vendor colours; badge shapes tell similar tints apart. An id that is
//! not in [`AGENTS`] is still valid: [`resolve`] shows it under its own id,
//! with a tint from [`crate::hash`]'s colour hash and a hashed badge shape.

use crate::hash::{DEFAULT_HASH_SEED, colour_hash, string_hash};

/// Version of the detection registry. Bumped when what the classifier
/// detects changes (not for a name, tint or badge edit); a bump invalidates
/// every cached log on the server.
pub const REGISTRY_VERSION: u32 = 1;

/// Longest agent id in bytes.
pub const MAX_AGENT_ID: usize = 32;

/// Badge drawn next to an agent-authored contributor. The discriminant is
/// the cell order of the decoration atlas.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum BadgeShape {
    Circle = 0,
    Diamond = 1,
    Hexagon = 2,
    Triangle = 3,
    Square = 4,
    Star = 5,
    Ring = 6,
    Plus = 7,
    Drop = 8,
    Bolt = 9,
}

impl BadgeShape {
    pub const COUNT: usize = 10;

    pub const ALL: [BadgeShape; BadgeShape::COUNT] = [
        BadgeShape::Circle,
        BadgeShape::Diamond,
        BadgeShape::Hexagon,
        BadgeShape::Triangle,
        BadgeShape::Square,
        BadgeShape::Star,
        BadgeShape::Ring,
        BadgeShape::Plus,
        BadgeShape::Drop,
        BadgeShape::Bolt,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        match self {
            BadgeShape::Circle => "circle",
            BadgeShape::Diamond => "diamond",
            BadgeShape::Hexagon => "hexagon",
            BadgeShape::Triangle => "triangle",
            BadgeShape::Square => "square",
            BadgeShape::Star => "star",
            BadgeShape::Ring => "ring",
            BadgeShape::Plus => "plus",
            BadgeShape::Drop => "drop",
            BadgeShape::Bolt => "bolt",
        }
    }
}

/// One row of the agent table.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AgentInfo {
    /// Wire id, `[a-z0-9][a-z0-9-]{0,31}`.
    pub id: &'static str,
    /// Display name (plain text that identifies the tool).
    pub name: &'static str,
    pub tint: [u8; 3],
    pub badge: BadgeShape,
}

const fn agent(id: &'static str, name: &'static str, tint: u32, badge: BadgeShape) -> AgentInfo {
    AgentInfo {
        id,
        name,
        tint: [(tint >> 16) as u8, (tint >> 8) as u8, tint as u8],
        badge,
    }
}

/// Registry v1. The order is the registry index used for the canonical
/// order of attributions (`gource-sources`).
pub const AGENTS: &[AgentInfo] = &[
    agent("claude", "Claude", 0xE69F00, BadgeShape::Circle),
    agent("codex", "Codex", 0x56B4E9, BadgeShape::Diamond),
    agent("copilot", "GitHub Copilot", 0x009E73, BadgeShape::Hexagon),
    agent("cursor", "Cursor", 0xCC79A7, BadgeShape::Triangle),
    agent("aider", "Aider", 0xF0E442, BadgeShape::Square),
    agent("jules", "Jules", 0x0072B2, BadgeShape::Star),
    agent("devin", "Devin", 0xD55E00, BadgeShape::Ring),
    agent("amp", "Amp", 0xA78BFA, BadgeShape::Plus),
    agent("openhands", "OpenHands", 0x8BC34A, BadgeShape::Drop),
    agent("amazon-q", "Amazon Q Developer", 0xC0C0C0, BadgeShape::Bolt),
];

/// Registry index of `id`, if it is a known agent.
pub fn agent_index(id: &str) -> Option<usize> {
    AGENTS.iter().position(|a| a.id == id)
}

/// The table row of `id`, if it is a known agent.
pub fn find(id: &str) -> Option<&'static AgentInfo> {
    agent_index(id).map(|i| &AGENTS[i])
}

/// Whether `id` matches `[a-z0-9][a-z0-9-]{0,31}`.
pub fn is_valid_id(id: &str) -> bool {
    let b = id.as_bytes();
    !b.is_empty()
        && b.len() <= MAX_AGENT_ID
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

/// What the scene shows for an agent id.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AgentDisplay<'a> {
    pub id: &'a str,
    /// The table's display name, or the id itself for an unknown agent.
    pub name: &'a str,
    pub tint: [u8; 3],
    pub badge: BadgeShape,
    /// Whether the id is in [`AGENTS`].
    pub known: bool,
}

/// Resolve any valid agent id: a table row, or for an id that is not in
/// the table (custom logs, newer registries) the id as name, the colour
/// hash of the id as tint and `hash(id) % 10` as badge.
pub fn resolve(id: &str) -> AgentDisplay<'_> {
    match find(id) {
        Some(a) => AgentDisplay {
            id,
            name: a.name,
            tint: a.tint,
            badge: a.badge,
            known: true,
        },
        None => {
            let c = colour_hash(id.as_bytes(), DEFAULT_HASH_SEED);
            let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            let h = string_hash(id.as_bytes(), DEFAULT_HASH_SEED) as u32;
            AgentDisplay {
                id,
                name: id,
                tint: [to_u8(c.x), to_u8(c.y), to_u8(c.z)],
                badge: BadgeShape::ALL[h as usize % BadgeShape::COUNT],
                known: false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_ids_are_valid_and_unique() {
        for (i, a) in AGENTS.iter().enumerate() {
            assert!(is_valid_id(a.id), "{}", a.id);
            assert_eq!(agent_index(a.id), Some(i));
        }
        let mut seen = [false; BadgeShape::COUNT];
        for a in AGENTS {
            assert!(
                !seen[a.badge.index()],
                "{}: every agent has its own badge",
                a.id
            );
            seen[a.badge.index()] = true;
        }
    }

    #[test]
    fn id_grammar() {
        for ok in ["a", "0", "amazon-q", "x-1", &"a".repeat(32)] {
            assert!(is_valid_id(ok), "{ok}");
        }
        for bad in ["", "-a", "A", "a_b", "a b", "ä", &"a".repeat(33)] {
            assert!(!is_valid_id(bad), "{bad}");
        }
    }

    /// sRGB byte to linear light.
    fn linear(c: u8) -> f64 {
        let c = c as f64 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    /// CIE L*a*b* (D65) of linear RGB.
    fn lab(rgb: [f64; 3]) -> [f64; 3] {
        let [r, g, b] = rgb.map(|v| v.clamp(0.0, 1.0));
        let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
        let f = |t: f64| {
            if t > 216.0 / 24389.0 {
                t.cbrt()
            } else {
                (24389.0 / 27.0 * t + 16.0) / 116.0
            }
        };
        let (fx, fy, fz) = (f(x), f(y), f(z));
        [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
    }

    /// Machado, Oliveira and Fernandes (2009) matrices at severity 1.
    const DEFICIENCIES: [(&str, [[f64; 3]; 3]); 3] = [
        (
            "protanopia",
            [
                [0.152286, 1.052583, -0.204868],
                [0.114503, 0.786281, 0.099216],
                [-0.003882, -0.048116, 1.051998],
            ],
        ),
        (
            "deuteranopia",
            [
                [0.367322, 0.860646, -0.227968],
                [0.280085, 0.672501, 0.047413],
                [-0.011820, 0.042940, 0.968881],
            ],
        ),
        (
            "tritanopia",
            [
                [1.255528, -0.076749, -0.178779],
                [-0.078411, 0.930809, 0.147602],
                [0.004733, 0.691367, 0.303900],
            ],
        ),
    ];

    /// Pairs of agent tints closer than CIE76 DeltaE 15 under a simulated
    /// colour vision deficiency. Each relies on the badge shape and the
    /// name in text to tell the agents apart (AI-61, Q12).
    const CLOSE_PAIRS: &[(&str, &str, &str)] = &[
        ("protanopia", "claude", "openhands"),
        ("deuteranopia", "codex", "amp"),
    ];

    #[test]
    fn tints_stay_apart_under_colour_vision_deficiencies() {
        let mut close = Vec::new();
        for (name, m) in DEFICIENCIES {
            let sim = |t: [u8; 3]| {
                let c = t.map(linear);
                lab([0, 1, 2].map(|i| m[i][0] * c[0] + m[i][1] * c[1] + m[i][2] * c[2]))
            };
            for (i, a) in AGENTS.iter().enumerate() {
                for b in &AGENTS[i + 1..] {
                    let (la, lb) = (sim(a.tint), sim(b.tint));
                    let de = ((la[0] - lb[0]).powi(2)
                        + (la[1] - lb[1]).powi(2)
                        + (la[2] - lb[2]).powi(2))
                    .sqrt();
                    if de < 15.0 {
                        close.push((name, a.id, b.id, de));
                    }
                }
            }
        }
        for &(name, a, b, de) in &close {
            assert!(
                CLOSE_PAIRS.contains(&(name, a, b)),
                "{a} and {b} are DeltaE {de:.1} apart under {name}"
            );
            assert_ne!(find(a).unwrap().badge, find(b).unwrap().badge);
        }
        assert_eq!(close.len(), CLOSE_PAIRS.len(), "stale allow-list entries");
    }

    #[test]
    fn unknown_ids_resolve_deterministically() {
        let known = resolve("cursor");
        assert!(known.known);
        assert_eq!(known.name, "Cursor");
        assert_eq!(known.tint, [0xCC, 0x79, 0xA7]);
        assert_eq!(known.badge, BadgeShape::Triangle);

        let a = resolve("windsurf");
        assert!(!a.known);
        assert_eq!(a.name, "windsurf");
        assert_eq!(a, resolve("windsurf"));
        assert_ne!(a.tint, [0, 0, 0]);
        let b = resolve("tabnine");
        assert!(a.tint != b.tint || a.badge != b.badge);
    }
}
