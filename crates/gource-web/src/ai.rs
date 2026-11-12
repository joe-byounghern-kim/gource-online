// SPDX-License-Identifier: GPL-3.0-or-later

//! The AI attribution surface of `GourceApp` (AI-46, AI-63): `ai_stats()`
//! and `check_style()` as hand-written JSON, and the `<code>: <text>`
//! message of option errors.

use std::fmt::Write;

use gource_core::settings::OptionError;
use gource_core::sim::{AgentStats, AttrStats, AttrTotals};
use gource_core::style::{self, codes};

use crate::status::push_json_string;

/// `ai_stats()` JSON:
///
/// ```json
/// {"registry":1,
///  "loaded":{"commits":1480,"assisted":412,"agent":57,"bot":30,"sets":41,
///            "agents":[{"id":"claude","name":"Claude","colour":"E69F00",
///                       "commits":400,"as_author":0,
///                       "models":[{"model":"Claude Opus 5.5","commits":380}],
///                       "other_models":0}],
///            "more_agents":0},
///  "played":{"commits":640,"assisted":190,"agent":20,"bot":11,"agents":[],
///            "more_agents":0}}
/// ```
///
/// `loaded` covers the whole log, `played` the commits played up to the
/// playback position. At most 16 agents (`more_agents` counts the rest)
/// and 8 models per agent (`other_models` counts the commits of the rest),
/// ordered by commits descending, then id or model text.
pub fn ai_stats_json(stats: &AttrStats) -> String {
    let mut out = String::with_capacity(256);
    let _ = write!(out, "{{\"registry\":{},\"loaded\":", stats.registry);
    push_totals(&mut out, &stats.loaded, Some(stats.sets));
    out.push_str(",\"played\":");
    push_totals(&mut out, &stats.played, None);
    out.push('}');
    out
}

fn push_totals(out: &mut String, t: &AttrTotals, sets: Option<u32>) {
    let _ = write!(
        out,
        "{{\"commits\":{},\"assisted\":{},\"agent\":{},\"bot\":{}",
        t.commits, t.assisted, t.agent, t.bot
    );
    if let Some(sets) = sets {
        let _ = write!(out, ",\"sets\":{sets}");
    }
    out.push_str(",\"agents\":[");
    for (i, a) in t.agents.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        push_agent(out, a);
    }
    let _ = write!(out, "],\"more_agents\":{}}}", t.more_agents);
}

fn push_agent(out: &mut String, a: &AgentStats) {
    out.push_str("{\"id\":");
    push_json_string(out, &a.id);
    out.push_str(",\"name\":");
    push_json_string(out, &a.name);
    let [r, g, b] = a.colour;
    let _ = write!(
        out,
        ",\"colour\":\"{r:02X}{g:02X}{b:02X}\",\"commits\":{},\"as_author\":{},\"models\":[",
        a.commits, a.as_author
    );
    for (i, m) in a.models.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"model\":");
        push_json_string(out, &m.model);
        let _ = write!(out, ",\"commits\":{}}}", m.commits);
    }
    let _ = write!(out, "],\"other_models\":{}}}", a.other_models);
}

/// `check_style(text)` JSON: `{"ok":true}`, or
/// `{"ok":false,"code":"style_bad_property","line":2,"message":"..."}` with
/// the same code and line `set_option("user-style", text)` rejects.
pub fn check_style_json(text: &str) -> String {
    match style::parse_rules(text) {
        Ok(_) => "{\"ok\":true}".to_string(),
        Err(e) => {
            let mut out = String::from("{\"ok\":false,\"code\":");
            push_json_string(&mut out, e.code);
            let _ = write!(out, ",\"line\":{},\"message\":", e.line);
            push_json_string(&mut out, &e.message);
            out.push('}');
            out
        }
    }
}

