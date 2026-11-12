// SPDX-License-Identifier: GPL-3.0-or-later

//! AI attribution in the simulation: assist state on screen, decorations,
//! played counts and their seek seeding, the agent key, the hover line,
//! `ai-filter` and `attr_stats`.

use glam::Vec2;

use super::super::*;
use crate::events::{AttrKind, Attribution, ChangeEvent, ChangeKind, EventLog, FileChange};
use crate::input::InputEvent;
use crate::view::{DecoKind, SceneView};

const A: ChangeKind = ChangeKind::Add;
const M: ChangeKind = ChangeKind::Modify;

type Credit<'a> = (AttrKind, &'a str, Option<&'a str>);
type Spec<'a> = (i64, &'a str, &'a [(&'a str, ChangeKind)], &'a [Credit<'a>]);

/// One commit per entry: (timestamp, user, changes, credits).
fn ai_log(commits: &[Spec]) -> EventLog {
    let mut log = EventLog::new();
    for (i, &(ts, user, changes, credits)) in commits.iter().enumerate() {
        let u = log.intern(user);
        let set: Vec<Attribution> = credits
            .iter()
            .map(|&(kind, id, model)| Attribution {
                kind,
                id: log.intern(id),
                model: model.map(|m| log.intern(m)),
            })
            .collect();
        let set = log.intern_attr_set(&set);
        let cs: Vec<FileChange> = changes
            .iter()
            .map(|&(p, kind)| FileChange {
                path: log.intern(p),
                kind,
                colour: None,
                event: ChangeEvent::Commit,
            })
            .collect();
        log.push_commit_attr(ts, u, cs, i as u64 * 10, set);
    }
    log.set_total_bytes(commits.len() as u64 * 10);
    log.set_complete(true);
    log
}

const OPUS: Credit = (AttrKind::Assist, "claude", Some("Claude Opus 5.5"));
const CODEX: Credit = (AttrKind::Assist, "codex", None);
const CURSOR: Credit = (AttrKind::Agent, "cursor", None);

/// jane: plain, then assisted by Claude; bob: plain; the cursor agent
/// authors; then a long quiet stretch of plain commits by bob.
fn scenario() -> EventLog {
    let paths: Vec<String> = (0..40).map(|i| format!("/docs/{i}.md")).collect();
    let docs: Vec<[(&str, ChangeKind); 1]> = paths.iter().map(|p| [(p.as_str(), A)]).collect();
    let mut specs: Vec<Spec> = vec![
        (1000, "jane", &[("/src/a.rs", A)], &[]),
        (1000, "bob", &[("/README.md", A)], &[]),
        (1100, "jane", &[("/src/b.rs", A), ("/src/c.rs", A)], &[OPUS]),
        (1200, "agent", &[("/web/x.ts", A)], &[CURSOR]),
        (1300, "jane", &[("/src/a.rs", M)], &[OPUS, CODEX]),
    ];
    // one plain commit about every seven frames
    for (i, d) in docs.iter().enumerate() {
        specs.push((2000 + i as i64 * 1000, "bob", d, &[]));
    }
    ai_log(&specs)
}

fn sim_with(options: &[(&str, &str)]) -> Simulation {
    let mut settings = Settings::default();
    for (name, value) in options {
        settings.apply_option(name, value).unwrap();
    }
    let mut sim = Simulation::new(settings);
    sim.set_viewport(1280, 720, 1.0);
    sim
}

fn user_view<'a>(
    sim: &Simulation,
    view: &'a SceneView,
    name: &str,
) -> Option<&'a crate::view::UserView> {
    let index = sim.user_map.keys().position(|n| n == name)?;
    view.users.get(index)
}

fn frames(sim: &mut Simulation, log: &EventLog, view: &mut SceneView, n: usize) {
    for _ in 0..n {
        sim.update(log, 1.0 / 60.0);
        sim.extract_view(log, view);
    }
}

