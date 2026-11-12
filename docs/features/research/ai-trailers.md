<!-- Research notes gathered on 2026-10-02 (branch web-mvp, HEAD 8a3ecec) for the feature specs in docs/features/. Working notes: claims are marked verified or inferred; file:line anchors describe that commit. -->

# External research: how AI coding tools mark commits (as of 2026-10-02)

"Verified" below means I checked it in vendor docs, vendor source, or real GitHub commits fetched with `gh` or `curl`. "Inferred" means it is a conclusion I drew. I edited no files. Clone and release-notes lookups used temporary directories only.

## 1. Main table

| Tool | Where the signal is | Exact pattern | Model name? | Source | Confidence |
|---|---|---|---|---|---|
| Claude Code (CLI/IDE) | Trailer. The human stays author. | `Co-Authored-By: <name> <noreply@anthropic.com>`. `<name>` is the model in use, e.g. `Claude Sonnet 5`, `Claude Opus 5.5`, `Claude Haiku 4.5`. It is plain `Claude` if the version is unknown, and `Claude Code` for unmatched or third-party model IDs. A subagent's commit names the subagent's model. | Yes, as `Claude <Family> <ver>` | https://code.claude.com/docs/en/settings-reference.md (section `attribution.commit`). Real commits: Elessiah/blueGenjiBot (`Co-authored-by: Claude Opus 5.5 <noreply@anthropic.com>`), kevintanes/sales-management-system (`Claude Haiku 4.5`, `Claude Sonnet 5`). | High |
| Claude Code, cloud or Remote Control session | Author and committer, plus a trailer | Author and committer are `Claude <noreply@anthropic.com>`, which resolves to GitHub user `claude` (id 81847). Adds `Claude-Session: https://claude.ai/code/session_<id>`. The `attribution.sessionUrl` setting, default true, controls that trailer. | Not in the author | Settings reference above. Real commits: leandersabel/solvent PR #82. | High |
| Claude GitHub Action (`claude-code-action`) | Author and committer. The trailer credits the human, not the AI. | Author and committer are `claude[bot] <41898282+claude[bot]@users.noreply.github.com>`. The trigger user gets `Co-authored-by: <human> <email>`. | No | `action.yml` defaults `bot_id` 41898282, `bot_name` claude[bot]; `src/create-prompt/index.ts` ~L412-453. Real commits: VolvoxCommunity/Volvox.Website. | High |
| OpenAI Codex CLI | Trailer. The model is told to add it via a prompt instruction; there is no git hook. | `Co-authored-by: Codex <noreply@openai.com>`. Config key `commit_attribution` in `config.toml`. A string is used verbatim; an empty string disables it. | No. The default is static; users must edit the string by hand. | PR https://github.com/openai/codex/pull/11617 (diff: `DEFAULT_ATTRIBUTION_VALUE`). Real commits: shunk031/skills, a-novel-kit/uikit. | High for the string. Medium for the default being on today. |
| OpenAI Codex cloud / GitHub app | Author login | `chatgpt-codex-connector[bot] <199175422+chatgpt-codex-connector[bot]@users.noreply.github.com>`. Seen in a test repo only. | No | Real PR daisy976/test-repo #2 and #3. | Medium |
| GitHub Copilot coding agent | Author. A trailer credits the human requester. | Author is `copilot-swe-agent[bot] <198982749+Copilot@users.noreply.github.com>` (GitHub login `Copilot`). Some commits show the author name as `Copilot`. Committer is `GitHub <noreply@github.com>` (web-flow). Trailer is `Co-authored-by: <requesting human> <id+login@users.noreply.github.com>`. | No | Real commits: claggjoshua5/Wreck-Less-Jump-calculatoor PR #33, phieri/viking-bio-pwa PR #251. | High |
| Copilot CLI / app / VS Code | Trailer | Copilot CLI/app: `Co-authored-by: Copilot App <223556219+Copilot@users.noreply.github.com>` followed by `Copilot-Session: <uuid>`. VS Code: `Co-authored-by: Copilot copilot@github.com`, set by `git.addAICoAuthor` (off, `chatAndAgent`, `all`). | No | https://github.com/github/copilot-cli/issues/5032 (quotes the CLI trailer); https://github.com/microsoft/vscode/issues/314311. | High for the strings, because the vendor issues quote them. The VS Code default has flipped between releases, and the issue says it is now back to `off`. |
| Cursor (IDE/CLI agent) | Trailers | `Co-authored-by: Cursor <cursoragent@cursor.com>` and `Made-with: Cursor`. Set in Settings > Agent > Attribution; the CLI key is `attribution.attributeCommitsToAgent` in `cli-config.json`. SDK and cloud agents ignore the opt-out (forum bug reports). | No | Real commits: QuietAgencyDev/Sullys1943, saltduck/gardenassist (both trailers on one commit), flightctl/flightctl (`Made-with: Cursor`). https://forum.cursor.com/t/local-executor-ignores-cli-config-attribution-opt-out-forcing-co-authored-by-trailer/167722 | High |
| Cursor cloud/background agent | Author and committer | `Cursor Agent <cursoragent@cursor.com>`, GitHub login `cursoragent` (id 199161495, type User). | No | Real commits: brshood/JobSeekrrr PR #21600. | High |
| Aider | Author/committer name suffix, message prefix, optional trailer | Default: author and committer name become `<User> (aider)`. Optional: message prefix `aider: ` (`--attribute-commit-message-author`). Optional: `Co-authored-by: aider (<model>) <aider@aider.chat>` (`--attribute-co-authored-by`, default off). When the trailer is on, the `(aider)` suffix is dropped unless explicitly enabled. | Yes, as `aider (gpt-5.4)` or `aider (gemini/gemini-3.1-pro-preview)`. It is the model's litellm ID and can carry a provider prefix. | Source: `aider/repo.py` on main, `f"\n\nCo-authored-by: aider ({model_name}) <aider@aider.chat>"`. Docs: https://aider.chat/docs/git.html. Real commits: proveo-ca/casbinjs. | High |
| Google Jules | Author and committer | `google-labs-jules[bot] <161369871+google-labs-jules[bot]@users.noreply.github.com>`. Jules lets users choose `Jules`, `co-authored`, or `user only` authorship. The co-author trailer text is not shown in the docs I fetched. | No | https://jules.google/docs/changelog/2026-02-19/. Real PRs: Jokersochi/- #154. | High for the bot mode. Low for the co-author mode. |
| Gemini CLI | No official convention | Maintainers have not answered the question. The community suggests `gemini-code-assist[bot] <176961590+gemini-code-assist[bot]@users.noreply.github.com>`, which is the Code Assist app, not the CLI. | No | https://github.com/google-gemini/gemini-cli/discussions/11447 | Medium that no default exists. |
| Devin | Author and trailer, in two shapes | Shape A: author `devin <devin@cognition.ai>`, with `Co-Authored-By: <human>`. Shape B: human author plus `Co-Authored-By: Devin AI <158243242+devin-ai-integration[bot]@users.noreply.github.com>`. | No | Real PRs: coah80/wii-ipl #774 (A), hudli123/hk-neon-arcade- #2 (B). | High that both exist. Medium on when each is used. |
| Amp (Sourcegraph, now Amp Inc.) | Trailers | `Co-authored-by: Amp <amp@ampcode.com>` plus `Amp-Thread-ID: https://ampcode.com/threads/T-<id>` | No | Real commits: unkeyed/unkey, dvydra/claude-tail, lox/amp-mcp-gateway. | High |
| OpenHands | Trailer | `Co-authored-by: openhands <openhands@all-hands.dev>` | No | Real commits in OpenHands/OpenHands (sampled from its latest 100 commits). | High |
| Amazon Q Developer | Author login (GitHub app) | `amazon-q-developer[bot] <208079219+amazon-q-developer[bot]@users.noreply.github.com>`. No trailer convention found. | No | Real PR: jobbyist/skinlabs-africa #165. | Medium |
| Sweep | Bot login only | `sweep-nightly[bot]` exists (id 131841235). I did not inspect its commits. | No | `gh api users/sweep-nightly[bot]` | Low |
| Windsurf/Codeium, Tabnine | Nothing found | No documented trailer or bot account. `gh api users/{windsurf,codeium,tabnine}[bot]` all returned 404. | n/a | Web search plus 404s | Low. Treat as unknown. |

