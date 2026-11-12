// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounded synthetic acceptance coverage, not an FPS or hosted-performance claim.
//! The generated custom fixture exercises production parsers, v3 wire transfer
//! and the real rich replay boundary. No provider I/O or committed large fixture.

use std::collections::BTreeSet;
use std::fmt::Write;

use gource_core::events::{
    AttrKind, ChangeEvent, ChangeKind, EventLog, PrCompleteness, PrLogCoverage, PrLogMetadata,
    PrPayload, PrState, StrId,
};
use gource_core::pull_request::PrPhase;
use gource_core::settings::{PullRequestMode, Settings};
use gource_core::sim::Simulation;
use gource_core::view::SceneView;
use gource_sources::rich_pulls::{FrozenPrAnchors, PrAnchorInput};
use gource_sources::{BatchEncoder, CustomLogParser, EventLogSink, ParseStats, apply_batch};

const OPEN_PRS: usize = 10_000;
const REVIEW_ROUNDS: usize = 5;
const REVIEWS: usize = OPEN_PRS * REVIEW_ROUNDS;
const RECORDS: usize = OPEN_PRS + REVIEWS;
const CREDIT_EVERY: usize = 100;
const DAY: i64 = 86_400;
const CREATED_AT: i64 = 1_700_000_000;
const BOUNDARY: i64 = CREATED_AT + 30 * DAY + OPEN_PRS as i64 - 1;
const REVIEWER: &str = "public-reviewer-prolific";
const MAX_FIXTURE_BYTES: usize = 16 * 1024 * 1024;
const REVIEW_KINDS: [ChangeEvent; REVIEW_ROUNDS] = [
    ChangeEvent::ReviewApproved,
    ChangeEvent::ReviewChangesRequested,
    ChangeEvent::ReviewCommented,
    ChangeEvent::ReviewApproved,
    ChangeEvent::ReviewCommented,
];

fn coverage() -> PrLogMetadata {
    PrLogMetadata {
        version: 1,
        from: CREATED_AT,
        to: BOUNDARY + DAY,
        as_of: BOUNDARY,
        lifecycle: PrCompleteness::Complete,
        reviews: PrCompleteness::Complete,
        files: PrCompleteness::Complete,
    }
}

fn fixture() -> Vec<u8> {
    let mut text = String::with_capacity(RECORDS * 160);
    for index in 0..RECORDS {
        let number = index % OPEN_PRS + 1;
        let owner = format!("public-owner-{}", number % 32);
        let (actor, kind, event) = if index < OPEN_PRS {
            (owner.as_str(), 'A', ChangeEvent::PrOpened)
        } else {
            (REVIEWER, 'M', REVIEW_KINDS[(index - OPEN_PRS) / OPEN_PRS])
        };
        let time = CREATED_AT + index as i64;
        let created = CREATED_AT + number as i64 - 1;
        let credit = if index % CREDIT_EVERY == 0 {
            ";ai=co:claude"
        } else {
            ""
        };
        let envelope = if index == 0 {
            let metadata = coverage();
            format!(
                ";pr-log=1,{},{},{},c,c,c",
                metadata.from, metadata.to, metadata.as_of
            )
        } else {
            String::new()
        };
        // One timestamp per record keeps acceptance counts explicit. A hundred
        // code areas bounds sibling layout size without hiding any PR nodes.
        writeln!(
            text,
            "{time}|{actor}|{kind}|/area-{}/.pull-requests/#{number} Proposal界 {number}||ev={};pr={number};pr-author={owner};pr-created={created};pr-state=open;pr-initial=0{credit}{envelope}",
            number % 100,
            event.as_meta().unwrap(),
        ).unwrap();
    }
    assert!(
        text.len() <= MAX_FIXTURE_BYTES,
        "fixture must remain bounded"
    );
    text.into_bytes()
}

fn assert_parse_stats(stats: ParseStats, input: &[u8]) {
    assert_eq!(stats.bytes, input.len() as u64);
    assert_eq!(stats.lines, RECORDS as u64);
    assert_eq!(stats.commits, RECORDS as u64);
    assert_eq!(stats.files, RECORDS as u64);
    assert_eq!(stats.skipped_lines, 0);
    assert_eq!(stats.bad_meta, 0);
}

fn direct(input: &[u8]) -> EventLog {
    let mut parser = CustomLogParser::new();
    let mut log = EventLog::new();
    {
        let mut sink = EventLogSink::new(&mut log);
        parser.push(input, &mut sink);
        parser.finish(&mut sink);
    }
    assert_parse_stats(parser.stats(), input);
    log.set_total_bytes(input.len() as u64);
    log.set_complete(true);
    log
}

#[derive(Default)]
struct TransferCounts {
    batches: usize,
    commits: usize,
    changes: usize,
}