/// The message of the `JsError` an option error throws. Errors that carry
/// a code (`user-style`: `"<code>: <message> (line N)"`) are passed as
/// `<code>: <text>`, the convention `parseWasmError` reads; others keep
/// their plain description.
pub fn option_error_message(e: &OptionError) -> String {
    match e {
        OptionError::InvalidValue { reason, .. } if has_code(reason) => reason.clone(),
        other => other.to_string(),
    }
}

fn has_code(reason: &str) -> bool {
    let known = [
        codes::MISSING_ARROW,
        codes::BAD_SELECTOR,
        codes::BAD_PROPERTY,
        codes::BAD_VALUE,
        codes::TOO_MANY_TERMS,
        codes::TOO_MANY_RULES,
        codes::TOO_MANY_LINES,
        codes::LINE_TOO_LONG,
        codes::RESERVED,
    ];
    reason
        .split_once(": ")
        .is_some_and(|(code, _)| known.contains(&code))
}

#[cfg(test)]
mod tests {
    use gource_core::settings::Settings;
    use gource_core::sim::ModelStats;

    use super::*;

    #[test]
    fn stats_json_has_the_documented_shape() {
        let agent = AgentStats {
            id: "claude".into(),
            name: "Claude \"x\"".into(),
            colour: [0xE6, 0x9F, 0x00],
            commits: 3,
            as_author: 1,
            models: vec![ModelStats {
                model: "Claude Opus 5.5".into(),
                commits: 2,
            }],
            other_models: 1,
        };
        let stats = AttrStats {
            registry: 1,
            loaded: AttrTotals {
                commits: 10,
                assisted: 2,
                agent: 1,
                bot: 0,
                agents: vec![agent],
                more_agents: 0,
            },
            sets: 4,
            played: AttrTotals::default(),
        };
        assert_eq!(
            ai_stats_json(&stats),
            "{\"registry\":1,\"loaded\":{\"commits\":10,\"assisted\":2,\"agent\":1,\"bot\":0,\
             \"sets\":4,\"agents\":[{\"id\":\"claude\",\"name\":\"Claude \\\"x\\\"\",\
             \"colour\":\"E69F00\",\"commits\":3,\"as_author\":1,\
             \"models\":[{\"model\":\"Claude Opus 5.5\",\"commits\":2}],\"other_models\":1}],\
             \"more_agents\":0},\"played\":{\"commits\":0,\"assisted\":0,\"agent\":0,\
             \"bot\":0,\"agents\":[],\"more_agents\":0}}"
        );
    }

    #[test]
    fn check_style_matches_set_option() {
        assert_eq!(check_style_json("ai => badge\n# x"), "{\"ok\":true}");
        let text = "ai => badge\nai => capes=on";
        assert_eq!(
            check_style_json(text),
            "{\"ok\":false,\"code\":\"style_bad_property\",\"line\":2,\"message\":\
             \"Unknown property \\\"capes\\\". Known: cape, shades, hat, halo, glow, badge, none.\"}"
        );
        let err = Settings::default()
            .apply_option("user-style", text)
            .unwrap_err();
        let msg = option_error_message(&err);
        assert!(
            msg.starts_with("style_bad_property: Unknown property"),
            "{msg}"
        );
        assert!(msg.ends_with("(line 2)"), "{msg}");

        // other options keep their description
        let err = Settings::default()
            .apply_option("ai-filter", "x")
            .unwrap_err();
        assert!(option_error_message(&err).starts_with("invalid value 'x' for 'ai-filter'"));
    }
}

/// The real engine natively over the web sample `ai-coauthors.log`.
#[cfg(test)]
mod sample_tests {
    use std::cell::Cell;

    use gource_core::settings::Settings;
    use gource_render::{
        Backend, BackendStats, Frame, GlyphSource, MonospaceGlyphs, Stream, TextureId,
    };

    use crate::engine::{Clock, Engine};
    use crate::parser::ChunkParser;

    #[derive(Default)]
    struct CountingBackend {
        glyphs: MonospaceGlyphs,
        /// Most commands of one frame, and whether any frame drew decorations.
        max_commands: usize,
        deco_frames: usize,
        twelve: usize,
    }