#[test]
fn assisted_user_wears_gear_while_credited_work_plays_then_loses_it() {
    let log = scenario();
    let mut sim = sim_with(&[]);
    let mut view = SceneView::default();

    let mut seen_jane = 0u8;
    let mut seen_agent = 0u8;
    let mut max_env = 0.0f32;
    let mut bob_ever = 0u8;
    for _ in 0..1200 {
        frames(&mut sim, &log, &mut view, 1);
        if let Some(u) = user_view(&sim, &view, "jane") {
            seen_jane |= u.decor.mask;
            max_env = max_env.max(u.decor.envelope);
            if u.decor.mask != 0 {
                assert_eq!(u.decor.scale, 1.0);
                assert_eq!(u.decor.motion, 1.0);
                assert_eq!(u.decor.dynamic, u.decor.mask);
            }
        }
        if let Some(u) = user_view(&sim, &view, "agent") {
            seen_agent |= u.decor.mask;
        }
        if let Some(u) = user_view(&sim, &view, "bob") {
            bob_ever |= u.decor.mask;
        }
    }
    let assisted = DecoKind::Cape.bit() | DecoKind::Shades.bit() | DecoKind::Badge.bit();
    let agent = DecoKind::Halo.bit() | DecoKind::Glow.bit() | DecoKind::Badge.bit();
    assert_eq!(seen_jane, assisted);
    assert_eq!(seen_agent, agent);
    assert_eq!(bob_ever, 0, "plain contributors look as before");
    assert_eq!(max_env, 1.0);
    // long after: jane is plain again (or gone)
    if let Some(&u) = sim.user_map.get("jane") {
        assert_eq!((sim.users[u].attr_set, sim.users[u].attr_env), (0, 0.0));
    }
}

#[test]
fn hidden_decorations_and_an_empty_style_draw_nothing() {
    let log = scenario();
    for options in [
        &[("hide", "decorations")][..],
        &[("hide-decorations", "true")][..],
        &[("ai-style", "none")][..],
    ] {
        let mut sim = sim_with(options);
        let mut view = SceneView::default();
        for _ in 0..600 {
            frames(&mut sim, &log, &mut view, 1);
            assert!(
                view.users.iter().all(|u| u.decor == Default::default()),
                "{options:?}"
            );
        }
    }
}

#[test]
fn rules_change_live_and_motion_can_be_frozen() {
    let log = scenario();
    let mut sim = sim_with(&[("decor-motion", "off")]);
    let mut view = SceneView::default();
    let caped = |sim: &Simulation, view: &SceneView| {
        user_view(sim, view, "jane").is_some_and(|u| u.decor.has(DecoKind::Cape))
    };
    for _ in 0..600 {
        if caped(&sim, &view) {
            break;
        }
        frames(&mut sim, &log, &mut view, 1);
    }
    assert!(caped(&sim, &view));
    let jane = user_view(&sim, &view, "jane").unwrap().decor;
    assert_eq!(jane.motion, 0.0);

    sim.set_option("user-style", "assisted => cape=off; hat=FF0000")
        .unwrap();
    sim.extract_view(&log, &mut view);
    let jane = user_view(&sim, &view, "jane").unwrap().decor;
    assert_eq!(jane.mask & DecoKind::Cape.bit(), 0);
    assert_eq!(jane.tint[DecoKind::Hat.index()], [255, 0, 0]);

    // a static rule for plain contributors draws at once
    sim.set_option("user-style", "human & !ai => halo=on")
        .unwrap();
    sim.extract_view(&log, &mut view);
    let bob = user_view(&sim, &view, "bob").unwrap();
    assert_eq!(bob.decor.mask, DecoKind::Halo.bit());
    assert_eq!(bob.decor.dynamic, 0);
    let c = bob.colour;
    let b = |x: f32| (x * 255.0 + 0.5) as u8;
    assert_eq!(
        bob.decor.tint[DecoKind::Halo.index()],
        [b(c.x), b(c.y), b(c.z)]
    );
}

#[test]
fn two_runs_are_identical_and_restart_clears_the_state() {
    let log = scenario();
    let run = |sim: &mut Simulation| -> Vec<SceneView> {
        let mut out = Vec::new();
        let mut view = SceneView::default();
        for _ in 0..240 {
            frames(sim, &log, &mut view, 1);
            out.push(view.clone());
        }
        out
    };
    let mut a = sim_with(&[("agent-key", "true")]);
    let mut b = sim_with(&[("agent-key", "true")]);
    let first = run(&mut a);
    assert_eq!(first, run(&mut b));
    assert!(
        first
            .iter()
            .any(|v| v.users.iter().any(|u| u.decor.mask != 0))
    );
    a.restart(&log);
    assert!(a.played_set_counts().is_empty());
    assert_eq!(run(&mut a), first);
}

/// Played counts with trailing zeros removed.
fn trimmed(counts: &[u32]) -> Vec<u32> {
    let mut v = counts.to_vec();
    while v.last() == Some(&0) {
        v.pop();
    }
    v
}