fn apply_pending(encoder: &mut BatchEncoder, log: &mut EventLog, counts: &mut TransferCounts) {
    let bytes = encoder.take_batch();
    if bytes.is_empty() {
        return;
    }
    assert_eq!(
        &bytes[..4],
        b"GEB\x03",
        "rich records must use real v3 batches"
    );
    let summary = apply_batch(log, &bytes).expect("generated rich v3 batch must apply");
    counts.batches += 1;
    counts.commits += summary.commits as usize;
    counts.changes += summary.changes as usize;
}

fn worker(input: &[u8], chunk: usize) -> (EventLog, TransferCounts) {
    let mut parser = CustomLogParser::new();
    let mut encoder = BatchEncoder::new();
    let mut log = EventLog::new();
    let mut counts = TransferCounts::default();
    for bytes in input.chunks(chunk) {
        parser.push(bytes, &mut encoder);
        apply_pending(&mut encoder, &mut log, &mut counts);
    }
    parser.finish(&mut encoder);
    apply_pending(&mut encoder, &mut log, &mut counts);
    assert_parse_stats(parser.stats(), input);
    assert!(counts.batches > 1);
    assert_eq!(counts.commits, RECORDS);
    assert_eq!(counts.changes, RECORDS);
    log.set_total_bytes(input.len() as u64);
    log.set_complete(true);
    (log, counts)
}

fn assert_import(log: &EventLog) {
    assert_eq!(log.commit_count(), RECORDS);
    assert_eq!(log.change_count(), RECORDS);
    assert_eq!(log.pr_changes().len(), RECORDS);
    assert_eq!(log.pr_coverage(), PrLogCoverage::Known(coverage()));
    assert_eq!(
        log.attr_set_count(),
        2,
        "F1 credit set survives alongside rich payloads"
    );
    let reviewer = log
        .lookup(REVIEWER)
        .expect("known prolific reviewer is retained");
    let mut reviewer_actions = 0;
    let mut credited = 0;
    let mut numbers = BTreeSet::new();
    for index in 0..RECORDS {
        let commit = log.commit(index);
        assert_eq!(commit.timestamp, CREATED_AT + index as i64);
        assert_eq!(commit.changes.len(), 1);
        let number = index % OPEN_PRS + 1;
        let change = commit.changes[0];
        let expected = if index < OPEN_PRS {
            numbers.insert(number);
            assert_eq!(change.kind, ChangeKind::Add);
            ChangeEvent::PrOpened
        } else {
            assert_eq!(change.kind, ChangeKind::Modify);
            assert_eq!(commit.user, reviewer);
            reviewer_actions += 1;
            REVIEW_KINDS[(index - OPEN_PRS) / OPEN_PRS]
        };
        assert_eq!(change.event, expected);
        let Some(PrPayload::Lifecycle(pr)) = log.pr_change(index) else {
            panic!("missing typed lifecycle at record {index}")
        };
        assert_eq!(pr.number, number as u64);
        assert_eq!(pr.created, CREATED_AT + number as i64 - 1);
        assert_eq!(log.str(pr.owner), format!("public-owner-{}", number % 32));
        assert_eq!(pr.state, PrState::Open);
        assert_eq!(pr.target, None);
        assert!(
            !pr.initial,
            "fixture consists of actual opens/reviews, not fake seeds"
        );
        let attrs = log.attr_set(commit.attrs);
        if index % CREDIT_EVERY == 0 {
            assert_eq!(attrs.len(), 1);
            assert_eq!(attrs[0].kind, AttrKind::Assist);
            assert_eq!(log.str(attrs[0].id), "claude");
            assert_eq!(attrs[0].model, None);
            credited += 1;
        } else {
            assert!(attrs.is_empty());
        }
    }
    assert_eq!(numbers.len(), OPEN_PRS);
    assert_eq!(reviewer_actions, REVIEWS);
    assert_eq!(credited, RECORDS / CREDIT_EVERY);
}

fn assert_same_log(worker: &EventLog, direct: &EventLog) {
    assert_eq!(worker.string_count(), direct.string_count());
    for index in 0..direct.string_count() {
        let id = StrId(index as u32);
        assert_eq!(worker.str(id), direct.str(id));
    }
    assert_eq!(worker.records(), direct.records());
    assert_eq!(worker.pr_changes(), direct.pr_changes());
    assert_eq!(worker.pr_coverage(), direct.pr_coverage());
    assert_eq!(worker.total_bytes(), direct.total_bytes());
    assert!(worker.is_complete());
    for index in 0..RECORDS {
        let a = worker.commit(index);
        let b = direct.commit(index);
        assert_eq!(a.changes, b.changes);
        assert_eq!(worker.attr_set(a.attrs), direct.attr_set(b.attrs));
    }
}

