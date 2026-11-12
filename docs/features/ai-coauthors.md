<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# F1: AI co-authored commits and the contributor style system

> **History.** A design record from the MVP. Mentions of `gource-server` refer to the original
> Rust clone service, which is not part of this repository; the hosted service implements the
> same API (`packages/protocol/src/api.ts`).

Status: draft for maintainer review. Date: 2026-10-02. Branch: `web-mvp` (MVP complete).
Scope: `docs/features/ai-coauthors.md`. Requirement ids `AI-n` are stable; tests and
milestones refer to them. Stories of other specs are cited with the feature (`F2 US-11`,
`F3 US-14`); a bare `US-n` is a story of this spec. Research notes:
`docs/features/research/data-path.md`, `docs/features/research/ai-trailers.md`,
`docs/features/research/user-render.md`. Shared decisions (D1 to D9), the glossary and
the shared constants (`SHARE_URL_SOFT_LIMIT`, `NAME_BEARING_OPTIONS`, the wasm size
budget) are in `docs/features/README.md`.

Paths are relative to the repository root. `file:line` anchors describe the tree at
HEAD `8a3ecec`.

## 1. Summary

When a commit was made with an AI coding agent ("Co-Authored-By: Claude Opus 5.5",
"Co-authored-by: Codex", an agent that is the author itself), the contributor who made it
gets visible gear in the scene: a cape in the agent's colour and shades when a human was
assisted; a halo, glow and badge when the agent is the author. The look is switchable and
driven by a rule list the viewer can edit, so the same mechanism later carries a
contributor's own claimed style.

The work has four parts. (1) A shared, versioned classifier in `gource-sources` turns commit
identities and trailers into attributions (agent id, model, role); no email leaves the
server. (2) The custom log gets an optional, extensible `key=value` field after an explicit
colour slot, so C++ Gource ignores it; the batch codec, `EventLog` and cache keys are
versioned for it. (3) The simulation keeps a per-contributor assist state tied to the
actions on screen, plus aggregates for hover text and an AI panel. (4) A style resolver
(rule list, cascade, JSON profile envelope) maps state to decorations that the frame
builder draws as sprite quads inside the existing quad stream (at most 2 extra draw calls)
and, for glow, inside the existing additive bloom stream (0 extra calls).

With no attribution data and no user rules, every frame is bit-identical to today.

What you will see after M1 (the first demo, section 9): open
`/?log=/samples/ai-coauthors.log`. The contributor `jane` wears a cape and shades while her
credited commits play and loses them a moment later; `Cursor Agent` has a halo, a glow
and a badge; everyone else looks as today. `hide=decorations` removes all of it and the
scene is then identical to one with no attribution data. M1b makes the `Co-authored-by`
trailers of a real repository do the same and adds a one-control choice of the built-in
look (`ai-style=default|cape|shades|hat|none`).

## 2. Goals / Non-goals

Goals

- G1. Make AI involvement in a repository's history visible and explorable: who is
  assisted, by which agent and model, how often, and when.
- G2. Classify from signals that already exist in git (trailers, identities) with a
  table-driven, versioned registry; one implementation in `gource-sources`. The server runs
  it; `gource-rs convert --from git` reaches it through the same parser and writer; the
  browser never classifies, because that would need emails in the page (AI-29, decision D6).
  Decision D5 (README) is amended accordingly: one classifier in `gource-sources`, used by
  the server pipeline and `gource-cli`; the browser never classifies. A git `--raw` file
  dropped on the landing page or opened with `?log=` is therefore shown without
  attribution; `gource-rs convert --from git` produces an attributed custom log.
- G3. Keep the interchange format backward compatible (C++ Gource reads the new logs
  unchanged) and extensible (issue and PR metadata will reuse the field).
- G4. A style system that is a rule list the viewer controls today and that accepts a
  contributor-claimed layer later without a format change (decision D4).
- G5. No change to default behaviour, frame hashes or draw-call counts for data without
  attributions.

Non-goals

- N1. Proving that an AI wrote anything. Trailers and identities are claims; the UI says
  "credited to" and "assisted by", never "written by".
- N2. Tracking human co-authors, reviewers or sign-offs.
- N3. Agent logos, brand marks or vendor colours in the scene (project constraint, CLAUDE.md
  and README).
  Decorations are generic shapes; agents differ by tint, badge shape and text. Tool names
  appear as plain text only to identify the tool; the project is not affiliated with or
  endorsed by any vendor (README line, section 10).
- N4. Detecting tools that leave no signal (Windsurf, Tabnine, Gemini CLI today; see
  section 11) or detection by diff content or commit style.
- N5. Per-viewer custom sprite art (uploading images). Only the six built-in decoration
  kinds, tinted.
- N6. Server-side storage of claimed styles, the claim API and its UI. They belong to the
  later spec "claimed styles" (F4); F2 owns accounts. This spec decides the ownership proof
  (AI-49, AI-50), defines the profile data model and reserves the wire key `cid`.
- N7. Classifying in the browser: a git `--raw` file opened locally or through `?log=` is
  shown without attribution (G2).

## 3. User stories

Each is testable; section 8 maps tests to them.

- US-1. As a viewer of a repository whose history has `Co-Authored-By: Claude Opus 5.5`
  trailers, I see the committing contributor wear a cape and shades while their commit's
  file actions play, and lose them a moment after.
- US-2. As a viewer, when I hover that contributor I read what is happening now,
  "AI-assisted: Claude Opus 5.5" (M2); the percentage form "credited to Claude Opus 5.5 in
  23% of 142 commits", counted up to the current playback position (the same numbers
  after a seek as after a straight playthrough), is M3 (Q16).
- US-3. As a viewer of a repository where `copilot-swe-agent[bot]` authors commits, I see
  that contributor with a halo, glow and badge for as long as it is active.
- US-4. As a viewer I can turn all decorations off with one control (`hide=decorations`),
  and the scene is then identical to a scene with no attribution data.
- US-5. (M2) As a viewer I can write `ai:codex => hat=agent; glow=agent` and see Codex-assisted
  contributors wear a glowing hat, with the rest of the default look unchanged.
- US-6. (M2) As a viewer I get a specific error naming the line and the problem when a rule is
  wrong, and the previous rules stay active.
- US-7. (M2) As a viewer I can show an agent key (like the file-extension key) that lists the
  agents seen so far with their colour and commit counts.
- US-8. (M2) As a viewer I can restrict the visualisation to AI-assisted commits, or exclude
  them (`ai-filter`), and the scene re-simulates from the downloaded history.
- US-9. As a viewer I can share a link that reproduces the same look (`?user-style=...`,
  M2), and export my rules as a JSON style profile and import one (M3).
- US-10. As a person who keeps custom logs, I can add `ai=co:claude:Claude Opus 5.5` to a
  log and see the same result; C++ Gource still reads the file.
- US-11. As a maintainer of the server, I am sure no contributor email appears in the
  cached log, the API responses or the page.
- US-12. As a viewer who is signed in (F2 account) and claims a contributor identity (D4b),
  my claimed style slots into the same resolver below the viewer's own rules (contract
  and ownership proof only here; storage, claim API and UI belong to the later spec
  "claimed styles", F4).
- US-13. (M3) As a viewer I can keep gear on for contributors who are habitually assisted
  (preset "Always on", selector `habitual`), at any playback speed, instead of only while a
  credited commit is on screen.
- US-14. (M3, Q16) As a viewer I can hide per-person AI numbers (`ai-person-stats=false`) and see only
  aggregate counts; the per-person contributor list is off unless I turn it on
  (`ai-leaderboard`).
- US-15. (M2) As a returning viewer my last rules and AI options are restored on any repository
  (browser storage), and a link's own options override them.
- US-16. (M2) As a viewer of a link or screenshot, I can tell when an AI filter is active or
  custom styling came from the link: a permanent text line while `ai-filter` is not `all`,
  and a dismissible notice when `user-style` or `ai-filter` arrive through the URL.
- US-17. (M2) As a viewer, a recording or a shared link shows the decorations the same
  options give with the same fixed-dt tick sequence (the settings snapshot, AI-72);
  `name:` rules (M3) on logs with more than 256 contributors may start a few ticks later
  than live.

## 4. UX

### 4.1 In the scene

Default look (built-in rules, section 5.8). Coordinates are fractions of the avatar quad
(`u` right, `v` down), see AI-55.

