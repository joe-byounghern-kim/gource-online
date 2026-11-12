// SPDX-License-Identifier: GPL-3.0-or-later

use gource_core::events::{ChangeEvent, EventLog};
use gource_core::pr_style::{DimHighlight, PrStyle};
use gource_core::pull_request::{PrNode, PrPhase};

fn proposal(phase: PrPhase) -> PrNode {
    let mut log = EventLog::new();
    PrNode {
        path: log.intern("/src/.pull-requests/#7 Example"),
        number: 7,
        owner: log.intern("Owner"),
        created: 1,
        phase,
        review: None,
        actor: log.intern("Actor"),
        target: None,
        generation: 1,
        terminal_started: None,
    }
}

#[test]
fn dim_highlight_distinguishes_proposals_verdicts_and_terminal_states() {
    let style = DimHighlight;
    let mut node = proposal(PrPhase::Open);
    let open = style.appearance(&node, 0.0);
    assert_eq!(open.glyph, "P");
    assert!(open.alpha > 0.0 && open.alpha < 1.0);
    for (event, glyph) in [
        (ChangeEvent::ReviewApproved, "A"),
        (ChangeEvent::ReviewChangesRequested, "!"),
        (ChangeEvent::ReviewCommented, "C"),
    ] {
        node.review = Some(event);
        let review = style.appearance(&node, 0.0);
        assert_eq!(review.glyph, glyph);
        assert!(!review.label.is_empty());
    }
    node.phase = PrPhase::Draft;
    node.review = None;
    assert_eq!(style.appearance(&node, 0.0).glyph, "D");
    node.phase = PrPhase::Merged;
    node.terminal_started = Some(5.0);
    let merged = style.appearance(&node, 5.0);
    assert_eq!(merged.glyph, "M");
    assert_eq!(merged.alpha, 1.0);
    assert_ne!(merged.colour, open.colour);
    assert_eq!(style.appearance(&node, 7.0).alpha, 0.0);
    node.phase = PrPhase::Closed;
    assert_eq!(style.appearance(&node, 5.0).glyph, "X");
}

#[test]
fn alternate_style_substitutes_through_the_same_immutable_trait_contract() {
    struct Outline;
    impl PrStyle for Outline {
        fn appearance(&self, node: &PrNode, runtime: f32) -> gource_core::pr_style::PrVisual {
            let mut visual = DimHighlight.appearance(node, runtime);
            visual.glyph = "O";
            visual.alpha = 0.75;
            visual
        }
    }
    let node = proposal(PrPhase::Open);
    let before = node;
    let styles: [&dyn PrStyle; 2] = [&DimHighlight, &Outline];
    let ordinary = styles[0].appearance(&node, 5.0);
    let alternative = styles[1].appearance(&node, 5.0);
    assert_ne!(alternative, ordinary);
    assert_eq!(alternative.glyph, "O");
    assert_eq!(
        node, before,
        "presentation cannot take ownership or mutate lifecycle"
    );
}

#[test]
fn visual_style_never_changes_lifecycle_and_file_flash_is_bounded() {
    let node = proposal(PrPhase::Open);
    let before = node;
    let _ = DimHighlight.appearance(&node, 1000.0);
    assert_eq!(node, before);
    assert_eq!(gource_core::pr_style::merge_flash_strength(0.0), 1.0);
    assert_eq!(gource_core::pr_style::merge_flash_strength(1.0), 0.0);
    assert_eq!(gource_core::pr_style::merge_flash_strength(-1.0), 0.0);
}