#[test]
fn seek_seeds_the_played_counts_like_a_straight_playthrough() {
    let log = scenario();
    let mut straight = sim_with(&[("agent-key", "true")]);
    let mut view = SceneView::default();
    // (next commit index, counts, agent rows) after every frame
    let mut seen = Vec::new();
    for _ in 0..3000 {
        frames(&mut straight, &log, &mut view, 1);
        let next = straight.status(&log).commits_processed;
        seen.push((
            next,
            trimmed(straight.played_set_counts()),
            view.hud.agents.len(),
        ));
    }
    for fraction in [0.1, 0.25, 0.5, 0.75] {
        let mut sim = sim_with(&[("agent-key", "true")]);
        frames(&mut sim, &log, &mut view, 5);
        sim.seek(&log, fraction);
        let cursor = sim.cursor;
        let counts = trimmed(sim.played_set_counts());
        let (_, expected, rows) = seen
            .iter()
            .find(|(next, _, _)| *next == cursor)
            .unwrap_or_else(|| panic!("no frame at cursor {cursor}"));
        assert_eq!(&counts, expected, "seek to {fraction}");
        sim.extract_view(&log, &mut view);
        assert_eq!(view.hud.agents.len(), *rows);
    }
    // after a seek past the credited commits the key lists both agents of
    // the assist sets and the author
    let mut sim = sim_with(&[("agent-key", "true")]);
    frames(&mut sim, &log, &mut view, 2);
    sim.seek(&log, 0.5);
    sim.extract_view(&log, &mut view);
    let rows: Vec<(&str, u32)> = view
        .hud
        .agents
        .iter()
        .map(|r| (view.str(r.name), r.count))
        .collect();
    assert_eq!(rows, [("Claude", 2), ("Codex", 1), ("Cursor", 1)]);
    assert!(view.hud.show_agent_key);
    assert_eq!(view.hud.agents_more, 0);
}

#[test]
fn agent_key_lists_eight_rows_then_n_more() {
    let ids: Vec<String> = (0..11).map(|i| format!("tool{i:02}")).collect();
    let mut specs: Vec<Spec> = Vec::new();
    let credits: Vec<[Credit; 1]> = ids
        .iter()
        .map(|id| [(AttrKind::Assist, id.as_str(), None)])
        .collect();
    let paths: Vec<String> = (0..30).map(|i| format!("/f{i}.c")).collect();
    let changes: Vec<[(&str, ChangeKind); 1]> = paths.iter().map(|p| [(p.as_str(), A)]).collect();
    for (i, change) in changes.iter().enumerate() {
        // tool00 gets 3, tool01 2 more than the rest, ...
        let tool = if i < 3 {
            0
        } else if i < 5 {
            1
        } else {
            i % 11
        };
        specs.push((1000 + i as i64, "u", change, &credits[tool]));
    }
    let log = ai_log(&specs);
    let mut sim = sim_with(&[]);
    let mut view = SceneView::default();
    frames(&mut sim, &log, &mut view, 600);
    assert!(
        view.hud.agents.is_empty() && !view.hud.show_agent_key,
        "off by default"
    );
    sim.set_option("agent-key", "true").unwrap();
    sim.extract_view(&log, &mut view);
    assert_eq!(view.hud.agents.len(), crate::view::AGENT_KEY_ROWS);
    assert_eq!(view.hud.agents_more, 3);
    let counts: Vec<u32> = view.hud.agents.iter().map(|r| r.count).collect();
    assert!(counts.windows(2).all(|w| w[0] >= w[1]), "{counts:?}");
    assert_eq!(view.str(view.hud.agents[0].name), "tool00");
    // unknown ids show under their id with a hashed tint
    let tint = crate::agents::resolve("tool00").tint;
    let c = view.hud.agents[0].colour * 255.0 + 0.5;
    assert_eq!([c.x as u8, c.y as u8, c.z as u8], tint);
}

#[test]
fn hover_shows_the_current_state() {
    let log = scenario();
    let mut sim = sim_with(&[]);
    let mut view = SceneView::default();
    let mut lines_seen = Vec::new();
    for _ in 0..900 {
        frames(&mut sim, &log, &mut view, 1);
        for name in ["jane", "agent", "bob"] {
            let Some(&u) = sim.user_map.get(name) else {
                continue;
            };
            let user = &sim.users[u];
            if user.is_fading(&sim.settings) {
                continue;
            }
            // point at the user, as mousetrace inverts it
            let cam = sim.camera.pos;
            let (w, h) = (sim.viewport.x, sim.viewport.y);
            let q = (user.pos() - cam.truncate()) / cam.z;
            let p = Vec2::new((w - q.x * h) / 2.0, (1.0 - q.y) * h / 2.0);
            sim.handle_input(InputEvent::PointerMove {
                x: p.x,
                y: p.y,
                buttons: 0,
            });
            sim.extract_view(&log, &mut view);
            if let Some(tb) = &view.hud.textbox {
                let lines: Vec<String> =
                    tb.lines.iter().map(|&l| view.str(l).to_string()).collect();
                if lines[0] == name && !lines_seen.contains(&lines) {
                    lines_seen.push(lines);
                }
            }
        }
    }
    let has = |lines: &[&str]| lines_seen.iter().any(|l| l == lines);
    assert!(
        has(&["jane", "AI-assisted: Claude Opus 5.5"]),
        "{lines_seen:?}"
    );
    assert!(
        has(&["jane", "AI-assisted: Claude Opus 5.5 and others"]),
        "{lines_seen:?}"
    );
    assert!(has(&["agent", "AI agent: Cursor"]), "{lines_seen:?}");
    assert!(has(&["bob"]), "{lines_seen:?}");
    assert!(!lines_seen.iter().any(|l| l[0] == "bob" && l.len() > 1));
}