## 2. Generic bots, which must not be classed as AI

All are GitHub `Bot` accounts, verified with `gh api users/<login>`:
- `dependabot[bot]` (id 49699333)
- `renovate[bot]` (id 29139614)
- `github-actions[bot]` (id 41898282)

The `[bot]@users.noreply.github.com` suffix alone therefore cannot mean AI. The Jules, Devin, Copilot, Claude, Codex and Amazon Q bots all share it. Classification needs a named allowlist or denylist of bot logins.

One gotcha follows from the id lookup. The Claude Action's default email uses id 41898282, which is `github-actions[bot]`'s id, while `claude[bot]` is actually id 209825114. GitHub resolves the avatar by id, so match on the name part and never on the numeric id. This is my inference from two verified facts (`action.yml` default and the users API).

## 3. Git trailer syntax and parsing

Everything in this section is verified against https://git-scm.com/docs/git-interpret-trailers.
- Keys are ASCII alphanumerics and hyphens, and are case-insensitive. `Co-Authored-By` and `Co-authored-by` are the same key. Real data shows both spellings on one squash-merged commit, so dedupe on email.
- The separator is `:`. Whitespace is allowed around the separator and in values, and values can fold over continuation lines.
- The trailer block is the last paragraph of the message. It must be all trailers, or contain at least one git-generated or configured trailer and be at least 25% trailers.
- Repeated keys are allowed, so multiple `Co-authored-by` lines are normal.
- Lines whose value starts with `//` are not trailers (URL exclusion).
- GitHub is stricter than git. Copilot CLI issue #5032 reports that a trailer followed by `Copilot-Session:` is not recognised as co-authorship on GitHub, although git itself still parses it. The AI trailer may therefore not show as a co-author on GitHub, so do not rely on GitHub's rendering.