```
   assisted human                 agent as author

        ,-.                            ___            <- halo (behind head)
       ( o_o)--shades                 /   \
   /\   `-'   /\                     ( o_o )  glow around the whole figure
  /  \_/  \__/  \  <- cape           /|   |\
  \___ torso ___/     (agent tint)    \___/  [#] <- badge (agent's shape)
```

| Decoration | Group | Default rule | Meaning |
|---|---|---|---|
| cape | behind avatar | `assisted` | a human whose current work is credited to an agent |
| shades | front | `assisted` | same, the "superpower" cue |
| halo | behind avatar | `agent` | the contributor is an agent |
| glow | additive, over the scene | `agent` | the contributor is an agent |
| badge | front, lower right | `ai` | which agent (tint and shape), for both cases |
| hat | front | none by default | available through rules and presets ("glowing hat") |

Behaviour: decorations fade in over 0.25 s when an AI-credited commit's first action is
applied, stay while its actions are pending or active, then hold 2 s and fade out over
0.75 s (AI-31). They always follow the avatar's own fade (AI-57). The cape reads as a
superhero cape even standing still: it is wider than the torso at the shoulders and flares
to a hem just below the avatar's base, sways and billows gently at rest, leans against the
direction of motion and flutters when moving, and folds up toward the shoulders as it
fades out; `decor-motion=off` freezes sway, billow, lean and flutter (AI-77).
Selecting or hovering a contributor does not change them. Gear that depends on the current
state only through `human` or a negated term fades the opposite way (out while the
assist state fades in, AI-44), so no rule pops (inverse polarity is M3; in M2 such a rule is static). Milestones: M1 draws a
static cape (no lean or flutter; `decor-motion` is M2) and one badge shape (circle); the
nine other badge shapes, lean, flutter and `decor-motion` arrive in M2. The default look
is dynamic: a habitually
assisted contributor looks plain between credited commits; the preset "Always on" and the
`habitual` selector (AI-73) keep gear persistent.

### 4.2 Hover box

The hover text box (`crates/gource-core/src/sim/extract.rs:461-467`, today one line, the
name) gets a second line when attribution data exists and `ai-person-stats` is on
(default). M2 ships the current-state form only (`AI-assisted: Claude Opus 5.5`, or
`AI agent: GitHub Copilot`, for the contributor's state at that moment, no percentages);
the percentage rows below, `ai-person-stats` and the `automation account` row are M3 and
wait for Q16. Copy ("credited to" is deliberate: trailers are claims, N1):

| Situation | Line 2 |
|---|---|
| assisted in some commits, one agent dominant | `credited to Claude Opus 5.5 in 23% of 142 commits` |
| several agents | `credited to Claude Opus 5.5 and others in 23% of 142 commits` |
| assisted in every commit | `credited to Codex in all 14 commits` |
| below 1% | `credited to Claude Opus 5.5 in <1% of 842 commits` |
| agent as author | `AI agent: GitHub Copilot (all 31 commits)` |
| generic bot | `automation account (dependabot)` |
| no attribution | no second line |

The model string is shown when known, otherwise the agent display name. "Commits" are the
commits up to the current playback position under the current filters. The number is
seek-invariant: after a seek it equals a straight playthrough to the same position
(AI-32). The percentages are lower bounds (trailers can be removed, Q5) and 0% is not proof
of no AI use; the panel carries the caveat "From commit trailers; not proof, may be
undercounted." With `ai-person-stats=false` there is no second line (AI-76).

### 4.3 Agent key

Option `agent-key` (default off) draws a list at the top right, same type style as the
file-extension key: a bullet in the agent's colour, the agent display name, the number of
commits up to the current playback position with that agent credited (seek-invariant). At most 8 rows, sorted by count descending then id; a last
row "N more" when truncated. It sits right-aligned below the date, its first row at a fixed
y = 68 logical pixels so the web page's top-right toolbar never covers it (AI-59), and does
not move or animate.

```
                                   Jul 14 2026
                                 (o) Claude   1,204
                                 (o) Codex      311
                                 (o) Copilot     87
```

### 4.4 Panel: new section "Contributors & AI"

Milestones: M1b ships only the `Built-in look` select (`ai-style`: Default, Cape only,
Shades only, Hat only, None) and the "Show decorations" checkbox. M2 adds Agent key, AI
commits filter, the rules textarea with presets, Reduce motion and the current-state hover
line. M3 adds Per-person numbers, Contributor list, Export/Import and the "AI in this
repository" histogram. The sketch shows the end state.

A fifth section `style` in `web/src/options.ts` (`SectionId`, currently four: `look`,
`playback`, `visibility`, `filters`, `web/src/options.ts:9-18`).

```
Contributors & AI
  [x] Show decorations                     (hide=decorations, inverted)
  [ ] Agent key                            (agent-key)
  AI commits   (o) All  ( ) Only assisted  ( ) Exclude assisted   (ai-filter, re-simulates)
  [x] Per-person AI numbers                (ai-person-stats)
  [ ] Contributor list                     (ai-leaderboard)
  [ ] Reduce motion                        (decor-motion=off; set once from the OS preference)
  Built-in look  [ Default v ]             (ai-style: Default | Cape | Shades | Hat | None)
  My rules                                                     [Preset v] [Export] [Import]
  +-------------------------------------------------------------+
  | ai:codex => hat=agent; glow=agent                           |
  | name:/^release-bot/i => badge=on                            |
  +-------------------------------------------------------------+
  ! Line 2: style_bad_property: unknown property "capes". Known: cape, halo, ...
  AI in this repository (loaded)            commits   assisted   by agent
    Claude   Claude Opus 5.5 ......  412   ####....
    Codex                            57   #.......
  From commit trailers; not proof, may be undercounted.
  Top contributors (only with "Contributor list" on)
    jane  142 commits  33 credited
```

The error line is produced from `GourceApp.check_style(text)` (AI-46), which carries the line
number and validates live without applying. While `ai-filter` is not `all` the HUD also draws a permanent text line
under the date (`AI commits: only assisted`, AI-74), so screenshots and recordings carry it.
The AI section and the filter control are disabled, with the note "This log has no AI
attribution data (detection off or log predates it).", when the server reported
`ai_detect: false` or the loaded log has no attribution sets (AI-29).

Presets (TypeScript constants that fill the rules box, not new options): "Hero" (empty,
the built-in look), "Minimal" (`ai => cape=off; shades=off; halo=off; glow=off`), "Glowing
hats" (`assisted => cape=off; shades=off; hat=agent; glow=agent`), "Always on" (M3,
`habitual => cape=agent; shades=on`, AI-73). The row editor (toggles and colour pickers
that generate rules) is not part of this feature (Q14); the textarea is the editor.

Error copy. Each message starts with a stable code (AI-46); the panel shows it after
"Line N:".

| Code | Message |
|---|---|
| `style_missing_arrow` | Rule needs a selector, `=>` and properties. |
| `style_bad_selector` | Unknown selector "X". Use human, bot, agent, assisted, habitual, ai, ai:AGENT, ai:AGENT/MODEL or name:/REGEX/. (M2 lists only human, agent, assisted, ai and ai:AGENT.) |
| `style_bad_regex` | The regular expression in name:/.../ is invalid or longer than 128 characters. |
| `style_bad_property` | Unknown property "X". Known: cape, shades, hat, halo, glow, badge, colour, scale, none. |
| `style_bad_value` | Property "P" does not accept "V". Use on, off, agent, hash or RRGGBB (scale: 0.5 to 2). |
| `style_too_many_terms` | A selector can combine at most 4 terms with &. |
| `style_too_many_rules` | At most 32 rules. |
| `style_too_many_lines` | At most 64 lines, comments included. |
| `style_line_too_long` | A rule can be at most 256 bytes. |
| `style_too_many_regexes` | At most 8 name:/.../ terms in total. |
| `style_reserved` | The selector "id:" is reserved for a later version. |
| `profile_bad_format` | Not a Gource style profile (JSON). |
| `profile_version_unsupported` | Style profile version N is newer than this page understands (1). |

M2 ships the codes of the minimal grammar (`style_missing_arrow`, `style_bad_selector`,
`style_bad_property`, `style_bad_value`, `style_too_many_terms`, `style_too_many_rules`,
`style_too_many_lines`, `style_line_too_long`, `style_reserved`); `style_bad_regex`,
`style_too_many_regexes` and the two `profile_*` codes arrive with the M3 features they
belong to.

### 4.5 URL

Non-default options only, repeated for lists, as the existing URL code does
(`web/src/url-state.ts:41-90`):
`/?repo=github.com/o/r&user-style=ai%3Acodex%20%3D%3E%20hat%3Dagent&ai-filter=only-assisted&hide=decorations`.
When `user-style` or `ai-filter` arrive through the URL the page shows a dismissible
notice, "This link applies custom styling" or "This link filters AI commits" (AI-74), and
the URL wins over the viewer's stored style (US-15).

## 5. Design

### 5.1 Data flow and layering

```
 git log (server)                  custom log (any producer)
   |  m1: meta line                   |  ts|user|A|/p|colour|ai=...;bot=...
   v                                  v
 GitRawParser --classify()--> ParsedCommit.attrs <-- CustomLogParser (meta field)
   |  (gource-sources)               |
   v                                  v
 CustomLogWriter -> cache (.log) --> LogParser (worker) -> BatchEncoder v2 -> apply_batch
                                                                              |
                                      EventLog { attr sets, CommitRecord.attrs }
                                                                              v
   gource-core: RAction.attr -> RUser assist state -> style Resolver -> UserView.decor
                                                                              v
   gource-render: back quads | avatars | front quads | glow (bloom stream) | HUD key
```

Layer ownership:

| Concern | Crate | New module |
|---|---|---|
| agent table (id, display name, tint, badge) | `gource-core` | `agents.rs` |
| detection patterns, trailer parsing, dedupe, registry digest test | `gource-sources` | `classify.rs` |
| wire field grammar | `gource-sources` | `meta.rs` |
| attribution storage, assist state, stats | `gource-core` | `attr.rs`, `user.rs`, `sim.rs` |
| rule grammar, cascade, resolver | `gource-core` | `style.rs` |
| decoration quads, key | `gource-render` | `deco.rs`, `agent_key.rs` |
| sprite atlas texture | `gource-render-webgl2` | `build.rs`, `textures.rs` |

Rationale for the split: `gource-core` is the only crate shared by sources (via
`events.rs`) and render, and it has no I/O. Putting the agent table there lets display
names and tints resolve on the client for ids the classifier never saw (custom logs), while
the patterns, which are a source concern, stay out of the simulation.

### 5.2 Signals and classification (`gource-sources`)

**AI-1.** `gource-core/src/agents.rs` holds `AGENTS: &[AgentInfo]` with
`{ id: &str, name: &str, tint: [u8;3], badge: BadgeShape }` and
`pub const REGISTRY_VERSION: u32 = 1`. `gource-sources/src/classify.rs` holds the matchers,
one `AgentMatcher` per agent, referencing ids from `AGENTS`. Ids match `[a-z0-9][a-z0-9-]{0,31}`.
An id that is not in `AGENTS` is valid (custom logs, newer registries): display name is the
id, tint is `colour_hash(id)`, badge is `hash(id) % 10`.

**AI-2.** Signals, all read per commit:

| Signal | Source | Role it yields |
|---|---|---|
| `Co-authored-by:` trailer | `%(trailers:key=Co-authored-by)` | Assist |
| `Made-with:` trailer (value is a tool name, no email) | `%(trailers:key=Made-with)` | Assist |
| author identity (raw name and email) | `%an`, `%ae` | Agent |
| committer identity, only when the author did not match (M3) | `%cn`, `%ce` | Assist |
| name suffix `(aider)` on the author (M3) | `%an` | Assist (suffix stripped from the contributor name, AI-4) |
| subject prefix `aider: ` (M3) | `%s` | Assist |

v1 (M1b, M2) reads the two trailers and the author identity only. The three rows marked
(M3) are specified for later and are not read before M3: they are weak or collide with
ordinary text (an `aider: ` subject prefix is also a conventional-commit scope), and the
suffix is the only identity rewrite in the system. The git command still emits the committer
pieces (AI-12) so that enabling them needs no format change; the v1 parser ignores them.

Why committer is Assist (M3): a human-authored commit that an agent committed (rebase, amend,
apply) is "human assisted"; it is rare and weak, so it never overrides an author match.
Why the aider suffix and subject prefix are Assist: aider is a tool a human drives, like
Claude Code, and the prefix `aider: ` is also an ordinary conventional-commit scope in
package repositories, so on its own it must not make a person look like an agent.
The raw (unmapped) `%an %ae %cn %ce` are used for classification, not the mailmapped
`%aN`, because a mailmap that renames an agent identity into a person must not erase the
fact. It cannot make that person an agent either: when the mapped author name (`user:`
line) differs from the raw `%an`, an Agent verdict is downgraded to Assist (vector T25).
The contributor shown is still the existing mailmapped `%aN`
(`crates/gource-server/src/pipeline.rs:749`), with one exception (AI-4); identity handling is
otherwise unchanged (decision in section 5.7).

**AI-3.** Matching rules, in this order, case-insensitive ASCII, whitespace trimmed:

1. exact email from `AgentMatcher.emails` (for example `noreply@anthropic.com`);
2. GitHub login: `AgentMatcher.gh_logins` matches when the email is
   `<digits>+<login>@users.noreply.github.com` (compare the login only, never the numeric id),
   or when the login ends in `[bot]` and the name equals the login;
3. special rules: `made_with_values` (v1); `author_name_suffix` and `subject_prefix` (M3).

A name alone never matches (a person called "Claude" or "Amp" is a person), except through
rule 2 for `[bot]` logins. Numeric ids are never compared because they are not stable
identifiers of the tool: the Claude Action's default email carries id 41898282, which is
`github-actions[bot]`'s id, while `claude[bot]` has id 209825114 (`docs/features/research/ai-trailers.md`
section 2).

**AI-4.** Role precedence per commit and agent: Agent beats Assist. If the author matches
agent A, trailers naming A are dropped, and the Agent entry adopts the model of a dropped
trailer when it has none (the Copilot coding agent's trailer credits the human requester,
which is not an agent and is ignored anyway). Human co-authors are not
recorded (N2). Identity exception (M3, with the aider suffix signal): when the ` (aider)` author suffix yields an Assist
verdict, `GitRawParser` removes the suffix (and the space before it) from the contributor
name, so a person who uses aider is one contributor, like a person who uses Claude Code.
This is the only identity rewrite; it exists only in the git path (custom logs keep names
verbatim).

**AI-5.** De-duplication and order, because squash merges copy trailers and the same
trailer appears with different key case: entries are unique by `(kind, agent, model)`.
An entry without a model is dropped when another entry for the same agent and kind has
one. Order is `(kind: Agent first, registry index, model)`; the canonical order is part of
the wire format so output is deterministic. At most 4 entries per commit; extra entries are
dropped and counted (`ClassifyStats.truncated`).

**AI-6.** Model extraction. A model is accepted only if it matches the bounded grammar of
its agent (ASCII, trimmed, at most 64 bytes); anything else makes the model `None` and the
agent is kept. The grammars exist because the trailer name is free text that anyone can
forge next to a real person's name (the email is the claim the classifier checks, the name
is not): only a bounded shape ever reaches hover, key and panel.

| Agent | Where | Rule |
|---|---|---|
| claude | trailer display name | Open family grammar, so a new model family needs no registry edit and no `REGISTRY_VERSION` bump: `^Claude [A-Z][a-z]{1,15}( [0-9]+(\.[0-9]+)*)?$` or `^Claude [0-9]+(\.[0-9]+)* [A-Z][a-z]{1,15}$`, ASCII only, with the family word `Code` excluded (it is the tool name): `Claude Opus 5.5`, `Claude Haiku 4.5`, `Claude Sonnet 5`, `Claude Fable 5`. `Claude`, `Claude Code`, `Claude is why Bob broke prod`, a URL or any other text give `None`. The shape stays ASCII, length-bounded and email-free, so a forged trailer name cannot carry free text or an address. |
| aider | trailer name `aider (<model>)` | `<model>` matches `^[A-Za-z0-9._-]+(/[A-Za-z0-9._:-]+)?$` (`gpt-5.4`, `gemini/gemini-3.1-pro-preview`, `ollama/llama3:8b`); anything else gives `None`. |
| others | none | model is `None`; the agent display name is shown. |

The wire grammar (AI-16, AI-17) is wider, because custom logs may name any agent and model;
the bounded grammars above apply to what the git classifier emits.

**AI-7.** Generic automation is not AI. In v1 (M1, M2) such an author, and any `[bot]` login that no agent matcher claims, is excluded: the commit gets no attribution and the contributor is a plain contributor. The Bot kind, the `bot` meta key and the `bot` selector below are specified for M3 (enabled when a bot-specific visual is wanted); until then the wire grammar reserves Bot kind code 2 and the `bot` key and no writer emits them. From M3: an author whose name ends in `[bot]`, or whose email
matches `<digits>+<login>[bot]@users.noreply.github.com`, and that no agent matcher claims,
yields kind Bot with id = sanitised lowercase login without `[bot]` (`dependabot`,
`renovate`, `github-actions`). A Bot entry is never combined with an Agent entry for the
same commit.

**AI-8.** The classifier output holds ids, models and kinds only. Names, emails and subjects
are inputs and are dropped on return. The public entry point is a pure function with no
allocation when no signal matches:

```rust
pub struct Identity<'a> { pub name: &'a str, pub email: &'a str }
pub struct Trailer<'a>  { pub key: &'a str, pub value: &'a str }
pub struct CommitSignals<'a> {
    pub author: Identity<'a>, pub committer: Identity<'a>,
    pub subject: &'a str, pub trailers: &'a [Trailer<'a>],
}
pub fn classify(signals: &CommitSignals<'_>, out: &mut AttrBuf); // AttrBuf: <= 4 entries, inline
```

**AI-9.** Registry v1 contents. Tints come from the Okabe-Ito colour-blind-safe set where
possible and are not vendor colours; badge shapes disambiguate similar tints.

| id | Display name | Tint | Badge | Matches (v1) |
|---|---|---|---|---|
| `claude` | Claude | E69F00 | circle | email `noreply@anthropic.com` (author: Agent, trailer: Assist); login `claude[bot]` |
| `codex` | Codex | 56B4E9 | diamond | trailer/author email `noreply@openai.com`; login `chatgpt-codex-connector[bot]` |
| `copilot` | GitHub Copilot | 009E73 | hexagon | logins `copilot-swe-agent[bot]` and `Copilot` (email form only); email `copilot@github.com` |
| `cursor` | Cursor | CC79A7 | triangle | email `cursoragent@cursor.com`; `Made-with: Cursor` |
| `aider` | Aider | F0E442 | square | email `aider@aider.chat` (author: Agent, trailer: Assist); author suffix ` (aider)` and subject prefix `aider: ` (both Assist, M3) |
| `jules` | Jules | 0072B2 | star | login `google-labs-jules[bot]` |
| `devin` | Devin | D55E00 | ring | email `devin@cognition.ai`; login `devin-ai-integration[bot]` |
| `amp` | Amp | A78BFA | plus | email `amp@ampcode.com` |
| `openhands` | OpenHands | 8BC34A | drop | email `openhands@all-hands.dev` |
| `amazon-q` | Amazon Q Developer | C0C0C0 | bolt | login `amazon-q-developer[bot]` |

Every row is backed by a real commit or vendor document in `docs/features/research/ai-trailers.md`
(References, section 12). Not in v1 and why: Gemini CLI (no official convention; the only
suggested identity is the Code Assist app, a different product), Windsurf, Tabnine and Sweep
(no documented trailer or identity found). Adding one is a registry edit plus a version
bump.

**AI-10.** Registry integrity. `gource-sources` depends on `gource-core`, so the test lives
in `gource-sources/src/classify.rs`: it hashes the serialised `AGENTS` (from core) and
`AGENT_MATCHERS` tables (FNV-1a) and compares the digest with the constant `REGISTRY_DIGEST`
stored there, and asserts `gource_core::agents::REGISTRY_VERSION == EXPECTED_REGISTRY_VERSION`
(a second constant beside it). Any table change fails the test until the digest is updated;
a change to what the matchers detect also needs a `REGISTRY_VERSION` bump, a cosmetic one
(name, tint, badge) does not. A bump invalidates every cached log (AI-28), so it is
reserved for behavioural changes. A second test checks that every matcher references an id
present in `AGENTS`.

**AI-11.** Unknown tools stay undetected. Producers of custom logs can still attribute any
agent by writing the field (section 5.4). An operator-supplied registry file is a possible
later extension (Q13).

### 5.3 Git command and `GitRawParser`

**AI-12.** The server's `git log` format
(`crates/gource-server/src/pipeline.rs:744-752`, today
`--pretty=format:user:%aN%n%ct`) becomes:

```
--pretty=format:user:%aN%n%ct%nm1:%an%x1f%ae%x1f%cn%x1f%ce%x1f%(trailers:key=Co-authored-by,key=Made-with,unfold,separator=%x1f)%x1e%s
```

Checked with git 2.43.0 in a scratch repository while drafting this spec: repeated `key=`
options select several trailer keys, `unfold` joins folded values, a TAB inside a subject
survives in the output, and the line stays one physical line (the parser test in section 8
reproduces the run). The line has
no leading or trailing blank. A merge header has no raw lines and is not followed by a blank
line: it is immediately followed by the next commit's `user:` line, exactly as today (module
doc `git.rs:15-24`; confirmed with git 2.43.0 on a scratch repository using the AI-12
format). Raw `--raw` lines always start with `:`, so a line starting `m1:` cannot be
mistaken for a file line.

**AI-13.** `GitRawParser` (`crates/gource-sources/src/git.rs:39-45` states `Block`,
`Timestamp`; `line()` at `git.rs:234`) gains a state `Meta` entered after the timestamp
line (`git.rs:239-248`). In `Meta`, a line starting `m1:` is consumed unconditionally (it
may contain TAB, the subject is free text), parsed as: the part before the first `0x1e` is
split on `0x1f` into `an, ae, cn, ce, trailer...`; the part after is the subject (only the
first 16 bytes matter). Fewer than 5 pieces (four identities plus the trailer slot, which is
empty but always present) gives no attribution and increments `ParseStats.bad_meta`
(AI-27). A line not starting `m1:` is processed as before, so logs from the old command
(and every existing fixture) parse unchanged. A trailer piece is split at its first `:` into
key and value; `Name <email>` values are split at the last `<`.

Per-commit state. `user:` resets the whole pending state (username, timestamp and
attributions); `m1:` assigns the attributions (never merges into earlier ones) and is
accepted only in `Meta`; the state is consumed once per header. So a merge header's `m1:`
credits are discarded when the next `user:` line arrives and can never leak onto the next
commit, even when that commit's `m1:` line is missing or malformed.

Hostile bytes. Names, emails and trailer values are arbitrary bytes, including `0x1f` and
`0x1e`. A commit is treated as unattributed, with `bad_meta += 1`, when any of the four
identity pieces contains a C0 control, when an email piece (2nd, 4th) contains whitespace,
or when a trailer piece has no `:` (each means a separator inside a name shifted the
fields). Subject bytes after the first `0x1e` are free text and ignored beyond the first 16.
The parser also compares the raw `an` with the `user:` name for AI-2 (mailmap downgrade)
and applies the AI-4 aider suffix strip here.

**AI-14.** The format needs git 2.22 or newer (`key=`, `valueonly`/`separator=` arrived in
2.22; `unfold` in 2.21; https://git-scm.com/docs/git-log). There is one startup
probe of `git --version` (`GitInfo`), shared with F2 PR-4: with F2's private mode on the
floor is 2.31, otherwise 2.22. Polarity, stated once: `GOURCE_AI_DETECT=1` enables
detection and `GOURCE_AI_DETECT=0` (flag form `--no-ai-detect`) disables it; the default
is enabled, unless the section 7 gate fails, in which case the shipped default is `0` and
detection becomes a per-request opt-in (the analysis request carries `detect_ai: true`,
which selects the `+ai<REGISTRY_VERSION>` cache key of AI-28 for that job), so the feature
stays reachable. Below git 2.22,
or with detection disabled, the server logs a warning and keeps the old command, so no
attribution is produced and everything else works. The effective state (flag and probed
version) selects the cache key (AI-28) and is reported as `ai_detect` (AI-29).
The pinned image installs the Debian package (`Dockerfile:59-62`); the probe makes the
version assumption checkable instead of implied. The `.mailmap` blob is not applied to
trailers (verified locally), so classification uses raw trailer emails.

### 5.4 Custom-log extension

C++ reference: the regex is
`^(?:\xEF\xBB\xBF)?([^|]+)\|([^|]*)\|([ADM]?)\|([^|]+)(?:\|#?([a-fA-F0-9]{6}))?`
(`src/formats/custom.cpp:21`), unanchored, so everything after the match is ignored. A field
placed directly after the path is hazardous (`1|u|A|/a|deadbeef` reads `deadbe` as a
colour); after an empty colour slot it is safe (`1|u|A|/a||X` gives no colour). The Rust
parser is equivalent (`crates/gource-sources/src/custom.rs:293-329`, `:388-400`, tests
pinned at `custom.rs:563-571`).

**AI-15.** Line shape, one new optional sixth column, always preceded by the colour slot
(empty or `[#]RRGGBB`):

```
timestamp|username|A/M/D|/path|<colour or empty>|<meta>
```

The writer emits `<meta>` only on the first line of a commit and only when the commit has
attributions (AI-21). The parser splits the text after the path's `|` at its next `|`: the
left part is the colour slot (parsed exactly as today, junk after six hex digits ignored),
the right part is `<meta>`.

**AI-16.** Meta grammar (ABNF-like). Unknown keys are ignored, so the field carries future
keys (`pr`, `issue`, `ref`, contributor ids) without a format change.

```
meta   = [ pair *( ";" pair ) ]
pair   = key "=" value                           ; value grammar depends on the key
key    = lower *( lower / DIGIT / "-" )          ; 1..24 chars
value  = *( vchar / pct )                        ; every key except `ai`
vchar  = any UTF-8 scalar except C0 and C1 controls, DEL, bidi controls
         (U+061C, U+200E, U+200F, U+202A-202E, U+2066-2069), "%", ";", "|", "="
ai     = credit *( "," credit )                  ; the value of key `ai`
credit = kind ":" comp [ ":" comp ]              ; kind, agent, model
comp   = *( cchar / pct )
cchar  = vchar except ":" and ","
pct    = "%" HEXDIG HEXDIG                       ; UTF-8 bytes, either hex case
```

Keys defined by this feature:

| Key | Value | Meaning |
|---|---|---|
| `ai` | `credit *( "," credit )`, `credit = kind ":" agent [ ":" model ]`, `kind = "co" / "au"` (the `ai` sub-grammar above) | AI attributions. `co` = a human's commit credited to the agent (Assist); `au` = the agent authored it (Agent). |
| `bot` | `bot-id` (`[a-z0-9][a-z0-9-]{0,31}`) | generic automation account (not AI); reserved, emitted from M3 (AI-7) |
| `cid` | 16 base64url characters | repo-scoped contributor pseudonym for claimed styles (AI-50); emitted only by the later spec "claimed styles" (F4), public repositories only |
| `x-*` | any | reserved for private extensions; ignored by the reader |

Escaping rule: writers percent-encode `%`, `;`, `|`, `=`, C0 and C1 controls and DEL in
every value, and additionally `:` and `,` inside an `ai` component (agent or model). The
`:` and `,` that form the credit structure are literal, so `ai=co:claude:Claude Opus 5.5,co:codex`
is valid and a model `a:b` is written `a%3Ab`. A space and `/` stay literal; leading and
trailing spaces are trimmed by writer and reader. Readers accept any percent-escape but
reject an escape that is not two hex digits or that decodes to invalid UTF-8, and reject a
decoded value containing a C0 or C1 control, DEL or a bidi control (the pair is ignored,
`bad_meta += 1`).

**AI-17.** Limits (reader and writer): the whole `<meta>` at most 512 bytes after escaping
(longer: whole field ignored); key at most 24 bytes; at most 4 credits per `ai`
(extra credits are not read); `agent` at most 32 bytes, `model` at most 64 bytes decoded
from the charset `[A-Za-z0-9 ._:/+-]` (otherwise: the whole `ai` pair is ignored); at most
8 pairs. Writer overflow: when the escaped field would exceed 512 bytes (four 64-byte
models full of `:` or `/` can), the writer drops trailing credits in AI-5 canonical order
until it fits and counts each drop in `WriteStats.meta_trimmed` (surfaced by the server's
job stats), so attributions are trimmed deliberately instead of the whole field being
discarded on read.

**AI-18.** Malformed values never cost the line. The rule is pair-level:

| Input | Behaviour |
|---|---|
| field longer than 512 bytes | whole field ignored, `bad_meta += 1` |
| pair without `=`, bad escape, bad key | that pair ignored, `bad_meta += 1` |
| unknown key | ignored silently |
| `ai` with a bad credit (unknown kind, bad agent id, over-long model) | whole `ai` pair ignored, `bad_meta += 1` |
| duplicate key | first wins, later ignored silently |
| legacy sixth column text such as `more` (`custom.rs:563-571`) | not `key=value`: ignored, `bad_meta += 1`; the colour is still read |

The file-colour of the line and the file change itself are never affected.

**AI-19.** C++ compatibility is verified, not argued: new cases in
`crates/gource-sources/tests/cpp-reference/cases.txt` (lines with `||ai=...`,
`|ff0000|ai=...;bot=...`, percent escapes, 512-byte fields) are run through the real
`harness.cpp` by `regen.sh`, and the recorded `expected.txt` must equal the result for the
same lines without the field. Documented limitation: C++ `--output-custom-log` drops the
field when it rewrites a log.

**AI-20.** Grouping is unchanged (same timestamp and same raw username, `custom.rs:181-191`).
For a group, the parser takes the union of the credits on all lines that carry a field,
de-duplicated and ordered as in AI-5. The union matters because two real commits of one
author with equal `%ct` collapse into one custom-log commit (data-path.md section 3) and the
writer puts each real commit's field on its own first line.

**AI-21.** `CustomLogWriter` (`crates/gource-sources/src/writer.rs:43-58`) writes, for the
first line of a commit with attributions, the colour slot (empty when the file has no
colour) then `|` and the meta text; other lines are unchanged. Sanitising reuses
`write_field` (`writer.rs:61`) for names and the AI-16 escaper for meta values.

**AI-22.** `BoundedWriter` (`crates/gource-server/src/pipeline.rs:596-636`) adds to its bound
`1 + 512` bytes per commit that has attributions (`LINE_OVERHEAD` at `pipeline.rs:608`
covers the colour slot but not the extra `|` and field).

**AI-23.** Examples (all valid; the first two are what the server writes):

```
1790942378|jane|A|/src/a.rs||ai=co:claude:Claude Opus 5.5
1790942378|jane|M|/src/b.rs
1790942500|Cursor Agent|A|/x.ts|ff8800|ai=au:cursor
1790942600|dependabot[bot]|M|/Cargo.lock||bot=dependabot
1790942700|jane|M|/a.rs||ai=co:claude:Claude Opus 5.5,co:codex
1790942800|Jane|A|/y.py||ai=co:aider:gemini/gemini-3.1-pro-preview
1790942900|jane|M|/z.py||ai=co:aider:ollama/llama3%3A8b
```

### 5.5 Batch codec, `EventLog`, sink order

**AI-24.** Batch format version 2 (`crates/gource-sources/src/batch.rs:89-90`, header 24
bytes, hard reject on mismatch at `batch.rs:428-430`). The worker and page ship in one
bundle, so the bump is safe; version 1 batches are rejected with the existing error.

```
header  = "GEB" version=2  first_string first_set first_commit  (u32 each, LE)
          string_count set_count commit_count change_count      (32 bytes in all)
strings = string_count * ( len: varint, bytes )                  ; as v1
sets    = set_count * ( n: varint (1..=4),
                        n * ( kind: u8 (0 assist, 1 agent, 2 bot: reserved, M3),
                              id: varint string id,
                              model: varint (0 = none, else string id + 1) ) )
commits = commit_count * ( timestamp zigzag, user varint, offset zigzag,
                           head: varint = (changes << 1) | has_set,
                           [set: varint, 1-based, global to the stream],
                           changes * (...) as v1 )
```

Set ids are global like string ids (first set of the stream is 1). `apply_batch` validates
`first_set == log.set_count()`, every string id below the limit, `n` in range, set ids
in range, all before touching the log (the all-or-nothing rule of
`batch.rs` module docs stays).

**AI-25.** `gource-core/src/events.rs` gains `AttrKind { Assist, Agent, Bot }`,
`Attribution { kind, id: StrId, model: Option<StrId> }` (Copy, 12 bytes), and in `EventLog`
(`events.rs:88-96`) a flattened table plus a lookup that interns whole sets by content:
`intern_attr_set(&[Attribution]) -> AttrSetId` (`AttrSetId(0)` = none). `CommitRecord`
(`events.rs:67-75`) gains `attrs: u32`; with `i64 + 4 * u32 + u64` the struct stays 32 bytes
(a test asserts `size_of::<CommitRecord>() == 32`). `push_commit` keeps its signature and a
new `push_commit_attr(..., attrs)` takes the set id. `EventLog` also keeps
`attr_commit_counts: Vec<u32>` (commits per set id), updated at push time, for the totals in
AI-33. The table is capped at 65,536 distinct sets. The cap is applied before the log is
touched, by the parsers (AI-27), so every consumer sees the same input; `apply_batch` and
`EventLogSink` additionally reject a stream whose set count would pass the cap, as a
defence that the in-tree encoder cannot trigger.

**AI-26.** Interning order, the contract of `sink.rs:11-20` extended: for each commit, the
username, then for a set not seen before the agent id and model of each entry in set order,
then file paths. `EventLogSink` (`sink.rs:36`) and `BatchEncoder` (`batch.rs:190`) both
follow it, so ids match on both sides; the existing round-trip tests
(`gource-sources/tests/batch.rs`, `sink.rs`) get attribution cases.

**AI-27.** `ParsedCommit` (`crates/gource-sources/src/lib.rs:52-60`) gains
`attrs: &'a [ParsedAttribution<'a>]` (empty = none), and `ParseStats` (`lib.rs:69-76`) gains
`bad_meta: u64`. `LogParser` exposes `meta_ignored()` next to `skipped_lines()`
(`crates/gource-web/src/wasm.rs:249`).

The 65,536-set cap lives in the same layer: each parser (`GitRawParser`,
`CustomLogParser`) normalises a commit's credits to the canonical AI-5 form and keeps a
hash set of the distinct canonical sets it has yielded; past the cap it yields an empty
`attrs` slice and counts `bad_meta`. `EventLogSink` and `BatchEncoder` therefore number
the same sets, which keeps the id-synchronisation contract of `sink.rs:11-20` and
`batch.rs:416` and avoids a `first_set != log.set_count()` hard reject mid-stream. One
parser instance feeds one `EventLog` (the worker's `LogParser` per loaded log). Test: a
log with 65,537 distinct sets round-trips through encoder and `apply_batch`, the last
commit gets set `0`, and loading completes.

### 5.6 Server and cache

**AI-28.** `KEY_VERSION` (`crates/gource-server/src/cache.rs:19-20`, `"gource-log-v1"`)
becomes `"gource-log-v2"`, and the key string also folds in the detection state of the run:
`format!("{KEY_VERSION}+ai{REGISTRY_VERSION}")` when detection is active (AI-14) and
`"{KEY_VERSION}+ai0"` when it is not (flag off or git older than 2.22), in both
`hash_fields` branches (`cache.rs:60-75`). The same prefix, `cache_key_prefix`, is the first
component of every private cache key (F2 PR-60). A registry bump therefore re-analyses
repositories instead of serving logs classified by an older table, and flipping
`GOURCE_AI_DETECT` or upgrading git never serves a log produced under the other state (an
operator who disabled detection for privacy never gets cached attributed logs). Cost, stated
openly: a `REGISTRY_VERSION` bump is a full cache invalidation (a re-clone and re-analysis
of every repository, and GitHub rate pressure); hence AI-10 reserves bumps for behavioural
changes. Unit test: the key differs for `ai1` versus `ai0` versus a different registry
version. Old entries are not matched by the same-variant cleanup
(`cache.rs:91-100`) and leave through LRU eviction (`cache.rs:102-117`). Old logs without the
field still parse (the field is optional), so a rollback is safe.

**AI-29.** Classification happens once, on the server, and is stored in the cached log; the
browser never reclassifies a log that has a field. Logs without the field are shown without
attribution (no inference from names, section 11 Q9). The server returns
`ai_detect: true|false` (the effective state of AI-14) in the job result (top level, next to `repo`; F2's
`repo.visibility` stays inside `repo`) and in the cache metadata beside the log (for
private jobs: the ephemeral sidecar in F2 M1, the index row from F2 M2a, `meta_enc` from F2
M2b, F2 PR-64); the page passes it to the AI section. When it is `false`, or the
loaded log has no attribution sets (`ai_stats().loaded.sets == 0`), the AI section and the
`ai-filter` control are disabled with the copy "This log has no AI attribution data
(detection off or log predates it).", so a viewer never reads "0 AI commits" as a fact.
Logs loaded with `?log=` or from a file have no `ai_detect` and use the second condition
only. e2e test: the disabled state (section 8.3).

### 5.7 Simulation semantics (`gource-core`)

Identity decision. The contributor remains the raw `%aN` author name, for agent-authored
commits as well. Reasons: filters (`user-filter`, `highlight-user`, `follow-user`) and
parity with C++ keep working on the name people see in the repository
(`sim.rs:1267-1268`, `filters.rs:131-136`); no identity rewriting is needed; the agent
identity is attached as state, not as a second naming scheme. The one exception is the
aider author suffix, which the git parser strips (AI-4), so "Jane (aider)" and "Jane" are
one contributor, assisted.

**AI-30.** Per-action credit. `RAction` (`crates/gource-core/src/action.rs:39-47`, Copy)
gains `attr: u32`, the commit's set id (0 = none). `add_file_action`
(`sim.rs:1258-1297`) copies `commit.attrs` into every action of the commit, including
Remove actions and directory-delete expansions.

**AI-31.** Assist state. `RUser` (`user.rs:64-85`) gains
`attr_set: u32`, `attr_env: f32`, `attr_hold: f32`. In `RUser::logic` (`user.rs:358`), after
the existing action loop:

1. `credited` = some active action has `attr != 0` and is not finished after its `logic`
   call, or the first pending action (`actions.front()`) has `attr != 0`. `attr_set` takes
   that action's set id. (The active list is already iterated by `retain_mut`; checking one
   field costs nothing, and only the front of the pending queue is read, so a 200k-file
   commit is O(1) extra.)
2. If `credited`: `attr_hold = ASSIST_HOLD` (2.0 s). Else `attr_hold = max(0, attr_hold - dt)`.
3. `target = 1` if `credited || attr_hold > 0`, else 0;
   `attr_env += clamp(target - attr_env, -dt / ASSIST_FADE_OUT, dt / ASSIST_FADE_IN)`
   with `ASSIST_FADE_IN = 0.25`, `ASSIST_FADE_OUT = 0.75`.
4. When `attr_env == 0` and not `credited`, `attr_set = 0`.

`dt` is the scaled simulation `dt` already passed to `logic`. "Applied" in the product sense
means the action is on screen: pending at the front, or active and beaming. The state is
about what is happening, not a flag set at commit time, so a fast-forwarded commit that is
applied in a burst by `max_file_lag` (`user.rs` activation loop) still shows its gear while
the beams play, and a commit still queued shows nothing. A user deleted after 10 s idle
(`user.rs:61`, `sim.rs:1435-1475`) takes the state with it; aggregates live outside the user
(AI-32).

**AI-32.** Aggregates. `Simulation` keeps `contributors: BTreeMap<StrId, ContribStats>`
(BTree for deterministic iteration), keyed by the log's user string id, updated in
`process_commit` (`sim.rs:1179`) once per commit that reaches it (so after
`commit_is_valid`, `sim.rs:1124`). `reset` (`sim.rs:797`) clears it, and `seek_to`
(`sim.rs:777-795`, which calls `reset` and then sets the cursor) re-seeds it over
`log[0..cursor)`: one pass that reads only the user id, the set id and
`commit_is_valid_cached` per commit (O(commits) once per seek, nothing is simulated). The
numbers therefore describe the history up to the playback position and equal a straight
playthrough to the same cursor (hover, key and `ai_stats().played` alike; the same seeding
applies to `played_set_counts`). `restart` has cursor 0 and seeds nothing. Milestones: M2 implements only the
`played_set_counts` part of this seeding (one pass over `log[0..cursor)`, needed by the
agent key); `contributors`, `ContribStats`, the hover percentages, `habitual` and the
leaderboard are M3 (Q16):

```rust
struct ContribStats {
    commits: u32,           // commits played
    assisted: u32,          // commits whose set has an Assist entry
    authored_by_agent: u32, // commits whose set has an Agent entry
    bot: u32,               // commits whose set has a Bot entry
    top: [(StrId /*agent*/, Option<StrId> /*model*/, u32); 4], // space-saving tally of Assist entries
}
// habitual (AI-73): commits >= 5 && 2 * assisted >= commits; its agent is top[0]
```

`RUser` carries its log string id (`log_id: StrId`, set in `add_user`, `sim.rs:1300`) so
hover and key code look up stats without string compares. Hover copy comes from section
4.2: share = `assisted / commits` rounded half up; the dominant entry is the largest `top`
count (ties: lower registry index, then model id); "and others" appears when more than one
agent is tallied.

**AI-33.** `Simulation::attr_stats(&EventLog) -> AttrStats` (not per frame):
`loaded` totals come from `EventLog::attr_commit_counts` and the sets (whole history),
`played` totals from a `played_set_counts: Vec<u32>` indexed by set id (reset and
re-seeded as in AI-32; it only grows when a new set id is first played), `contributors` (M3) is
the top 10 by `assisted + authored_by_agent` (minimum 3 commits) and is empty unless
`ai-leaderboard` is on (AI-76). Per-agent `models` lists are capped at 8 plus an `other`
bucket and agents at 16, sorted by count descending then id, so a hostile log with
thousands of distinct models gives a bounded response (AI-63). Exposed to the page as
`GourceApp.ai_stats(): string` (AI-63), not folded into `status()` because it carries lists
and is read at about 2 Hz.

**AI-34.** `ai-filter` (`all`, `only-assisted`, `exclude-assisted`; restart option).
`Filters::commit_is_valid` (`filters.rs:140-149`) and `commit_is_valid_cached`
(`filters.rs:155-165`) gain a third test on the commit's set: `only-assisted` requires an
Assist or Agent entry; `exclude-assisted` rejects commits that have one. Bot entries count as
neither. `ai-filter` composes with `user-filter` and the file filters by AND, and
`Filters::sync` (`filters.rs:113`) invalidates nothing because the verdict reads the
commit, not a cached string.

**AI-35.** Determinism. Nothing in AI-30..34 reads wall time or a random source. The only
keyed value is a per-user flutter base phase `keyed_u64(seed, stream_key(b"deco.phase"),
log_id)` mapped to `[0,1)`, computed in `RUser::new` with the helpers that are `pub(crate)`
in `hash.rs:106-132`; it does not follow `hash-seed`, so a recolour does not reshuffle it.
The flutter clock is simulation time, not wall time: extraction writes
`UserDecor.phase = fract(phase0 + 1.5 * elapsed)` computed in `f64` from `pawn.elapsed`
(crate-private, `pawn.rs:34`, advanced by the scaled `dt`), so the builder needs no clock
and `?fixed-dt` recordings are reproducible (AI-56, AI-72).

**AI-36.** Extraction does not receive the `EventLog` (`sim.rs:590-593` discards it;
`extract(&mut SceneView)` stays as is). Instead `process_commit`, which has the log, interns
a `SetInfo` the first time a set id is played (and during the seek seeding of AI-32):
`Simulation.attr_infos: Vec<SetInfo>` indexed by set id, with owned display data (agent
display name, model text, tint of the first AI entry, kinds) plus
`attr_names: BTreeMap<StrId, Box<str>>` for the strings that `ContribStats.top` refers to.
Hover text, the key rows and the resolver (selector tests, `ai:id/glob` matching, `agent`
tint) read only these tables. This is a contract change (section 5.14). Steady-state
frames allocate nothing: `attr_infos`, `attr_names`, `played_set_counts` and
`contributors` grow only when a new set id or contributor first appears (warm-up); the hover
line is written with `fmt::Write` straight into `view.text` (`view.rs`
`SceneView::push_text` uses the same `String`). `crates/gource-core/tests/alloc.rs` gets an
AI scenario.

### 5.8 Style system (`gource-core/src/style.rs`)

A style is a function from a contributor's current state to a set of decorations. The user
writes it as a list of rules; the resolver evaluates them in order.

**AI-37.** Option `user-style`: a multi-value option (one rule per line, repeated URL
parameters; `Ctx::lines`, `options.rs:232-238`, which drops only empty lines). The style
parser ignores lines starting `#`. The list replaces the previous one. Live (no restart).
`option_value("user-style")` returns the accepted rule lines, trimmed, comments and blank
lines dropped, joined by `\n`, so apply, read back, apply again is the identity (tested
with a non-empty list, section 8.1). A JSON profile export (M3) therefore does not keep
comments. `user-style` is name-bearing: its rules (`name:` terms from M3, or rules a viewer
copied from a private repository) can hold contributor names, and the option is stored in
`localStorage` and the URL. The constant `NAME_BEARING_OPTIONS = [user-filter,
user-show-filter, file-filter, file-show-filter, highlight-user, follow-user, title,
user-style]` in `gource-core/src/options.rs` (exported to the page) lists the options that
routinely hold contributor or file names; F2 (PR-91, section 4.6) and F3 (REC-71) cite it.
The list is final; the first feature that needs the constant adds it.

**AI-38.** Rule grammar:

```
rule      = selector *SP "=>" *SP props
selector  = term *( *SP "&" *SP term )                     ; 1..4 terms, AND
term      = [ "!" ] ( "human" / "agent" / "assisted"
                    / "habitual"                            ; AI-73
                    / "bot" [ ":" bot-id ]
                    / "ai" [ ":" agent-id [ "/" model-glob ] ]
                    / "name:" "/" pattern "/" [ "i" ]
                    / "id:" opaque )                        ; "id:" reserved (AI-50)
props     = prop *( *SP ";" *SP prop ) [ ";" ]
prop      = name [ "=" value ]
```

The rule line is split at its last `=>`, so a `name:` pattern may contain `=` and `>`;
property values never contain `=>`. The selector is split on `&` only outside
`name:/.../`: a pattern starts after `name:/`, ends at the first `/` that is not escaped
as `\/`, and may be followed by one `i`; so `name:/a\/b/` and `name:/[a&b]/` are single
terms. `model-glob` supports `*` and `?`, case-insensitive,
matched by a small iterative matcher (no regex), at most 64 characters.

Milestones: M2 accepts the terms `human`, `agent`, `assisted`, `ai`, `ai:agent-id` and the
negation `!`; `bot`, `habitual`, `name:/…/`, `ai:id/glob` and `id:` are M3 (`id:` stays
reserved, AI-50) and are rejected with `style_bad_selector` until then.

**AI-39.** Selector semantics. Let S be the contributor's current attribution set (AI-31;
empty when `attr_set == 0`).

| Term | True when |
|---|---|
| `human` | S has no Agent and no Bot entry (assisted humans are still human) |
| `agent` | S has an Agent entry |
| `assisted` | S has an Assist entry |
| `bot` / `bot:id` | S has a Bot entry (with that id) |
| `ai` | S has an Assist or Agent entry |
| `ai:id` / `ai:id/glob` | S has an Assist or Agent entry for that agent (and, with a glob, a model that matches; an entry without model matches only `*`) |
| `habitual` | the contributor is habitually assisted (AI-73); independent of S |
| `name:/re/[i]` | the first 128 bytes of the contributor name match the regular expression (unanchored search, `regex-lite`) |
| `!term` | negation |

Name matches (M3) are kept as a bitmask on the user and computed lazily (AI-45): a contributor
is queued when created and when the rules change, and a deterministic number of
contributors is evaluated per simulation step, inside `Simulation::update`; until
evaluated, its `name:` terms read false. Attribution terms are cheap set tests. All selectors but `name:` and `habitual`
therefore change as the state changes, which is what makes "while it is happening" work.

**AI-40.** Properties:

| Property | Values | Effect |
|---|---|---|
| `cape`, `halo`, `shades`, `hat`, `badge`, `glow` | `on`, `off`, `agent`, `hash`, `RRGGBB` | enables a decoration and sets its tint. `agent` is the agent tint of the first matching entry (registry or hashed id); `hash` is the contributor's colour; `on` is `agent` when S has an AI entry and `hash` otherwise; for `shades`, `on` is a fixed dark lens `1A1A24` |
| `colour` | `hash` (default), `agent`, `RRGGBB` | avatar tint, mixed with white 60/40 and dimmed 0.9 like `assign_user_image` (`user.rs:134-139`) so avatars keep Gource's softness |
| `scale` | 0.5 to 2 | multiplies decoration quad sizes about the avatar centre (the avatar itself follows `user-scale`) |
| `none` | flag | shorthand for every decoration `off` |

A bare property (`badge`) means `on`. Unknown names and bad values are errors (AI-46).
Milestones: M2 accepts `on`, `off`, `agent` and `RRGGBB` for the six decoration kinds and
`none`; the value `hash` and the properties `colour` and `scale` are M3.

**AI-41.** Cascade, not first match. Rules apply in order; for each property the last rule
that sets it wins; `off` clears an earlier `on`. Reason: a viewer adds `ai:claude =>
cape=3366CC` without copying the built-in rules, and a later claimed or viewer layer can
override one property without restating the others. Specificity is deliberately not
computed (order is the only precedence, which a user can read off the list). Layer order is
built-in, then claimed (reserved), then viewer (AI-49).

**AI-42.** Built-in rules, version 1 (`BUILTIN_RULES_V1`, text identical to what the panel
shows under "Show built-in rules"):

```
ai       => badge=on
assisted => cape=agent; shades=on
agent    => halo=agent; glow=agent
```

**AI-43.** `ai-style` (`default`, `cape`, `shades`, `hat` or `none`; default `default`; live;
M1b): the value selects the built-in layer 0, parsed by the same resolver that M2 exposes to
viewer rules, so M2 adds only the viewer layer. `default` is `BUILTIN_RULES_V1` (AI-42);
`cape` is `ai => badge=on`, `assisted => cape=agent`, `agent => halo=agent; glow=agent`;
`shades` is the same with `shades=on` instead of the cape; `hat` is the same with
`hat=agent; glow=agent` for `assisted`; `none` removes layer 0 only, so a viewer can keep their
own rules and drop the built-ins. `hide=decorations` (a
new element of the `hide` list, appended after `root` so the existing order of
`option_value("hide")` is unchanged, `options.rs:52`, `:439-443`) is the master toggle:
no decoration is drawn, from any layer. `hide=users` also hides decorations because they
ride with the user quads. The panel's checkbox is the inverse of `hide=decorations`.

**AI-44.** Alpha provenance (the inverse polarity is M3; in M2 a rule whose state terms are all `human` or negated is evaluated as static). Each enabled decoration carries a polarity, taken from the
rule that last set it: positive when the selector contains a positive `bot`, `agent`,
`assisted` or `ai` term; inverse when it contains state terms that are all `human` or
negated; static otherwise (only `name:`, `habitual` or nothing, so `name:/^alice/ =>
halo=gold` shows steadily). A positive decoration is multiplied by `attr_env`, an inverse
one by `1 - attr_env`, a static one by 1. Consequently a hat enabled by `human => hat=on`
fades out while the assist state fades in, instead of popping. The `colour` property mixes
toward its target by the same factor (a positive rule tints toward the agent as the
envelope rises), and `scale` interpolates from 1 by it. `UserDecor` carries `dynamic` and
`inverse` masks (AI-51).

**AI-73.** `habitual` (M3). A selector term that is true when `ContribStats` (AI-32) says
`commits >= 5` and `2 * assisted >= commits`; it is static (AI-44). Its `agent` tint is
the contributor's dominant agent (`top[0]`) when no current set exists. Because the stats
are seek-invariant (AI-32), the look is the same at any playback speed: it is the
persistent signal that the current-state selectors cannot give. The resolver cache key
includes the `habitual` bit (AI-47). Tests at `time_scale` 0.1 and 20 (section 8.1).

**AI-45.** Limits: at most 32 rules, 256 bytes per line, 4 terms per selector, 8 `name:`
terms in total, patterns at most 128 bytes, at most 64 lines including comments
(`style_too_many_lines`). Regexes compile with `regex_lite::RegexBuilder`
`size_limit(8 KiB)` and `nest_limit(16)`
(https://docs.rs/regex-lite/0.1.9/regex_lite/struct.RegexBuilder.html), which bound the
compiled program and therefore the cost of a single scan; regex-lite matches without
backtracking. A scan covers only the first 128 bytes of a contributor name (cut at a
character boundary). Evaluation (M3) is lazy and deterministic: it runs inside `Simulation::update`, once per
tick, never in `extract` (so F3's planner and recorder, which tick differently from the live
view, stay equal to each other); at most 256 contributors are evaluated (all their `name:`
terms) per simulation step, counted, never timed, so frames stay reproducible. The worst case per frame is `256 * 8` scans of at most 128 bytes
against a program of at most 8 KiB, so a hostile pattern in a shared link cannot freeze
a tab on a repository with tens of thousands of contributors. Benchmark acceptance test
(section 8.1): 8 worst-case patterns against 100,000 synthetic names finish, and no single
step exceeds 2 ms (native release). Everything else is integer work.

**AI-46.** Errors. `Settings::apply_option("user-style", ...)` returns
`OptionError::InvalidValue { option, value, reason }` (`settings.rs:52`, same shape as
`regex_lines`, `options.rs:241-253`) with `value` the offending line and `reason` equal to
`"<code>: <message> (line N)"`, `N` 1-based over all lines. Codes are the table in 4.4. On
error nothing is applied and the previous rules remain (the same behaviour as every option).
The echoed `value` is capped at 80 bytes (cut at a character boundary, `...` appended). Error
convention (README, shared with F3): server HTTP error codes are unprefixed snake_case;
client-side and wasm codes carry a feature prefix (`style_`, `profile_`, `record_`); every
wasm error is a `JsError` whose message is `<code>: <text>`, parsed by the one helper
`parseWasmError(e) -> {code, text}` in `web/src/errors.ts`, used by F1 and F3.
`GourceApp.check_style(text: string): string` runs the same `style::parse_rules` without
applying and returns JSON, `{"ok":true}` or
`{"ok":false,"code":"style_bad_property","line":2,"message":"..."}`; it exists only to
carry the line number and to validate live without applying, and its `code` equals the code
`parseWasmError` gives for `set_option` on the same input. The panel calls it on input and
shows `Line N: code: message`. `set_option` still throws on the same input (it is the
authority; `check_style` is a convenience over the same parser).

**AI-47.** Resolver and caching. `Simulation::new` compiles the layers from `Settings`
(`user_style`, `ai_style`); `set_option` arms for `user-style`, `ai-style` and `hide`
(`sim.rs:456-505`) rebuild them and bump `style_epoch`. Each `RUser` caches
`(style_epoch, attr_set, name_mask, habitual) -> ResolvedStyle`; extraction compares four
integers per user and re-resolves only on a difference. `ResolvedStyle` is a small `Copy` struct
(per-kind tint mode and dynamic bit, colour mode, scale). `hash` tints are resolved at
extraction from the user's current colour, so `hash-seed` changes apply without a rebuild.

**AI-48.** (M3) Style profile (serialisation for saved, shared and claimed styles). A JSON
envelope around the same rule lines, so there is one grammar and one parser:

```json
{
  "format": "gource-style-profile",
  "version": 1,
  "name": "Glowing hats",
  "registry": 1,
  "rules": [ "assisted => cape=off; shades=off; hat=agent; glow=agent" ],
  "options": { "ai-style": "default", "agent-key": "true" }
}
```

Rules: `rules` entries are rule lines (comments are accepted on import and dropped, AI-37); `options` is limited to
`ai-style`, `agent-key`, `ai-filter` (hide is not importable, so an import cannot hide the
UI); unknown fields are ignored; `version` greater than 1 is rejected
(`profile_version_unsupported`); a missing `format` or `rules` is `profile_bad_format`;
`registry` is informational (an older registry than the page's is fine, ids are open).
The TypeScript module `web/src/style-profile.ts` reads and writes the envelope (it needs
only `JSON.parse`); validation of `rules` is `set_option` on the engine, so the grammar is
never duplicated. A Rust server stores the envelope as opaque text and validates `rules`
with `gource-core::style::parse_rules`, which has no dependencies beyond `regex-lite`.

**AI-49.** Precedence and ownership, for D4. Effective rules are `built-in (version-pinned) ++
claimed ++ viewer`, evaluated by the cascade, so viewer override beats contributor claim
beats built-in. A claimed profile belongs to one contributor: its rules are evaluated only for
that contributor's key, and `name:` and `scale` above 1.5 are rejected in that layer. F1
ships the layer order, the profile data model (AI-48) and the reserved wire key; it ships no
code seam and no storage. The later spec "claimed styles" (F4) owns storage, the claim API,
the UI and the code seam, whose contract is fixed here so that no format changes:
`Simulation::set_claimed_styles(&mut self, claims: &[(ClaimKey, &str)])` with
`struct ClaimKey { user: StrId, cid: Option<Cid> }` (`Cid` is the 16 ASCII characters of
AI-50; the rule text is in the profile grammar), and
`Simulation::claims_snapshot() -> Vec<(StrId, String)>`, which returns the active claimed
rules as profile-grammar text; F3 embeds it in its recipe field `claims` (F3 REC-73) so the
fork applies the claims from the recipe and never from the viewer's account. A viewer who is
signed in (F2 account) and claims a contributor identity proves ownership by the numeric id
(decision, recorded as Q15): the claim check is `cid(repo, account.provider_user_id) == cid
in the log` (AI-50). The viewer switch `claimed-styles` (boolean, default true, one click
off, live, mirrored in the URL; AI-72 lists it in the snapshot) is registered by F4 together
with the first claimed profile, never later, so a viewer is never forced to see other
people's styling. Claimed looks never apply to private repositories unless the repository
owner opts in (F2 PR-109).

**AI-50.** The contributor pseudonym `cid`, decided: repo-scoped HMAC. During the git parse
step, in memory only, when the commit author's identity is
`<digits>+<login>@users.noreply.github.com`, the server computes
`cid = base64url(HMAC-SHA256(purpose key "gource/cid", host "/" repo_id "/" numeric_id))[0..16]`
and writes only `cid=` into the meta field (AI-16). It is emitted for public repositories
only, never for private ones (F2 PR-109), and by F4 once that spec exists (it needs the
repository's numeric id from the GitHub API and a server-side key: F2's `gource/cid`
purpose key when a keyring exists, otherwise a key file). No email is stored or shipped, and
`cid` is repo-scoped, so it cannot link a person across repositories. Contributors whose
commits use other emails cannot claim until F2 Q15's opt-in verified-email permission
exists. The selector `id:<cid>` stays reserved and is rejected with `style_reserved`, and
readers ignore an unknown `cid` value (no behaviour in F1 depends on it).

### 5.9 Render contract

**AI-51.** `SceneView` extension (`crates/gource-core/src/view.rs:154-171`). `UserView`
(Copy, Default) gains `decor: UserDecor`; the `Default` value draws nothing, so every
existing literal (`gource-render/tests/common/mod.rs:85`, `builder/tests.rs:77`,
`gource-cli/src/hash.rs:298`) keeps compiling through `..UserView::default()`.

```rust
#[derive(Copy, Clone, Default, PartialEq)]
pub struct UserDecor {
    pub mask: u8,           // bit per DecoKind (Cape, Halo, Shades, Hat, Badge, Glow); 0 = none
    pub dynamic: u8,        // bits multiplied by `envelope` (positive polarity, AI-44)
    pub inverse: u8,        // bits multiplied by `1 - envelope` (inverse polarity)
    pub badge: u8,          // BadgeShape index
    pub tint: [[u8; 3]; 6], // per kind
    pub envelope: f32,      // attr_env
    pub scale: f32,         // decoration scale (1 = default)
    pub vel: Vec2,          // pawn.accel at extraction, world units per second
    pub phase: f32,         // flutter phase in [0,1): fract(phase0 + 1.5 * elapsed), AI-35
    pub motion: f32,        // 1, or 0 with decor-motion=off (zeroes lean, lift and flutter)
}
```

`extract_users` (`sim/extract.rs:319-351`) fills it from the resolved style, `attr_env`,
`pawn.accel` (`pawn.rs:26-53`; `accel` is velocity after the speed cap and friction,
`user.rs:427-434`) and the phase (AI-35). The view carries no clock; every input the
builder uses is in `UserDecor`. `UserView.colour` already carries the `colour` property
override, so the renderer needs no avatar-colour logic. With `hide-users` nothing is
extracted (as today); with `hide=decorations`, `UserDecor` stays default.

**AI-52.** Draw order and contiguity. `frame.quads` is built in three consecutive blocks
from the same `view.users` order (alphabetical by name, `sim.rs:247`), in
`FrameBuilder::scene` (`gource-render/src/builder.rs:290-303`, block `user_vbo`):

1. **back block**: for each user with Cape or Halo, the cape quad then the halo quad;
2. **avatar block**: the existing user quads, unchanged and contiguous (their `Range` is
   what the shadow and textured commands already use);
3. **front block**: for each user with Shades, Hat or Badge, in that order.

Commands, in order (new ones marked):

| # | Command | Stream | Program | Texture | Blend |
|---|---|---|---|---|---|
| 1-5 | edge shadow, edges, file shadows, action beams, files (unchanged) | | | | |
| 6 | user shadows over the avatar block (unchanged) | Quads | Shadow 0.5 | User | Alpha |
| 7 | **back block** (new) | Quads | Textured | Deco | Alpha |
| 8 | avatars (unchanged) | Quads | Textured | User | Alpha |
| 9 | **front block** (new) | Quads | Textured | Deco | Alpha |
| 10 | bloom plus user glow (one range, state unchanged) | Bloom | Bloom | White | Additive |
| 11-12 | text, two commands (unchanged) | Text | Text | Glyphs | Alpha |

Commands 7 and 9 are skipped when empty by `push_command` (`emit.rs:96-108`); they cannot
merge with each other because the avatar block lies between them. Consequence: capes of all
users sit under all avatars and front gear over all avatars, instead of per-user
interleaving; overlapping users are 100 units apart by design (`PERSONAL_SPACE_DIST`,
`user.rs:50`), so the difference is rarely visible, and it keeps the quad ranges contiguous
(per-user interleaving would cost about two commands per user).

**AI-53.** Draw-call budget: 10 today (`docs/web-mvp/PERFORMANCE.md:178`); decorations add at
most 2 (back and front blocks) for a worst case of 12; glow adds none (AI-58); the agent key
adds none (text stream only, AI-59); F3 adds none either (F3 REC-116). With no decorations the
command list is identical to today's.

**AI-54.** Atlas. New `TextureId::Deco` (`gource-render/src/lib.rs:58`) and a texture
`data/decorations.png`, 256x256 RGBA (256 KiB raw), cells 64x64 with art inside 56x56 and a
4 px gutter whose RGB is the extruded edge colour at alpha 0. `build.rs` decodes each PNG at
build time and `include_bytes!`s the raw RGBA into the wasm
(`gource-render-webgl2/build.rs:14,33-43`), so the atlas costs its raw size in the wasm
binary (for scale, `user.png` is 786 KB raw); that is why the atlas is small and why section
7 carries a wasm size row. The Deco texture sets `TEXTURE_MAX_LEVEL = 2` (the existing
trilinear chain, `textures.rs:96-118`, would otherwise bleed neighbours through the
gutter at mip 4 and above, the far-zoom case); beyond that a decoration is under 16 px wide
and minifies from mip 2. Artwork is neutral greyscale with alpha so the vertex colour is the
tint (the `Textured` program is `texture * colour`). Every badge shape is drawn at 0.8 of its cell inside a
dark rim (grey 24, 0.085 of the cell wide): dark texels stay dark after tinting, so a badge
keeps its contrast on a cape or avatar of its own colour. Contents: cape, halo, shades, hat, and
ten badge shapes (circle, diamond, hexagon, triangle, square, star, ring, plus, drop, bolt),
14 of the 16 cells. M1a's atlas draws cape, halo, shades, hat and one badge shape (circle);
the other nine badge shapes are drawn into their reserved cells in M2.
The art is original, generated by `scripts/deco/atlas.py` (polygons and gradients only,
Pillow), which also prints the rectangle table checked in as
`crates/gource-render/src/deco_atlas.rs`. Wiring: one `TEXTURES` entry
(`crates/gource-render-webgl2/build.rs:14`), a `Textures.deco` field
(`textures.rs:30-36`) and one match arm (`backend.rs:306-312`); `uv_scale` stays `[1,1]`
(`backend.rs:264-268`). No brand marks (N3).

**AI-55.** Anchors, as `(u, v)` rectangles of the avatar quad (`u` right, `v` down, avatar
art is 384x512, head centre about (0.49, 0.31) with radius 0.28 of the width, shoulders at
v of about 0.50, body from v 0.49 to 0.85; measured from `data/user.png`). Sizes scale by
`UserDecor.scale` about the avatar centre.

| Kind | Rect (u0,v0)-(u1,v1) | Notes |
|---|---|---|
| cape | (-0.16,0.43)-(1.16,1.00) | top edge fixed at the shoulders; the art's shoulders sit at u 0.135 to 0.865 (outside the torso, which is u 0.32 to 0.68 there) and flare to hem corners at u -0.14 and 1.14, v 0.975 (the avatar's base is v 0.86); bottom edge sheared by AI-56 |
| halo | (0.24,-0.04)-(0.74,0.10) | above the head, behind it |
| shades | (0.27,0.24)-(0.71,0.36) | over the eyes |
| hat | (0.20,-0.14)-(0.78,0.17) | sits on the head |
| badge | (0.70,0.68)-(0.92,0.84) | lower right of the body, shape from `UserDecor.badge` |
| glow | circle, radius 3 x avatar width, centre (0.5, 0.5) | additive, AI-58 |

Values are constants in `deco.rs` and can be tuned against the golden screenshot; the
requirement is the table's structure, not the digits.

**AI-56.** Cape motion. The cape is a quad with explicit corners (new `quad4` helper in
`emit.rs` next to `quad`, `emit.rs:27`): top corners fixed; bottom corners move by
`sx = clamp(-vel.x / 250, -1, 1) * 0.6 * W` horizontally and are lifted by
`min(1, |vel| / 250) * 0.2 * H` vertically (the cape streams backward when fast), plus a
flutter `0.04 * W * sin(2*pi*phase)` where `phase` is `UserDecor.phase` (advanced by the
simulation, AI-35), using `libm::sinf` (`gource-render` already depends on `libm`). At rest
the hem also billows: the two bottom corners spread apart by
`b = 0.03 * W * (1 - min(1, |vel| / 250)) * (1 - cos(2*pi*phase)) / 2` each (right corner
`+b`, left corner `-b`), so a standing cape sways and breathes instead of hanging still,
and the billow gives way to the lean as the user speeds up. Lean, lift, flutter and billow
are all multiplied by `UserDecor.motion`. While the cape fades (its AI-57 factor `f`, the
envelope or its inverse, not `user.alpha`), each bottom corner is pulled toward the top
corner on its side to `0.35 + 0.65 * f` of its distance, so a fading cape folds up toward
the shoulders as it dims instead of lingering as a dark wedge. `W` and `H` are the avatar's
drawn width and height. All inputs come from the view, so frames stay deterministic.

**AI-57.** Alpha. A decoration quad's alpha is `user.alpha` times `envelope` for dynamic
kinds and `1 - envelope` for inverse kinds (AI-44), so fade-in at creation and the 1 s fade-out of an idle user
(`user.rs:232-239`) apply to decorations unchanged.

**AI-58.** Glow uses the bloom path, not a new program: one `BloomVertex` quad per user with
the Glow bit, appended to the same `frame.bloom` range after the directory bloom quads
(`builder.rs:305-329`), so the Bloom command (state unchanged, contiguous) covers both.
Colour is `tint * 0.85 * user.alpha * envelope` (the vertex shader clamps colour to 1,
`shaders.rs:20-34`, and the shader's profile peaks at half the colour at the centre),
radius `3 * W * scale`, texcoord
packed as for directories (`[radius, centre.x, centre.y, 0]`, `builder.rs:316-323`). The
glow is drawn after the avatars, so it also tints the avatar; the wide radius flattens the
profile (`cos(2d / radius)`), which puts a visible aura around the figure at default zoom,
next to bright directory blooms, while the centre gains at most `0.43 * tint`. User
glow is emitted even when `hide=bloom` is set (`hide=decorations` controls it). The existing
bloom shader noise term depends on `sin` and differs per GPU
(`docs/web-mvp/PARITY.md` summary); glow inherits that and the golden screenshot tolerates
it.

**AI-59.** Agent key. `HudView` (`view.rs:262-276`) gains `agents: Vec<AgentKeyRow>`,
`show_agent_key: bool` and `filter_note: Option<TextRange>` (AI-74); a row is `{ name: TextRange, colour: Vec3, count: u32 }`. The
simulation rebuilds the rows only when `played_set_counts` changed (a dirty flag set in
`process_commit`), so steady state costs nothing. The renderer lays rows out statically
(right-aligned at the top right, row height `scaled_font_size + 6`, as `FileKey`,
`key.rs:240-300`) with the first row at a fixed y = 68 logical pixels (`KEY_TOP` in
`agent_key.rs`). Placement: the web page's DOM toolbar (`.viewer-tools`) covers the top
right of the live view from y 12 to 56 CSS pixels, and logical pixels are CSS pixels there
(`ui_scale` is the device pixel ratio), so the key starts 12 px below it; in recordings
there is no toolbar and the same layout leaves a short gap under the date, which keeps
recordings and the live view identical. The top right is kept rather than the top left
because Gource's file-extension key fills the left edge from the top and the title sits
bottom left. The renderer emits text only: the bullet `U+25CF` as its own run in the agent
colour, then name and count in the font colour, all into the existing text stream, so it
adds no quads and no draw command (a swatch quad would add a HUD quad command whenever the
file key is off, `builder.rs:149`, `:589-608`). No animation. The bullet glyph must exist
in the glyph source (canvas 2D falls back across fonts; `data/fonts/FreeSans.ttf` is not
guaranteed to contain it): M2 verifies it in the browser and falls back to `*`.

**AI-60.** Hover text. `extract_hud` (`extract.rs:409-470`) appends the AI line (4.2) to
`textbox_lines` for a hovered user, writing it into `view.text` with `fmt::Write` from the
`SetInfo` and `attr_names` tables (AI-36), and only when `ai-person-stats` is on; the
second line uses the user's colour like the first.

**AI-61.** Accessibility, with what is and is not claimed.
(a) Colour: seven of the ten tints are Okabe-Ito values; `A78BFA`, `8BC34A` and `C0C0C0`
are not, and `E69F00`, `D55E00` and `F0E442` are close under deuteranopia. So tint is never
the only channel: every agent has a distinct badge shape, and the hover line, the key and
the panel name it in text. A unit test simulates protanopia, deuteranopia and tritanopia
(Machado matrices) over the ten tints and fails for any pair closer than CIE76 DeltaE 15
that is not on a documented allow-list, each entry of which must have different badge
shapes (Q12 lists the known pair).
(b) Size: at default zoom a badge is only a few pixels (AI-55 rects), so the shape is not a
reliable cue there; text is the reliable channel. The builder draws a badge at least
8 px x `ui_scale` tall (device pixels, capped at 0.4 of the avatar height) so it stays
visible when zoomed out and a 4K recording (F3, `ui_scale` 2) is the 1080p view, sharper; a
render test at `ui_scale` 1 and 2 asserts the ratio of badge to avatar is equal.
(c) Motion: flutter and lean are small and fixed, and `decor-motion=off` removes them
(AI-77); the page sets that option once from `prefers-reduced-motion` when the URL does not
give it, which keeps recordings deterministic because the option is part of the snapshot.
(d) Keyboard and screen readers: hover is mouse-only and the key is canvas text, so the
panel section is the accessible route; it lists every agent seen, with counts, as real text
(e2e check, section 8.3).

**AI-62.** Hash and goldens. The function `hash_view` (today `crates/gource-cli/src/hash.rs:5-40`,
moved to `crates/gource-core/src/view_hash.rs` by F3 REC-82; whichever spec lands first, the
other edits the new location) reads view fields by name, so `decor` is invisible to existing hashes. Encoding stays version 1
byte for byte when every user's `decor.mask == 0`; otherwise a `DECO` section (tag, count,
then for each user with a non-zero mask: index, mask, dynamic, inverse, badge, tints,
envelope bits, scale bits, velocity bits, phase bits, motion bits) is appended. Existing golden tables
(`crates/gource-core/tests/golden/*.tsv`) therefore stay valid.

**AI-74.** Visible filters. While `ai-filter` is not `all`, extraction writes
`HudView.filter_note` (`AI commits: only assisted` or `AI commits: excluding assisted`)
and the renderer draws it as one text line under the date, in the text stream (no quads, no
command), so screenshots and recordings carry it. When `user-style` or `ai-filter` are set
by URL parameters at load, the page shows a dismissible notice (AI-64). The HUD line is
not dismissible. It is drawn regardless of `hide=date`: with `hide=date` it takes the date
line's position, and the agent key keeps its top-right position (anchored to the row the date
would occupy), so a screenshot or recording always carries the filter notice. With `ai-filter=all` nothing is drawn and frames are unchanged.

### 5.10 Options

All through `Settings::apply_option` / `option_value` and the round-trip test
(`options.rs` test "every option value applies back unchanged", noted at `options.rs:904`).

| Option | Type and values | Default | Effect | Notes |
|---|---|---|---|---|
| `user-style` | multi-value rules | empty | Live | AI-37 to AI-46 |
| `ai-style` | `default` / `cape` / `shades` / `hat` / `none` | `default` | Live | AI-43; M1b |
| `ai-filter` | `all` / `only-assisted` / `exclude-assisted` | `all` | Restart | `effect_of` (`options.rs:171-176`) and `RESTART_OPTIONS` (`web/src/options.ts:157`) |
| `agent-key` | boolean | false | Live | AI-59 |
| `ai-person-stats` | boolean | true | Live | AI-76; false: no hover line, empty `contributors` |
| `ai-leaderboard` | boolean | false | Live | AI-76; the panel's contributor list |
| `decor-motion` | `full` / `off` | `full` | Live | AI-77; the page sets `off` once from `prefers-reduced-motion` |
| `claimed-styles` | boolean | true | Live | AI-49; registered by the later spec "claimed styles" (F4), listed here so the URL and snapshot grammar is fixed; mirrored in the URL and listed in the snapshot (AI-72) |
| `hide` | gains element `decorations`; alias `hide-decorations` | not hidden | Live | `HIDE_FIELDS` becomes 12 entries, `options.rs:52`; `HIDE_ELEMENTS`, `web/src/options.ts:75-87` |

`OPTION_NAMES` (`options.rs:67-148`) gets the names of each milestone (seven by M2; `claimed-styles` with F4). The Rust option table and
`web/src/options.ts` are mirrored by hand today (`RESTART_OPTIONS` comment, PLAN.md section
4); the unit test that compares them gets the new names.

**AI-76.** (M3, Q16) Per-person numbers. Hover percentages and the contributor list profile named
individuals from claims that anyone can forge or strip, and in private repositories (F2)
they are workplace data. So: `ai-person-stats` (default on) is the viewer's one switch to
show only aggregates; the contributor list is off unless `ai-leaderboard` is on and is never
drawn into the canvas (so never into a recording); copy says "credited to" and carries the
caveat of 4.2 and 4.4. For a source with `source.private` (F2 PR-93) the page sets
`ai-person-stats=false` once through `set_option` when neither the URL nor stored settings give
a value (the same mechanism as `decor-motion`, AI-77), so workplace data defaults to
aggregates.

**AI-77.** `decor-motion`: `full` or `off`. `off` sets `UserDecor.motion = 0`, which freezes
cape lean, lift and flutter; it never reads the OS. The TypeScript layer sets it once from
`matchMedia('(prefers-reduced-motion: reduce)')` when neither the URL nor stored settings
give it, through `set_option`, so it is mirrored in the URL and travels with the snapshot.

### 5.11 Browser API and TypeScript

**AI-63.** `GourceApp.ai_stats(): string` (JSON, hand-written like
`crates/gource-web/src/status.rs:1-110`). Schema:

```json
{
  "registry": 1,
  "loaded": { "commits": 1480, "assisted": 412, "agent": 57, "bot": 30, "sets": 41,
              "agents": [ { "id": "claude", "name": "Claude", "colour": "E69F00",
                            "commits": 400, "as_author": 0,
                            "models": [ { "model": "Claude Opus 5.5", "commits": 380 } ] } ] },
  "played": { "commits": 640, "assisted": 190, "agent": 20, "bot": 11, "agents": [ ] },
  "contributors": [ { "name": "jane", "commits": 142, "assisted": 33, "agent": 0 } ]
}
```

All names pass through the existing JSON string escaper (`status.rs` `push_json_string`).
Bounds (AI-33): at most 16 agents, at most 8 models per agent plus an `other` bucket,
`contributors` at most 10 and empty unless `ai-leaderboard`; order is count descending then
id. `LogParser.meta_ignored(): number` is added (AI-27), and `GourceApp.check_style` (AI-46).

**AI-64.** TypeScript: `web/src/options.ts` (section `style`, seven option defs plus the
`decorations` entry of `hide`, `HIDE_ELEMENTS`
adds `decorations`, `RESTART_OPTIONS` adds `ai-filter`); `ui/panel.ts` gets a `rules` control
kind (textarea, preset select, inline error line from `check_style` (AI-46); errors thrown by
`set_option` (`wasm.rs:166`) are read with the shared `parseWasmError`); `style-profile.ts` (M3: export as a downloaded JSON file, import
from a file input or pasted text); `ui/ai-panel.ts` (reads `ai_stats()` every 500 ms while
the section is open; DOM built with `textContent` only). `url-state.ts` needs no change for
the list option; when the serialised URL exceeds `SHARE_URL_SOFT_LIMIT` (2,000 characters, README; F2 PR-10 reduces
a longer `return` to the source parameters at sign-in) the panel shows
"This style is long; use Export to share it." (the link itself stays valid; Export is M3). The last `user-style` and the AI options are
stored in `localStorage` (wrapped in try/catch; the page renders correctly without it) and
restored on any repository, with URL parameters overriding them (US-15). The dismissible
notices of AI-74 and the disabled state of AI-29 live here too.

**AI-65.** Mock engine (`web/src/engine/mock.ts`, header at `mock.ts:3-5`) accepts the new
options, returns canned `ai_stats()`, and does a minimal `=>` check on `user-style`; the full
error table is covered against the real engine (section 8).

### 5.12 Parity statement

**AI-66.** With no attributions in the log and `user-style` empty: no `UserDecor` bit is
set, no back or front quads exist, avatar and bloom ranges are byte-identical, the command
list is identical, `hash_view` is identical (AI-62), `sim_golden*.tsv` and the C++ parity
renders (`docs/web-mvp/PARITY.md`) are untouched, and `gource-cli` hash runs on every
existing log agree with today's. A required test builds one log twice, with `ai=` fields and
with them stripped, and asserts that the first rendered with `hide=decorations` equals the
second, frame for frame.

### 5.13 Settings snapshot

**AI-72.** Decision D9. The settings snapshot is the URL-state options (non-default only,
including repeated `user-style` lines, `ai-style`, `ai-filter`, `agent-key`,
`ai-person-stats`, `ai-leaderboard`, `decor-motion`, `claimed-styles` and `hide=decorations`) plus
F3's recording settings. The profile envelope of AI-48 is only an import and export wrapper over
those same options, never a second serialisation, so the two cannot drift: import calls
`set_option` and export reads `option_value`. A recording shows the decorations the same options
give with the same fixed-dt tick sequence, so nothing in this feature reads wall time (AI-35),
the flutter phase is keyed and simulation-driven, `name:` rules (M3) are evaluated inside
`Simulation::update` (AI-45), and a run with `?fixed-dt` is frame-identical; the `DECO` section
of `hash_view` (AI-62) is the test that checks it. `name:` rules on logs with more than 256
contributors may start a few ticks later in a recording than live. The recording recipe
embedded by F3 (`docs/features/recording.md`) lists exactly the non-default options among
`user-style` (multi-value: lines joined by `\n` in `option_value`, repeated parameters in
the URL), `ai-style`, `ai-filter`, `agent-key`, `ai-person-stats`, `ai-leaderboard`,
`decor-motion`, `claimed-styles` and `hide=decorations`, plus its `claims` field (AI-49); F3
test T-16 asserts it. Story US-17; test in section 8.2.

### 5.14 Contract changes

**AI-75.** Everything this feature changes in an existing contract, so the implementation
plan can size it:

| Contract | Change |
|---|---|
| batch format | version 2, header 32 bytes, set table; the `GEB\x01` literals in `gource-sources/tests/batch.rs` (about lines 103, 278, 621) become `\x02` |
| `ParsedCommit`, `ParseStats` (`lib.rs:52-76`) | `attrs`, `bad_meta`; about 14 `ParsedCommit {` literal sites change |
| `EventLog`, `CommitRecord` | set table, `attrs` field; `push_commit` keeps its signature, about 26 call sites unchanged |
| `SceneView`, `UserView`, `HudView` | `UserDecor`, `agents`, `show_agent_key`, `filter_note` |
| `Simulation` | `attr_infos`, `attr_names`, `contributors`, resolver; `extract` signature unchanged (AI-36) |
| git command (`pipeline.rs:744-752`) | format of AI-12; `docs/web-mvp/PLAN.md` section 5 (line 304) to be updated |
| cache key | AI-28 |
| `set_option` surface | `user-style`, `ai-style`, `ai-filter`, `agent-key`, `ai-person-stats`, `ai-leaderboard`, `decor-motion`, `hide=decorations` |
| `LogParser`, `GourceApp` (wasm) | `meta_ignored`, `ai_stats`, `check_style`; `docs/web-mvp/PLAN.md` section 4 to be updated |
| job result and cache metadata | `ai_detect` |
| `gource-cli` | no new command; `convert --from git` and the `hash` path are rebuilt against the new `ParsedCommit`/`EventLog` types (compile-time changes only); F3 M2 later adds `record-plan` |
| `hash_view` | `DECO` section (AI-62), at the function's location when F1 lands (today `crates/gource-cli/src/hash.rs:5-40`, moved by F3 REC-82) |
| draw calls | 10 to at most 12 (AI-53) |

## 6. Security & privacy

**AI-67.** What crosses the wire (D6). Allowed: agent id, kind, model string, the `bot` id (M3)
and, for public repositories only, `cid` (AI-50: noreply-id identities only, emitted by F4).
Not allowed, anywhere (cache file, API response, SSE progress event, batch, page, server logs and tracing
output): author, committer and co-author emails, raw trailer text, commit subjects. The classifier runs inside the
server's parse step and returns ids only (AI-8); the cached `.log` therefore contains no
email that was not already absent today (`%aN` names are shown today, emails never were).
Rationale: attribution needs a tool identity, not a person's address, and an email in a
public cache file would be harvestable by anyone who can download a log. For private
repositories (F2) the same grammar applies; `cid` is never emitted (F2 PR-109). The log with
attributions is repository-private data (it shows which tools a team uses): in F2 M1 it is an
ephemeral `private/<job>/` file, from F2 M2a a cache entry (plain 0600, sealed from M2b); F1
adds nothing to that. `ai_stats()` is never sent anywhere.

**AI-68.** Everything attribution-bearing is untrusted text. Trailers and identities can be
forged by anyone who can commit, so attribution is a claim (N1; UI copy says "credited to").
Models are accepted only if they match the bounded per-agent grammar (AI-6), so a forged
trailer name cannot put free text such as "Claude is why Bob broke prod" next to a real
person's name; agent ids only from `[a-z0-9-]` or the registry; strings render through canvas text and
`textContent`, never `innerHTML`. A hostile custom log can at worst show decorations the
log author chose.

**AI-69.** Resource caps: 512-byte field, 4 credits, 64-byte model, 65,536 distinct sets
(enforced in the parsers, AI-27), 32 rules, 128-byte patterns with `size_limit(8 KiB)`
and `nest_limit`, bounded `ai_stats()` output (AI-33, AI-17, AI-25, AI-45). Memory for a
150,000-commit log with attributions is a few hundred KB beyond today's, asserted in the
allocation test (section 8.1).

**AI-70.** Regex cost. User patterns run with `regex-lite` (no backtracking), with an 8 KiB
compiled-size cap, on at most 128 bytes of name, at most 256 contributors per simulation
step (AI-45). A hostile pattern in a shared link is bounded by those limits, and the page
tells the viewer that the link applies custom styling (AI-74).

**AI-71.** Forged-attribution amplification is bounded: the classifier needs an exact
registry email or login, so a commit can only claim to be one of ten known tools, which a
trailer can do anyway, and its model text is limited to a bounded grammar (AI-6), so the
forger gains no free-text surface. Attribution is a claim and is presented as one (AI-76).

**AI-78.** Misleading presentation. A shared link can change what a viewer sees
(`ai-filter`, `hide=decorations`, `user-style` with `name:` rules). The visible filter line
and the load notices of AI-74 make that state visible in the page, in screenshots and in
recordings. Vendor names are plain text that identifies tools (N3, section 10).

## 7. Performance budget

| Item | Budget | Basis |
|---|---|---|
| Draw calls | 10 today, at most 12 with decorations, 0 added by glow and key, 0 added by F3 recording (F3 REC-116) | AI-52, AI-53; PERFORMANCE.md:178; PLAN.md:66 "about a dozen" |
| Wasm size | Budgets are cumulative against `gource_web_bg.wasm` at HEAD 8a3ecec (1,558,409 bytes raw): F1 +150 KiB gzip, F3 +60 KiB gzip (F3 REC-119), together at most +210 KiB gzip; F1's share is at most 400 KiB raw and 150 KiB gzipped by M1a (atlas 256 KiB raw, AI-54, plus code). Each milestone measures against the artifact of the previous merged milestone with `scripts/build-web.sh --release` and records the number in `docs/web-mvp/PERFORMANCE.md` | AI-54; `build.rs` embeds raw RGBA |
| Extra quads | at most 5 per user in `frame.quads`, 1 per user in `frame.bloom` | 5 x 4 x 32 B = 640 B/user; 1,000 users is 640 KB against 3.2 MiB (git/git) to 19.6 MiB (100k files) uploaded today |
| Extraction | 3 integer compares per user, resolve only on change | AI-47 |
| Per-frame allocation | none after warm-up | AI-36; extend `gource-core/tests/alloc.rs` and `gource-render/tests/no_alloc.rs` |
| Name matching (M3) | at most 256 contributors evaluated per step; no step above 2 ms with 8 worst-case patterns on 100,000 names | AI-45 |
| Seek | the stats re-seed (M2: `played_set_counts` only; M3: per-contributor stats) is O(commits) once per seek; at 150,000 commits it adds at most 5 ms (measured) | AI-32 |
| `CommitRecord` | stays 32 bytes (about 3.2 MB at 100k commits) | AI-25 |
| Batch overhead | 0 bytes for commits without attributions (one extra bit in the varint, a second byte only above 63 changes); about 15 bytes per attributed commit before gzip | AI-24 |
| Log size | field written once per attributed commit, about 35 bytes typical; at 30% of 150,000 commits about 1.6 MB on a 36 MB log (cap150k, PERFORMANCE.md), before gzip | AI-21 |
| Server history phase | the new format must add at most 25% to `git log` plus parse time on git/git, measured with `scripts/perf`; if the budget is exceeded, detection ships disabled by default and becomes a per-request opt-in (`GOURCE_AI_DETECT=0` unless set, `detect_ai` request field, AI-14) until fixed | AI-14; `%(trailers)` parses each message, so this is an unmeasured cost today |
| Frame time | decorations on 30% of 500 users: at most +0.1 ms sim, +0.1 ms build vs no decorations | gate in `crates/gource-render/tests/perf.rs` style benches |

## 8. Testing & acceptance criteria

### 8.1 Unit

- `gource-sources` classifier (AI-1 to AI-11). One test per row of this table, inputs
  copied from the real commits and vendor docs in `docs/features/research/ai-trailers.md`:

| Vector | Input (identity or trailer) | Expected |
|---|---|---|
| T1 | trailer `Co-authored-by: Claude Opus 5.5 <noreply@anthropic.com>` | Assist claude, model `Claude Opus 5.5` |
| T2 | trailers `Co-Authored-By: Claude Haiku 4.5 <noreply@anthropic.com>` and `Claude Sonnet 5` | two Assist entries, ordered by model |
| T3 | trailer `Claude <noreply@anthropic.com>`; `Claude Code <...>` | Assist claude, model None |
| T4 | author and committer `Claude <noreply@anthropic.com>` | Agent claude |
| T5 | author `claude[bot] <41898282+claude[bot]@users.noreply.github.com>` plus a human `Co-authored-by` | Agent claude (numeric id ignored); human trailer dropped |
| T6 | trailer `Codex <noreply@openai.com>` | Assist codex |
| T7 | author `chatgpt-codex-connector[bot] <199175422+chatgpt-codex-connector[bot]@users.noreply.github.com>` | Agent codex |
| T8 | author `copilot-swe-agent[bot] <198982749+Copilot@users.noreply.github.com>`, committer `GitHub <noreply@github.com>`, trailer = human | Agent copilot |
| T9 | trailers `Copilot App <223556219+Copilot@users.noreply.github.com>`; `Copilot <copilot@github.com>` | Assist copilot (one entry) |
| T10 | trailers `Cursor <cursoragent@cursor.com>` and `Made-with: Cursor` | one Assist cursor |
| T11 | author `Cursor Agent <cursoragent@cursor.com>` | Agent cursor |
| T12 | author `Jane (aider)`; trailer `aider (gpt-5.4) <aider@aider.chat>`; trailer `aider (gemini/gemini-3.1-pro-preview) <aider@aider.chat>`; subject `aider: fix` | v1: Assist aider with models `gpt-5.4` and `gemini/gemini-3.1-pro-preview` from the trailers; the contributor name stays `Jane (aider)` and the subject prefix is ignored. M3 vector T12b: suffix and prefix alone give Assist, never Agent, and the name becomes `Jane` (AI-4) |
| T13 | author `google-labs-jules[bot] <161369871+google-labs-jules[bot]@users.noreply.github.com>` | Agent jules |
| T14 | author `devin <devin@cognition.ai>`; trailer `Devin AI <158243242+devin-ai-integration[bot]@users.noreply.github.com>` | Agent devin; Assist devin |
| T15 | trailers `Amp <amp@ampcode.com>`, `openhands <openhands@all-hands.dev>` | Assist amp; Assist openhands |
| T16 | author `amazon-q-developer[bot] <208079219+amazon-q-developer[bot]@users.noreply.github.com>` | Agent amazon-q |
| T17 | authors `dependabot[bot]`, `renovate[bot]`, `github-actions[bot]` | no attribution (excluded, not AI); M3 vector T17b: Bot, not AI |
| T18 | human `Claude Smith <claude@smith.example>`; `Bob <bob@old.example>` trailer; `gemini-code-assist[bot]` | no attribution (name alone does not match; Gemini not in v1) |
| T19 | squash: the T1 trailer twice with keys `Co-Authored-By` and `co-authored-by`, plus the T3 bare form | one entry (dedupe, bare dropped) |
| T20 | author agent A and a trailer naming A | one Agent entry (AI-4) |
| T21 | 6 distinct trailers | 4 entries, `truncated` counted |
| T22 | model `Claude Opus 5.5 <script>` and a 70-byte model | model None, agent kept (AI-6) |
| T23 | trailer `Claude is why Bob broke prod <noreply@anthropic.com>` | Assist claude, model None |
| T24 | trailer `Claude see http://evil.example <noreply@anthropic.com>`; aider trailer `aider (Bob is to blame) <aider@aider.chat>` | model None for both, agents kept |
| T25 | raw author `claude[bot]`, mailmapped (`user:`) name `Jane Doe` | Assist claude (downgraded, AI-2) |
| T26 | author name containing `0x1f` (shifts the fields); author name containing `0x1e`; email piece with a space; trailer piece without `:` | no attribution, `bad_meta` counted per commit |
| T27 | trailer `Claude Fable 5 <noreply@anthropic.com>` (a family that is in no registry table) | Assist claude, model `Claude Fable 5`, no registry edit (AI-6) |

  Vectors T1 to T4, T6 to T16 are taken from real commits (References); T5, T17, T18 to T27
  are derived or synthetic and are marked as such in the test file. The registry digest
  test also sits in this crate (AI-10).
- Registry digest and id-reference tests (AI-10).
- Cache key (AI-28): `ai1`, `ai0` and another registry version give three different keys.
- Meta grammar (AI-15 to AI-18): table-driven valid and malformed inputs (every row of the
  malformed table; every escape; 512, 513 bytes; 4 and 5 credits; duplicate keys; legacy
  `ff0000|more`; a model `a:b` and a model containing `,` written escaped and read back
  unchanged; decoded C0, C1 and bidi controls rejected; the AI-23 examples including the
  literal `:` and `,` of `ai=co:claude:Claude Opus 5.5,co:codex`; writer overflow with four
  64-byte models full of `/` and `:` drops trailing credits and counts `meta_trimmed`), plus
  a chunking test (1 byte to whole file) with attributions: parsed
  commits are identical for every chunk size (the existing property in
  `gource-sources/tests/chunking.rs`).
- Union and grouping (AI-20); writer first-line-only and round trip
  parser -> writer -> parser (AI-21, AI-23).
- Batch v2 (AI-24, AI-26): encoder -> `apply_batch` equals direct `EventLogSink`; every
  rejection (bad set id, `n` out of range, version 1 batch); `size_of::<CommitRecord>() == 32`;
  a log with 65,537 distinct sets loads completely and the last commit has set `0` (AI-27).
- Git parser: a `GitRawParser` fixture of the new format including a merge header (followed
  directly by the next `user:` line, AI-12), a merge whose `m1:` credits must not reach the
  next commit even when that commit's `m1:` is missing or malformed, an agent-authored
  merge, a stale `m1:` followed by `user:`, a TAB in the subject, an empty trailer piece, a
  truncated meta line, the hostile-byte vectors T26 and an old-format file
  (AI-13); fixture produced by the command in AI-12 against a scratch repo in the test
  (skipped when `git` is older than 2.22).
- Tests of `name:`, `habitual`, inverse polarity, `scale`, `colour`, `ai:id/glob`, the Bot kind and
  profile import and export run when their milestone (M3) lands; the list below is the full set.
- Style grammar and errors (AI-37 to AI-46, AI-73): every code in 4.4 with line numbers,
  including `style_too_many_lines`; `name:/a\/b/` and `name:/[a&b]/` parse as one term;
  glob matcher cases; limits; `option_value` round trip with a non-empty list (apply a rule
  list, read back, apply again, compare); `check_style` returns the same code and line as
  `set_option`, with the echoed line capped at 80 bytes; cascade (later wins, `off`
  clears, `none`); polarity (positive, inverse, static) and the `human => hat=on`
  crossfade without a pop; `ai-style=none`; `hide=decorations`; `habitual` at `time_scale`
  0.1 and 20 gives the same decorations; the regex cost benchmark of AI-45 (8 worst-case
  patterns against 100,000 synthetic names, no step above 2 ms, the per-step count never
  above 256).
- Simulation (AI-30 to AI-36): state machine with a scripted action sequence (rise 0.25 s,
  hold 2 s, fall 0.75 s at `dt = 1/60`, with `time_scale` 2); pending-front rule; contributor
  stats and hover copy for every row of 4.2; `ai-filter` all three values over a fixture;
  determinism (two runs equal, phase independent of `hash-seed`); `restart` clears state
  and a seek to 50% gives stats equal to a straight playthrough to the same cursor (AI-32);
  `ai-person-stats=false` removes the hover line and `contributors`; allocation test in
  `tests/alloc.rs`, with a memory assertion for a 150,000-commit attributed log.
- Render (AI-51 to AI-60): synthetic `SceneView`: block order and ranges (back, avatar,
  front), command list and count (10 without, 12 with both blocks, 11 with one), contiguity,
  alpha propagation, cape corners for known `vel`, glow quad appended to the bloom range,
  glow independent of `hide=bloom`, agent-key rows (text only, no new command), the filter
  line (text only), `motion = 0` zeroes lean and flutter, badge at least 8 px x `ui_scale` tall with an equal badge-to-avatar ratio at `ui_scale` 1 and 2, atlas
  table (rectangles inside 256x256, gutters of at least 4 px, no overlap, Deco
  `TEXTURE_MAX_LEVEL` is 2); the colour-blind pair test of AI-61; `no_alloc.rs` with
  decorations.

### 8.2 Golden and parity

- AI-62/AI-66: `hash_view` v1 unchanged for all existing logs; new fixtures
  `tests/fixtures/custom-log/ai-coauthors.log` (assisted human, agent author, bot, multiple
  credits, unordered timestamps, malformed meta) and `ai-coauthors-stripped.log` (same
  events without `ai=`), with a golden `DECO` hash table beside `sim_golden.tsv`.
- Test pair: stripped log versus the original rendered with `hide=decorations` give equal
  frame hashes at every step (AI-66). Two runs of the decorated fixture with `?fixed-dt`
  and the same options give equal frame hashes (AI-72).
- C++ differential cases (AI-19).
- `gource-rs convert --from git` on the AI fixture writes the AI-23 lines (the git path through
  parser and writer, AI-75).
- F2's `private_log_has_no_unclassified_names` (F2 PR-107) runs against the classifier once
  both F1 M1b and F2 M1a exist; whichever lands second adds it.
- Server integration (`crates/gource-server` tests, scratch bare repository): commits with
  trailers and identities carrying emails; the cached `.log`, its gzip copy and every
  `/api/jobs/*` response, every SSE progress event and the captured server stderr and
  tracing output contain none of the fixture's email addresses and none of the raw trailer
  text (AI-67); the bounded model grammars exclude `@` (AI-6), so a model cannot smuggle an
  address. A second run with detection off writes a log with no field and a different
  cache key (AI-28), and the job result reports `ai_detect: false`.
- Native versus wasm hash equality of the decorated fixture through the existing wasm
  harness (`scripts/parity`).

### 8.3 End to end

Playwright with `?mock=1` (no wasm): section visible, the new options round-trip through the
URL and reload, `ai-filter` marked restart, rules textarea with the preset select, mock
`ai_stats()` rendered by `ai-panel.ts`, export then import of a profile yields the same
rules, URL length hint, stored rules restored on reload and overridden by a URL (US-15),
the notices of AI-74, the filter line under the date, the disabled state (US-16, AI-29) for
a mock log with no sets and for `ai_detect: false`, and `ai-person-stats` / `ai-leaderboard`
toggling the hover line and the list.

Playwright against the real engine (`E2E_REAL=1`, `?fixed-dt=0.016`, new sample
`web/public/samples/ai-coauthors.log` next to `file-removal.log`): after `step(n)`,
`status().draw_calls` is 12 while a decorated user is on screen and 10 with
`hide=decorations`; `ai_stats()` numbers equal the fixture's; a bad rule shows
`style_bad_property` with the line number and keeps the previous canvas; canvas pixels
differ between default and `hide=decorations` and are equal between `hide=decorations` and
the stripped log; the panel lists every agent seen in the fixture as text with its count
(AI-61d); screenshot of the hero look recorded in `screenshots.spec.ts`, compared as a
pixel golden with the bloom tolerance of `PARITY.md` (MAD at most 1.5 and at most 3% of pixels
differing by more than 24 levels: the bloom-intense ceiling of 1.426 and 2.676%) and with the
measurable criterion, which is the gate, that decoration pixels cover at least 5% of the
decorated avatar's bounding box at the default zoom.

### 8.4 Acceptance, per requirement group

| Group | Accepted when |
|---|---|
| Classification AI-1..11 | T1..T27 pass (M3 vectors T12b, T17b with M3); registry digest test in CI |
| Wire AI-12..29 | meta, batch, git-parser and C++ differential tests pass; old logs and old-format git output parse unchanged; cache key test shows the registry version and the detection state in the hash input |
| Simulation AI-30..36 | state-machine, stats, filter, determinism and allocation tests pass |
| Style AI-37..50 | grammar, error, cascade, profile tests pass; option round trip passes |
| Render AI-51..62, AI-74, AI-77 | block, command, geometry, atlas, colour-blind and motion tests pass; the hero pixel golden matches within tolerance |
| UI AI-63..65 | mock e2e and real e2e pass |
| Security AI-67..71 | server integration test (below) passes; hostile-input unit tests (bad escapes, 100 KB meta, 65,537 distinct sets, 60,000 distinct models giving a bounded `ai_stats()`, pathological regex) finish within the limits |
| Parity AI-66 | stripped-versus-hidden test and all existing goldens pass untouched |
| Snapshot AI-72 | two `?fixed-dt` runs with the same options hash equal; the recipe lists the options |
| Accessibility AI-61 | colour-blind pair test, minimum badge size, `decor-motion=off` test and the panel text e2e pass |

## 9. Milestones

Each milestone cites the user stories it satisfies. The cut follows the request: a contributor
visibly wearing a cape, shades or a glowing hat first, a style the viewer can edit second,
power-user features and per-person numbers last. Sparkline, decoration shadows and name-only
inference are not specified here (they are listed at the end of this section). The
cross-feature ordering block at the end is identical in F1, F2, F3 and `docs/features/README.md`.

**M1a: the demoable slice (a cape on screen from a sample log).** US-1 (sample log), US-3, US-4,
US-10.
`AGENTS` table (AI-1, first half; `agents.rs`), custom-log field read and write (AI-15..23)
with the C++ differential cases (AI-19), batch v2 and `EventLog` sets with the cap in the
parsers (AI-24..27), assist state and `SetInfo` (AI-30, AI-31, AI-36), `BUILTIN_RULES_V1`
parsed by the resolver with the minimal grammar and no viewer rules (AI-38 subset, AI-41,
AI-42, AI-47), `hide=decorations` (AI-43), decorations cape (static), shades, halo, badge
(tint plus one shape) and glow through the bloom stream (AI-51..58 without lean, flutter or
`decor-motion`), the atlas cells those need (AI-54), `DECO` hash and goldens (AI-62, AI-66),
the sample `web/public/samples/ai-coauthors.log` and the fixture
`tests/fixtures/custom-log/ai-coauthors.log`, the contract table (AI-75), wasm size recorded.
Exit: `/?log=/samples/ai-coauthors.log` renders a cape and shades on the assisted user and a
halo and glow on the agent author in the real e2e, with the 5% coverage criterion met (and the
hero pixel golden within the tolerance of section 8.3); the stripped-versus-hidden frame
equality test passes; draw calls at most 12; all existing goldens untouched and `gource-cli`
hashes equal to today's; C++ reads the new logs unchanged; the wasm size is within budget.

**M1b: real repositories and the one-control look.** US-1 (real trailers), US-11.
Classifier and registry matchers with the digest test (AI-1 second half, AI-2 v1 signals,
AI-3..AI-11), git format and parser (AI-12..14), `ai_detect` and the cache key (AI-28, AI-29),
`ai-style=default|cape|shades|hat|none` and the panel's Built-in look select and Show
decorations checkbox (AI-43, section 4.4), server integration test and the no-email grep test
(AI-67), the section 7 server gate with its opt-in fallback, the `docs/web-mvp/PLAN.md` updates.
Exit: a scratch repository with trailers goes through the server to a page that shows the
decorations, and `ai-style=hat` swaps the cape for a glowing hat; no email in `cache/*.log`, its
gzip copy, `/api/jobs/*`, SSE events or server stderr; the server gate is measured and recorded
(detection default decided here); F2's `private_log_has_no_unclassified_names` is added if
F2 M1a exists. A manual smoke check on a public repository with real Claude trailers is
recorded but is not the gate.

**M2: controls and viewer rules.** US-2 (current-state line), US-4 (panel), US-5 to US-8, US-9
(share link), US-15, US-16, US-17.
Hover current-state line (AI-60, reduced), the nine other badge shapes, lean and flutter,
`decor-motion` (AI-56, AI-77), glow tuning, agent key with the `played_set_counts` seeding
(AI-59, AI-32 partial), `user-style` with the minimal grammar, cascade, resolver cache and
the M2 error codes (AI-37..43, AI-45 limits, AI-46, AI-47), `ai-filter` and its HUD line
(AI-34, AI-74), the panel section with presets, `check_style`, stored style and URL notices
(AI-64), `ai_stats()` loaded and played aggregates (AI-33 without `contributors`, AI-63),
`NAME_BEARING_OPTIONS` (AI-37), mock engine (AI-65), accessibility checks (AI-61), settings
snapshot test (AI-72).
Exit: US-4 to US-8, US-15 to US-17 pass in the real e2e; the round-trip test passes with the new
options and a non-empty rule list; every M2 error code has a test; the wasm and frame budgets
are recorded.

**M3: style power-user features and per-person insight.** US-2 (percentages), US-9 (export and
import), US-13, US-14. Starts with the decisions Q16 and Q17.
`name:` selector with lazy evaluation (AI-45, AI-70), `habitual` and the "Always on" preset
(AI-73), inverse polarity (AI-44), the `hash` value and the `colour` and `scale` properties,
model globs, the Bot kind, `bot` key and selector (AI-7), the committer, aider-suffix and
aider-prefix signals with the identity rewrite (AI-2, AI-4), the JSON profile envelope with
export and import (AI-48), per-contributor stats, hover percentages, `ai-person-stats`,
`ai-leaderboard` and the contributor list (AI-32, AI-33, AI-76), the regex benchmark, the section 7
budget table re-measured and recorded in `docs/web-mvp/PERFORMANCE.md`.
Exit: US-9, US-13 and US-14 pass in the real e2e; the regex benchmark of AI-45 passes; budgets are
recorded.

Not in F1: claimed styles. The later spec "claimed styles" (F4) owns storage, claim API, UI and
the code seam; AI-49 and AI-50 fix its contract (US-12). It follows F1 M3 and F2 M2a.

**M1 as the maintainer will see it.**

1. After M1a, build and open `/?log=/samples/ai-coauthors.log`.
2. `jane` wears a cape and shades while her credited commits' beams play, and loses them
   about a second later.
3. `Cursor Agent` shows a halo, a glow and a badge as long as it is active.
4. Append `&hide=decorations`: the scene is exactly the plain Gource scene.
5. After M1b, point the server at a repository whose history has `Co-Authored-By: Claude`
   trailers: the same gear appears on the contributors who made those commits; `&ai-style=hat`
   swaps the cape for a glowing hat; `&ai-style=none` removes the built-in look.

Later, not specified here and each needing its own short spec before work: an AI-share
sparkline under the seek slider (its drawing surface, DOM or HUD quads, is undecided),
decoration shadows behind a rule property (two more draw commands), the per-agent row
editor (Q14), an operator registry file (Q13), optional name-only inference (Q9).

**Cross-feature ordering (identical in F1, F2, F3 and the README).** Hard dependencies: F3 M1a
step 3 (the `hash_view` move to `gource-core`) before F1 M1b, or F1 M1b applies its DECO
section at the new path; F1 M1a (batch v2) before F3 M2 (`export_log` must write v2 and send
multi-value options as one string joined by `\n`); F1 M1b before F3 T-29 (and F1 M2 for
`user-style` and `agent-key`); F2 M1b (CSP) and F3 M1b: whichever ships second adds F3 T-2,
T-16 and T-19 (resp. the recording flows) to F2's `csp-clean` run; F2 M2a and PR-110 before F3
M3 private renders; the later spec "claimed styles" (F4) after F1 M3 and F2 M2a; the two
`gource_web_bg.wasm` size budgets are cumulative (section 7: at most +210 KiB gzip together).

## 10. Risks

| Risk | Effect | Mitigation |
|---|---|---|
| `%(trailers)` slows `git log` on large repositories | longer history phase | measured gate (section 7), detection off by default if the gate fails, startup version probe (AI-14) |
| Trailers are optional and removable (Claude Code `attribution.commit: ""`, Codex `commit_attribution` empty, Cursor SDK ignoring the opt-out) | undercounting | UI copy says "credited"; section 11 Q5 lists the limit; the count is a lower bound |
| Squash merges copy or drop trailers | miscounts per commit | dedupe (AI-5); the PR author's squash commit carries the union; documented |
| Vendors change trailer text (Codex default on or off is unconfirmed; the VS Code default flipped between releases) | stale registry | registry is data with a digest test and version in the cache key; unknown-but-valid ids still display |
| Mailmap does not apply to trailers | the same human appears under several emails; irrelevant for AI emails | classification uses exact tool emails, not person identity |
| Two commits with equal author and `%ct` collapse in the custom log | one commit counted | union of credits (AI-20); stats count the group once |
| 12 draw calls leaves little headroom | later features compete | F3 adds no draw calls (F3 REC-116); any later overlay needs its own spec; decoration shadows are a later, separate spec; the key adds none; glow adds none |
| Per-block draw order differs from per-user interleaving | rare overlap artefacts | users are 100 units apart by default; accepted (AI-52) |
| Hand-drawn atlas looks off in the scene | visual quality | tunable rect table, golden screenshot reviewed by the maintainer |
| `regex-lite` has no look-around (a rule written with PCRE habits fails) | confusing errors | `style_bad_regex` message names the subset; same engine as the existing filters |
| Two copies of the option table (Rust and TypeScript) drift | broken UI | the existing sync test gets the new names |
| Aggregates are by display name | a mailmap merging people merges stats | consistent with how Gource already treats identity |
| Glow shader noise differs per GPU | golden pixel flake | compare with tolerance as for bloom (`PARITY.md`) |
| A new Claude model family shows no model | none: the grammar is an open family (AI-6), so no registry edit and no cache-wide invalidation | only the word `Code` is excluded; a family word that is not a model name would show as a model, which is harmless text of bounded shape |
| Per-person AI numbers read as a performance ranking | profiling of named people from claims | `ai-person-stats` switch, leaderboard off by default, "credited to" wording, caveat (AI-76) |
| Vendor names and a cape could read as endorsement or as a trademark use | brand complaint | names as plain text only, a README line "not affiliated or endorsed", no logos or vendor colours (N3); the preset is called "Hero", no third-party character names in UI or identifiers |
| The wasm grows with the atlas | slower first load | 256 KiB atlas, size budget row (section 7) |
| Habitual use looks plain at slow playback | the insight depends on speed | `habitual` selector and preset "Always on" (AI-73) |

## 11. Open questions

Each has a recommended answer; accepting them all gives the design above.

1. Contributor name for agent-authored commits: keep the raw author name, or rename to the
   agent's display name? Recommended: keep the raw name (section 5.7).
2. Aider's `(aider)` suffix: strip it so aider commits merge into Jane as "assisted"
   (recommended: aider is a tool a human drives, the same workflow as Claude Code must look
   the same, and a person must not appear as two avatars), or keep `Jane (aider)` as a
   separate agent contributor? Specified in AI-4 but deferred to M3 with the suffix signal
   (AI-2); until then `Jane (aider)` stays a separate contributor.
3. Decorations on by default for data that has attributions? Recommended: yes, that is the
   feature; `hide=decorations` and the panel checkbox turn them off, and the share link
   carries the choice.
4. Committer-only agent match counts as Assist (AI-2)? Recommended: yes, weakest signal,
   never beats the author; deferred to M3, v1 reads trailers and the author identity only.
5. Accept that counts are a lower bound because attribution can be disabled? Recommended:
   yes; show "credited" wording and no percentages without a commit count.
6. Are Bot entries wanted at all (no default decoration, only `bot` selectors)? Recommended:
   reserve the kind and the wire key now and enable them in M3; v1 only excludes generic bots
   from AI (AI-7).
7. Ship a pseudonymous contributor id (`cid`) in logs for future claims? Decided: a repo-scoped
   HMAC of the numeric GitHub id from a noreply identity, public repositories only (AI-50);
   emitted by the later spec "claimed styles" (F4), reserved in the grammar now.
8. Should decoration shadows exist by default? Recommended: no (two more commands); not in
   this feature, a later separate spec.
9. Infer agents from contributor names for logs with no fields (for example a custom log
   with a user `claude[bot]`)? Recommended: no in M1 and M2, because it creates data from
   nothing and breaks the parity statement for plain logs; users can write
   `name:/claude/i => badge=on` themselves. Revisit as an explicit `ai-infer` option.
10. Agent key default? Recommended: off, like the file key, with the checkbox in the panel.
11. Minimum git version 2.22 (AI-14) acceptable for self-hosted servers? Recommended: yes,
    the probe falls back cleanly.
12. Per-agent tint conflicts (Codex sky blue and Jules blue are close)? Recommended: keep;
    badge shapes and names disambiguate, the colour-blind test of AI-61 lists the pair on its
    allow-list; revisit after the first user feedback.
13. An operator-supplied registry file (`--agent-registry <file>`: id, name, emails, logins,
    tint, badge) for self-hosted servers and private forges? Recommended: not in F1; if added
    later, validate it with the built-in limits and hash its content into the cache key
    (AI-28); extra matchers live outside the digest test.
14. A per-agent row editor (toggles and colour pickers that generate rules) for viewers who
    will not learn the grammar? Recommended: not in F1 (textarea, presets and
    import/export only); build it as a follow-up on top of the same rules, which stay the
    single source.
15. How an account proves it owns a contributor for claimed styles (D4b)? Decided (AI-49,
    AI-50, shared with F2 Q15): the numeric GitHub id. `cid(repo, account.provider_user_id)`
    must equal the `cid` in the log, a repo-scoped HMAC computed in memory during the git parse
    step from a `<digits>+<login>@users.noreply.github.com` identity, so no email is stored or
    shipped and a person cannot be linked across repositories; other emails need F2's opt-in
    verified-email permission. Storage, claim API and UI: the later spec "claimed styles" (F4).
16. Ship per-person AI percentages, `ai-person-stats`, the contributor list and the seek
    re-seeding they need at all? Recommended: not before the maintainer asks; M3 candidate
    behind this decision (the user asked to see AI involvement, not to rank people).
17. Are the style power-user features (`name:` regex, `habitual`, inverse polarity, `scale`,
    `colour`, globs, profile JSON) part of F1 M3 or their own short spec? Recommended: M3 here;
    split them into their own spec if M2 slips.

## 12. References

Vendor and git documentation

- Claude Code attribution setting and fallbacks (`attribution.commit`, `attribution.sessionUrl`):
  https://code.claude.com/docs/en/settings-reference.md
- Claude GitHub Action defaults (`action.yml`, `bot_id`, `bot_name`) and trailer prompt
  (`src/create-prompt/index.ts`): https://github.com/anthropics/claude-code-action
- Codex CLI `commit_attribution`, default `Co-authored-by: Codex <noreply@openai.com>`:
  https://github.com/openai/codex/pull/11617
- Copilot CLI trailer and the GitHub rendering note: https://github.com/github/copilot-cli/issues/5032
- VS Code `git.addAICoAuthor`: https://github.com/microsoft/vscode/issues/314311
- Cursor attribution opt-out reports: https://forum.cursor.com/t/local-executor-ignores-cli-config-attribution-opt-out-forcing-co-authored-by-trailer/167722
- Aider commit attribution (author suffix, `aider: ` prefix, trailer): https://aider.chat/docs/git.html
  and source `aider/repo.py` at https://github.com/Aider-AI/aider
- Jules authorship modes: https://jules.google/docs/changelog/2026-02-19/
- Gemini CLI has no stated convention: https://github.com/google-gemini/gemini-cli/discussions/11447
- git trailers: https://git-scm.com/docs/git-interpret-trailers
- `git log` `%(trailers:...)` options and `%an %ae %cn %ce`: https://git-scm.com/docs/git-log
- GitHub co-author syntax: https://docs.github.com/en/pull-requests/committing-changes-to-your-project/creating-and-editing-commits/creating-a-commit-with-multiple-authors
- Co-author contribution counting is disputed:
  https://github.com/orgs/community/discussions/186410
- A detector that also checks `%an`/`%ae` and `%cn`/`%ce` because squash merges regenerate
  trailers: https://github.com/Comfy-Org/ComfyUI_frontend/pull/18920
- `regex-lite` `RegexBuilder` limits: https://docs.rs/regex-lite/0.1.9/regex_lite/struct.RegexBuilder.html
- Okabe-Ito colour-blind-safe palette: https://jfly.uni-koeln.de/color/

Sampled real commits behind the vectors (repository level; commit ids are in the working
notes `docs/features/research/ai-trailers.md`)

- Claude trailers: https://github.com/Elessiah/blueGenjiBot,
  https://github.com/kevintanes/sales-management-system; Claude cloud session:
  https://github.com/leandersabel/solvent; Claude Action: https://github.com/VolvoxCommunity/Volvox.Website
- Codex trailers: https://github.com/shunk031/skills, https://github.com/a-novel-kit/uikit;
  Codex connector: https://github.com/daisy976/test-repo
- Copilot coding agent: https://github.com/claggjoshua5/Wreck-Less-Jump-calculatoor,
  https://github.com/phieri/viking-bio-pwa
- Cursor: https://github.com/QuietAgencyDev/Sullys1943, https://github.com/saltduck/gardenassist,
  https://github.com/flightctl/flightctl, https://github.com/brshood/JobSeekrrr
- Aider: https://github.com/proveo-ca/casbinjs
- Jules: https://github.com/Jokersochi/-
- Devin: https://github.com/coah80/wii-ipl, https://github.com/hudli123/hk-neon-arcade-
- Amp: https://github.com/unkeyed/unkey, https://github.com/dvydra/claude-tail,
  https://github.com/lox/amp-mcp-gateway
- OpenHands: https://github.com/OpenHands/OpenHands
- Amazon Q Developer: https://github.com/jobbyist/skinlabs-africa

In-repo contracts

- `docs/web-mvp/PLAN.md` (crates, browser API section 4, clone service section 5)
- `docs/web-mvp/PARITY.md`, `docs/web-mvp/PERFORMANCE.md`
- Working notes: `docs/features/research/data-path.md`, `docs/features/research/ai-trailers.md`, `docs/features/research/user-render.md`