fn assert_seeded_boundary(log: &EventLog) {
    let mut sim = Simulation::new(Settings {
        start_timestamp: BOUNDARY,
        days_per_second: 0.0,
        auto_skip_seconds: -1.0,
        file_idle_time: 0.0,
        pull_request_mode: PullRequestMode::Full,
        pr_flow_hud: true,
        ..Settings::default()
    });
    sim.set_viewport(1280, 720, 1.0);
    for _ in 0..2 {
        sim.update(log, 1.0 / 60.0);
    }
    let status = sim.status(log);
    assert_eq!(status.current_time, BOUNDARY);
    assert_eq!(status.commits_processed, RECORDS);
    assert_eq!(
        status.files, OPEN_PRS,
        "all still-open proposals are materialised"
    );
    assert_eq!(
        status.users, 0,
        "prefix seeds must not create actor avatars"
    );
    assert!(
        status.finished,
        "there are no actions at/after this boundary"
    );
    assert_eq!(sim.pr_nodes().len(), OPEN_PRS);
    let flow = sim
        .pr_flow_stats(log)
        .expect("complete lifecycle supports repository flow");
    assert_eq!(flow.open, OPEN_PRS as u64);
    assert_eq!(flow.oldest_days, Some(30));
    for node in sim.pr_nodes() {
        assert_eq!(node.phase, PrPhase::Open);
        assert_eq!(node.generation, 1 + REVIEW_ROUNDS as u64);
        assert_eq!(node.review, Some(ChangeEvent::ReviewCommented));
        assert_eq!(log.str(node.actor), REVIEWER);
        assert_eq!(node.created, CREATED_AT + node.number as i64 - 1);
        assert_eq!(node.age_days(status.current_time), 30);
        assert_eq!(node.terminal_started, None);
    }
    let mut scene = SceneView::default();
    sim.extract_view(log, &mut scene);
    assert!(scene.users.is_empty(), "no seeded avatar draw payload");
    assert!(
        scene.actions.is_empty(),
        "no prefix action/beam draw payload"
    );
    assert!(
        !scene.files.is_empty(),
        "seeded PR geometry reaches real extraction"
    );
    // The production extractor represents a PR with three linked discs in
    // the existing file pass. Scene quads are not entity/file counts.
    assert_eq!(scene.files.len() % 3, 0);
    assert!(
        scene.files.len() <= OPEN_PRS * 3,
        "frustum culling may omit whole linked-disc groups"
    );
    for discs in scene.files.as_chunks::<3>().0 {
        assert!(discs[0].size.x > discs[1].size.x);
        assert_eq!(discs[1].size, discs[2].size);
        assert_eq!(discs[0].colour, discs[1].colour);
        assert_eq!(discs[1].colour, discs[2].colour);
        assert_eq!(discs[0].alpha, discs[1].alpha);
        assert_eq!(discs[1].alpha, discs[2].alpha);
    }
    for file in &scene.files {
        assert!(file.pos.is_finite());
        assert!(file.size.is_finite());
        assert!(file.colour.is_finite());
        assert!(file.alpha.is_finite() && (0.0..=1.0).contains(&file.alpha));
    }
    let hud = scene.hud.pr_flow.expect("flow HUD is extracted");
    assert_eq!(scene.str(hud), "PRs (repository): 10000 open, oldest 30d");
    assert_eq!(scene.hud.pr_key.len(), 3);
    assert_eq!(scene.hud.pr_coverage.len(), 3);
    for (range, expected) in scene.hud.pr_coverage.iter().zip([
        "Lifecycle: complete",
        "Reviews: complete",
        "Files: complete",
    ]) {
        assert_eq!(scene.str(*range), expected);
    }
    // The private new-proposal counter is asserted directly in native
    // sim/pr_tests boundary fixtures. This public-API stress test does not
    // scrape Debug or invent an accessor solely to repeat that assertion.
    eprintln!(
        "seed boundary: {} open PRs, oldest {}d, {} avatars, {} actions, {} linked-disc quads",
        flow.open,
        flow.oldest_days.unwrap(),
        status.users,
        scene.actions.len(),
        scene.files.len()
    );
}

#[test]
fn sixty_thousand_rich_actions_survive_v3_chunking_and_seed_a_real_start_boundary() {
    let input = fixture();
    let direct = direct(&input);
    assert_import(&direct);
    for chunk in [4093, 64 * 1024] {
        let (worker, counts) = worker(&input, chunk);
        assert_import(&worker);
        assert_same_log(&worker, &direct);
        eprintln!(
            "rich import: {} bytes, {} opens + {} reviews, {} F1 records, chunk {}, {} v3 batches",
            input.len(),
            OPEN_PRS,
            REVIEWS,
            RECORDS / CREDIT_EVERY,
            chunk,
            counts.batches
        );
        if chunk == 64 * 1024 {
            assert_seeded_boundary(&worker);
        }
    }
}