Git's `%(trailers)` format, checked against `Documentation/pretty-formats.txt` at each tag:

| Option | Introduced in | Note |
|---|---|---|
| `%(trailers)` (plain) | by v2.13.0 | |
| `unfold`, `only` | by v2.21.0 | |
| `key=<K>`, `separator=`, `valueonly` | v2.22.0 | RelNotes: "pick trailers selectively by keyword, show only values" |
| `keyonly`, `key_value_separator=`, and `=<BOOL>` argument syntax | v2.31.0 | |

- `key=` matches case-insensitively and the trailing colon is optional.
- Passing `key=` automatically turns on `only`.
- A comma inside `separator=` must be written `%x2C`.
- The local `git --version` is 2.43.0, so every option above is available here.
- A usable command for the server is `git log --format='%H%x1f%(trailers:key=Co-authored-by,valueonly,separator=%x1e)'`. I did not run it against a repo; it follows from the docs above.

## 4. How GitHub counts co-authors

- GitHub documents `Co-authored-by: name <email>`, one line per co-author. It says "for the commit to count as a contribution, use an email address associated with their account" (https://docs.github.com/en/pull-requests/how-tos/commit-changes/creating-a-commit-with-multiple-authors).
- Sources conflict on whether a co-author gets contribution-graph credit. A community discussion answer says it does not: "does not change how contributions are counted for graphs, stats, or streaks" (https://github.com/orgs/community/discussions/186410). The GitHub doc wording above implies it does. I could not settle this.
- Squash merge regenerates `Co-authored-by` trailers from the squashed commits' authors. This is why message-only detection misses cases. The Comfy-Org/ComfyUI_frontend PR #18920 fixed its detector for exactly this reason and now checks `%an <%ae>` and `%cn <%ce>` as well as trailers.

## 5. Detection from the email domain

Verified by sampling real commits. These emails do identify AI tools, with no model name in the email itself:

| Email | Tool |
|---|---|
| `noreply@anthropic.com` | Claude |
| `noreply@openai.com` | Codex |
| `cursoragent@cursor.com` | Cursor |
| `aider@aider.chat` | Aider |
| `amp@ampcode.com` | Amp |
| `openhands@all-hands.dev` | OpenHands |
| `devin@cognition.ai` | Devin |
| `copilot@github.com` | VS Code Copilot |

The `[bot]@users.noreply.github.com` family also identifies AI tools, but only via an allowlist (see section 2). Matching on the email alone is unreliable for two reasons:
- Users can change every trailer. Claude Code supports `attribution.commit: ""`, and a CLAUDE.md rule overrides the default.
- The `(aider)` author suffix has no fixed email.

## 6. Model-name extraction

| Tool | Where the model name lives | Parse |
|---|---|---|
| Claude Code | Trailer display name | `Claude (Opus\|Sonnet\|Haiku) <ver>` |
| Aider | Parenthesised text in the trailer name | `aider \((.+)\)`, may include a provider prefix such as `gemini/` |
| Everyone else | No model name | Use a tool-only value |

- Claude names can be bare `Claude` or `Claude Code` (fallbacks), so treat the model as unknown in those cases.
- Cursor's `Generated-By: cursor-agent/0.42 (model: ...)` example appeared only in a third-party article (crashoverride.com). I did not find it in a real commit. Treat it as unverified.

## Gaps

- The Claude Code trailer wording and fallbacks are verified from official docs, and this session's own commit instructions match that format. I did not check the Windsurf, Tabnine or Sweep commit conventions beyond the 404 lookups.
- I did not confirm that Codex's attribution is on by default in the current release. The `codex_git_commit` feature flag was `Removed` on main (`codex-rs/features/src/lib.rs`), and the config file path from the PR 404s on main.
- Several `gh search` calls hit GitHub's secondary rate limit, so a few queries (Windsurf, Gemini CLI commits) are missing.