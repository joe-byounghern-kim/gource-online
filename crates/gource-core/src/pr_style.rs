// SPDX-License-Identifier: GPL-3.0-or-later
//! PR presentation rules. Styles never mutate lifecycle truth or committed files.

use glam::Vec3;

use crate::events::ChangeEvent;
use crate::pull_request::{PR_TERMINAL_SECONDS, PrNode, PrPhase};
use crate::settings::PullRequestStyle;

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PrVisual {
    pub glyph: &'static str,
    pub label: &'static str,
    pub colour: Vec3,
    pub alpha: f32,
}

/// Alternative styles consume the same immutable lifecycle snapshot.
pub trait PrStyle {
    fn appearance(&self, node: &PrNode, animation_runtime: f32) -> PrVisual;
}

#[derive(Copy, Clone, Debug, Default)]
pub struct DimHighlight;

impl PrStyle for DimHighlight {
    fn appearance(&self, node: &PrNode, animation_runtime: f32) -> PrVisual {
        let (glyph, label, colour, mut alpha) = match node.phase {
            PrPhase::Open => ("P", "Open proposal", Vec3::new(0.55, 0.7, 1.0), 0.5),
            PrPhase::Draft => ("D", "Draft proposal", Vec3::new(0.65, 0.65, 0.7), 0.35),
            PrPhase::Merged => ("M", "Merged proposal", Vec3::new(0.75, 0.45, 1.0), 1.0),
            PrPhase::Closed => ("X", "Closed proposal", Vec3::new(1.0, 0.45, 0.4), 0.8),
        };
        let mut visual = PrVisual {
            glyph,
            label,
            colour,
            alpha,
        };
        if matches!(node.phase, PrPhase::Merged | PrPhase::Closed) {
            alpha *= node
                .terminal_age(animation_runtime)
                .map_or(0.0, |age| (1.0 - age / PR_TERMINAL_SECONDS).clamp(0.0, 1.0));
            visual.alpha = alpha;
            return visual;
        }
        if let Some(event) = node.review {
            let (glyph, colour) = match event {
                ChangeEvent::ReviewApproved => ("A", Vec3::new(0.4, 1.0, 0.55)),
                ChangeEvent::ReviewChangesRequested => ("!", Vec3::new(1.0, 0.65, 0.3)),
                ChangeEvent::ReviewCommented => ("C", Vec3::new(0.45, 0.8, 1.0)),
                _ => return visual,
            };
            visual.glyph = glyph;
            visual.colour = colour;
            visual.label = event.label().unwrap_or(label);
        }
        visual
    }
}

pub fn appearance(style: PullRequestStyle, node: &PrNode, runtime: f32) -> PrVisual {
    match style {
        PullRequestStyle::DimHighlight => DimHighlight.appearance(node, runtime),
    }
}

/// A one-animation-second cosmetic flash of an existing exact-associated file.
/// Negative/nonfinite ages never produce a highlight.
pub fn merge_flash_strength(age: f32) -> f32 {
    if !age.is_finite() || age < 0.0 {
        0.0
    } else {
        (1.0 - age).clamp(0.0, 1.0)
    }
}