#[test]
fn allocated_anchor_survives_played_code_directory_delete_and_close_reopen() {
    let anchors = FrozenPrAnchors::new(
        &[PrAnchorInput {
            number: 7,
            title: "Immutable|title",
            changed_paths: &["/src/a.rs"],
            completeness: PrCompleteness::Complete,
        }],
        &["/src/a.rs", "/src/"],
    )
    .unwrap();
    let path = anchors.get(7).unwrap().path.as_str();
    assert_eq!(path, "/src/.pull-requests/#7 Immutable-title");
    let fields = "pr=7;pr-author=owner;pr-created=100;pr-state=open;pr-initial=0";
    let input = format!(
        "100|coder|A|/src/a.rs\n\
         100|owner|A|{path}||ev=pr-opened;{fields};pr-log=1,0,200,200,c,c,c\n\
         130|coder|D|/src/\n\
         160|closer|D|{path}||ev=pr-closed;{fields}\n\
         190|reopener|A|{path}||ev=pr-reopened;{fields}\n"
    );
    let mut parser = CustomLogParser::new();
    let mut encoder = BatchEncoder::new();
    parser.push(input.as_bytes(), &mut encoder);
    parser.finish(&mut encoder);
    assert_eq!(parser.stats().skipped_lines, 0);
    assert_eq!(parser.stats().bad_meta, 0);
    let mut log = EventLog::new();
    let mut counts = TransferCounts::default();
    apply_pending(&mut encoder, &mut log, &mut counts);
    assert_eq!((counts.commits, counts.changes), (5, 5));
    assert_eq!(log.pr_changes().len(), 3);
    log.set_total_bytes(input.len() as u64);
    log.set_complete(true);
    let pr_path = log.lookup(path).unwrap();
    for record in log.pr_changes() {
        let file = log.commit(record.change_index as usize).changes[0];
        assert_eq!(
            file.path, pr_path,
            "all lifecycle rows use the allocator's frozen path"
        );
    }
    let mut sim = Simulation::new(Settings {
        days_per_second: 1.0 / DAY as f32,
        auto_skip_seconds: -1.0,
        file_idle_time: 0.0,
        pull_request_mode: PullRequestMode::Full,
        pr_flow_hud: true,
        ..Settings::default()
    });
    sim.set_viewport(1280, 720, 1.0);
    let mut scene = SceneView::default();
    let mut code_present = false;
    for _ in 0..6000 {
        sim.update(&log, 1.0 / 60.0);
        if sim.status(&log).commits_processed == 2 {
            sim.extract_view(&log, &mut scene);
            if scene.files.len() == 4 {
                code_present = true; // one real code disc plus the three PR discs
                break;
            }
        }
    }
    assert!(
        code_present,
        "genuine code add must be visible before directory delete"
    );
    let mut directory_deleted = false;
    for _ in 0..6000 {
        sim.update(&log, 1.0 / 60.0);
        let status = sim.status(&log);
        if status.commits_processed == 3 && status.files == 1 {
            directory_deleted = true;
            break;
        }
    }
    assert!(
        directory_deleted,
        "actual /src/ code delete must remove the ordinary file"
    );
    let node = sim.pr_node_by_path(pr_path).unwrap();
    assert_eq!(node.phase, PrPhase::Open);
    assert_eq!(
        node.generation, 1,
        "ordinary code delete does not alter PR lifecycle"
    );
    assert_eq!(sim.pr_flow_stats(&log).unwrap().open, 1);
    let mut saw_close = false;
    let mut reopened = false;
    for _ in 0..6000 {
        sim.update(&log, 1.0 / 60.0);
        let node = sim.pr_node_by_path(pr_path).unwrap();
        saw_close |= node.phase == PrPhase::Closed;
        if sim.status(&log).commits_processed == 5
            && node.phase == PrPhase::Open
            && node.generation == 3
        {
            reopened = true;
            break;
        }
    }
    assert!(
        saw_close,
        "close is genuinely consumed, not only prefix-folded"
    );
    assert!(
        reopened,
        "reopen is genuinely consumed at the same frozen path"
    );
    let node = sim.pr_node_by_path(pr_path).unwrap();
    assert_eq!(node.path, pr_path);
    assert_eq!(node.number, 7);
    assert_eq!(node.created, 100);
    assert_eq!(log.str(node.owner), "owner");
    assert_eq!(log.str(node.actor), "reopener");
    assert_eq!(log.str(node.path), anchors.get(7).unwrap().path);
    assert_eq!(sim.pr_flow_stats(&log).unwrap().open, 1);
    assert_eq!(sim.status(&log).files, 1);
    sim.extract_view(&log, &mut scene);
    assert_eq!(
        scene.files.len(),
        3,
        "only the reopened typed marker remains"
    );
}