#[test]
fn ai_filter_restricts_the_played_commits_and_shows_a_line() {
    let log = scenario();
    let users_of = |filter: &str| {
        let mut sim = sim_with(&[("ai-filter", filter)]);
        let mut view = SceneView::default();
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..1200 {
            frames(&mut sim, &log, &mut view, 1);
            seen.extend(sim.user_map.keys().cloned());
        }
        let note = view.hud.filter_note.map(|r| view.str(r).to_string());
        let played: u32 = sim.played_set_counts().iter().sum();
        (seen.into_iter().collect::<Vec<_>>(), note, played)
    };
    let (all, note, played) = users_of("all");
    assert_eq!(all, ["agent", "bob", "jane"]);
    assert_eq!((note, played), (None, 45));
    let (only, note, played) = users_of("only-assisted");
    assert_eq!(only, ["agent", "jane"]);
    assert_eq!(note.as_deref(), Some("AI commits: only assisted"));
    assert_eq!(played, 3);
    let (excl, note, played) = users_of("exclude-assisted");
    assert_eq!(excl, ["bob", "jane"]);
    assert_eq!(note.as_deref(), Some("AI commits: excluding assisted"));
    assert_eq!(played, 42);
}

#[test]
fn attr_stats_count_loaded_and_played_commits() {
    let log = scenario();
    let mut sim = sim_with(&[]);
    let stats = sim.attr_stats(&log);
    assert_eq!(stats.registry, crate::agents::REGISTRY_VERSION);
    assert_eq!(stats.sets, 3);
    let l = &stats.loaded;
    assert_eq!((l.commits, l.assisted, l.agent, l.bot), (45, 2, 1, 0));
    let ids: Vec<(&str, u32, u32)> = l
        .agents
        .iter()
        .map(|a| (a.id.as_str(), a.commits, a.as_author))
        .collect();
    assert_eq!(ids, [("claude", 2, 0), ("codex", 1, 0), ("cursor", 1, 1)]);
    assert_eq!(l.agents[0].name, "Claude");
    assert_eq!(l.agents[0].colour, [0xE6, 0x9F, 0x00]);
    assert_eq!(l.agents[0].models.len(), 1);
    assert_eq!(l.agents[0].models[0].model, "Claude Opus 5.5");
    assert_eq!(l.agents[0].models[0].commits, 2);
    assert_eq!(stats.played, AttrTotals::default());

    let mut view = SceneView::default();
    frames(&mut sim, &log, &mut view, 1);
    let p = sim.attr_stats(&log).played;
    assert!(p.commits >= 1 && p.commits < 45 && p.agent == 0, "{p:?}");
    frames(&mut sim, &log, &mut view, 600);
    assert_eq!(sim.attr_stats(&log).played, stats.loaded);
}

#[test]
fn attr_stats_are_bounded_for_hostile_logs() {
    // 3000 distinct models of one agent and 40 agents
    let models: Vec<String> = (0..3000).map(|i| format!("m{i}")).collect();
    let agents: Vec<String> = (0..40).map(|i| format!("a{i}")).collect();
    let mut credits: Vec<[Credit; 1]> = models
        .iter()
        .map(|m| [(AttrKind::Assist, "claude", Some(m.as_str()))])
        .collect();
    credits.extend(agents.iter().map(|a| [(AttrKind::Agent, a.as_str(), None)]));
    let changes = [("/x", A)];
    let specs: Vec<Spec> = credits
        .iter()
        .enumerate()
        .map(|(i, c)| (1000 + i as i64, "u", &changes[..], &c[..]))
        .collect();
    let log = ai_log(&specs);
    let stats = Simulation::new(Settings::default()).attr_stats(&log);
    let l = &stats.loaded;
    assert_eq!(l.agents.len(), STATS_MAX_AGENTS);
    assert_eq!(l.more_agents, 41 - STATS_MAX_AGENTS as u32);
    let claude = &l.agents[0];
    assert_eq!((claude.id.as_str(), claude.commits), ("claude", 3000));
    assert_eq!(claude.models.len(), STATS_MAX_MODELS);
    assert_eq!(claude.other_models, 3000 - STATS_MAX_MODELS as u32);
}