    impl Backend for CountingBackend {
        type Error = String;

        fn resize(&mut self, _: u32, _: u32) {}

        fn glyphs(&mut self) -> &mut dyn GlyphSource {
            &mut self.glyphs
        }

        fn draw_frame(&mut self, frame: &Frame) -> Result<BackendStats, String> {
            let n = frame.commands.len();
            self.max_commands = self.max_commands.max(n);
            let deco = frame
                .commands
                .iter()
                .filter(|c| c.texture == TextureId::Deco && c.stream == Stream::Quads)
                .count();
            if deco > 0 {
                self.deco_frames += 1;
            }
            if n == 12 && deco == 2 {
                self.twelve += 1;
            }
            Ok(BackendStats {
                draw_calls: n as u32,
                ..BackendStats::default()
            })
        }
    }

    struct NoClock(Cell<f64>);

    impl Clock for NoClock {
        fn now_ms(&self) -> f64 {
            self.0.get()
        }
    }

    const SAMPLE: &str = include_str!("../../../apps/player/tests/samples/ai-coauthors.log");

    fn engine(options: &[(&str, &str)]) -> Engine<CountingBackend, NoClock> {
        let mut e = Engine::new(
            CountingBackend::default(),
            NoClock(Cell::new(0.0)),
            Settings::default(),
        );
        e.begin_log();
        let mut parser = ChunkParser::new(0);
        for chunk in SAMPLE.as_bytes().chunks(4096) {
            e.append_batch(&parser.push(chunk)).unwrap();
        }
        e.append_batch(&parser.finish()).unwrap();
        assert_eq!(parser.stats().bad_meta, 0);
        e.finish_log(SAMPLE.len() as f64);
        for (name, value) in options {
            e.set_option(name, value).unwrap();
        }
        e.restart();
        e.resize(1280.0, 720.0, 1.0).unwrap();
        e.set_fixed_dt(1.0 / 60.0);
        e
    }

    #[test]
    fn sample_draws_decorations_in_at_most_twelve_commands() {
        let mut e = engine(&[]);
        for _ in 0..3600 {
            e.step(1).unwrap();
        }
        let b = e.backend();
        assert!(b.deco_frames > 60, "{}", b.deco_frames);
        assert!(b.twelve > 0, "no frame drew both decoration blocks");
        assert_eq!(b.max_commands, 12);
        assert!(e.frame_data().bloom.len() >= 4);

        let mut hidden = engine(&[("hide", "decorations")]);
        for _ in 0..3600 {
            hidden.step(1).unwrap();
        }
        let b = hidden.backend();
        assert_eq!((b.deco_frames, b.max_commands), (0, 10));
    }

    #[test]
    fn sample_stats_count_the_credits() {
        let mut e = engine(&[]);
        let json = e.ai_stats();
        let commits = e.log().commit_count();
        assert!(
            json.starts_with(&format!(
                "{{\"registry\":1,\"loaded\":{{\"commits\":{commits},"
            )),
            "{json}"
        );
        for agent in ["\"id\":\"claude\"", "\"id\":\"codex\"", "\"id\":\"cursor\""] {
            assert!(json.contains(agent), "{agent} missing in {json}");
        }
        assert!(json.contains("\"model\":\"Claude Opus 5.5\""));
        assert!(json.contains(",\"played\":{\"commits\":0,"), "{json}");
        for _ in 0..600 {
            e.step(1).unwrap();
        }
        assert!(!e.ai_stats().contains(",\"played\":{\"commits\":0,"));
        let stats = e.simulation().attr_stats(e.log());
        let credited = SAMPLE.lines().filter(|l| l.contains("|ai=")).count() as u32;
        // every meta field starts a commit of its own in this sample
        assert_eq!(stats.loaded.assisted + stats.loaded.agent, credited);
    }
}
