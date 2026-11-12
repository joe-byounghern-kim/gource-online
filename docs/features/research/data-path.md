<!-- Research notes gathered on 2026-10-02 (branch web-mvp, HEAD 8a3ecec) for the feature specs in docs/features/. Working notes: claims are marked verified or inferred; file:line anchors describe that commit. -->

# Notes: commit authorship flow and where AI co-author data could enter

> **History.** A design record from the MVP. Mentions of `gource-server` refer to the original
> Rust clone service, which is not part of this repository; the hosted service implements the
> same API (`packages/protocol/src/api.ts`).

Repo state: branch `web-mvp`, HEAD 8a3ecec. Everything below is **verified** by reading the cited code, or by running local git 2.43.0 in a scratch repo, unless it is marked *inferred*. Paths are relative to the repository root.

## 1. How authorship flows today (verified)

- **Git command.** The server runs `git -c core.quotepath=off -c mailmap.blob=<oid|empty> log --reverse --raw --encoding=UTF-8 --no-renames --no-show-signature --pretty=format:user:%aN%n%ct <range>` (`crates/gource-server/src/pipeline.rs:744-752`).
  - It runs with no network (`local_command`, `pipeline.rs:743`), against a blobless bare clone.
  - Only the author name (`%aN`) and committer time (`%ct`) are fetched. There is no email, no `%B`, no `%(trailers)`.
  - `gitcmd.rs` builds the command environment only (`gitcmd.rs:75-133`). It contains no log format.
- **Commit messages and trailers are not fetched.** Only `%aN` and `%ct` are requested, so the answer to question (2) is no.
  - A blobless clone omits file blobs, not commit objects. Commit messages are available to `git log` with no extra fetch, because `GIT_NO_LAZY_FETCH` and `protocol.allow=never` apply only to missing blobs.
  - The `.mailmap` blob is fetched separately and size-capped (`pipeline.rs:396-565`, `max_mailmap_bytes`). It is applied through `-c mailmap.blob=` (`pipeline.rs:414-418`).
- **Server parse.** `GitRawParser` reads the git output (`crates/gource-sources/src/git.rs`).
  - A block starts at `user:<name>`, and the next line is the timestamp via `atol` (`git.rs:255-264`, `239-248`).
  - File lines need a TAB (`git.rs:274-281`). A blank line ends the commit (`git.rs:250-253`).
  - Merge headers with no files are dropped (`git.rs:319-320`).
  - It emits a `ParsedCommit { timestamp, username, files, byte_offset }` (`crates/gource-sources/src/lib.rs:52-60`).
  - `ParsedFile` is `{ path, kind, colour: Option<[u8;3]> }` (`lib.rs:43-50`).
- **Server write.** `CustomLogWriter` writes one line per file: `ts|user|A|path[|RRGGBB]`. It replaces `|` with `_` and drops CR/LF (`crates/gource-sources/src/writer.rs:193-224`).
  - `BoundedWriter` enforces `max_log_bytes` (256 MiB default, `config.rs:73`) using a `LINE_OVERHEAD` constant (`pipeline.rs:605-608`, `620-636`). That constant would need updating for any new column.
  - The log goes to `<key>.log.partial` plus a `.log.gz.partial` copy (`pipeline.rs:30-31`, `154-198`). Both are renamed on completion.
- **Client parse.** The web worker runs the wasm `LogParser`, which is `ChunkParser { CustomLogParser, BatchEncoder }` (`crates/gource-web/src/parser.rs:9-35`).
  - Batches reach the main thread through `GourceApp.append_batch`, which calls `apply_batch` into the `EventLog` (`crates/gource-web/src/wasm.rs:89`, `engine.rs:265`).
  - The TypeScript worker protocol carries only opaque `ArrayBuffer` batches (`web/src/worker/protocol.ts:21-28`). It needs no change if the batch codec is extended.
- **Simulation identity.** `EventLog` holds `CommitRecord { timestamp, user: StrId, first_change, change_count, byte_offset }` (`crates/gource-core/src/events.rs:67-75`).
  - The user is resolved by name string: `user_map.get(log.str(user))` (`crates/gource-core/src/sim.rs:1267-1268`).
  - Identity is therefore the display name only. There is no email and no actor type.
- **User filters.** Filters match the interned user string and are cached per `StrId` (`crates/gource-core/src/filters.rs:84-101`, `131-136`).

## 2. Custom-log tolerance of a 6th field

**C++ (verified).** The regex at `src/formats/custom.cpp:21` is:
`^(?:\xEF\xBB\xBF)?([^|]+)\|([^|]*)\|([ADM]?)\|([^|]+)(?:\|#?([a-fA-F0-9]{6}))?`

- It has no `$` anchor. `Regex::matchOffset` calls `pcre2_match` with options 0 on the whole line (`src/core/regex.cpp`), so trailing text after the match is ignored.
- `[^|]+` for the path stops at the next `|`.
- Colour group 5 is optional. If it does not match, `pcre2_match` returns rc=5, so `entries` has 4 elements. The code `entries.size()>=5 && entries[4].size()>0` is then false and the line has no colour (`custom.cpp:89-98`).
- Cases:
  - `1|u|A|/a|ff0000|X` gives colour ff0000, and `X` is ignored.
  - `1|u|A|/a||X` (empty colour slot) has no colour, and `X` is ignored. This is **safe**.
  - `1|u|A|/a|X` (colour slot absent) is **hazardous**. If `X` begins with six hex characters (`deadbeef…`, `facade…`), it is read as a colour.
  - Anything in a 7th or later field is ignored.
- **A new field must be preceded by an explicit colour slot, either empty or six hex digits.** Then both parsers ignore the new field.

**Rust `CustomLogParser` (verified).** It is equivalent to C++.

- `parse_line` takes the path up to the next `|` and passes the remainder `tail` to `parse_colour` (`custom.rs:314-329`).
- `parse_colour` takes an optional `#`, then needs six hex digits at the start of the tail. Whatever follows is ignored (`custom.rs:387-400`).
- Existing tests pin this (`custom.rs:563-571`):
  - `1|a|A|p|ff0000|more` gives red.
  - `1|a|A|p|` gives no colour.
  - `1|a|A|p|12345` gives no colour.
- The doc comment states "anything after the match is ignored" (`custom.rs:30-33`).
- Neither parser rejects lines with extra fields.

## 3. Grouping and per-commit attachment

- **Grouping rule (verified).** Consecutive lines with the same timestamp and the same raw username form one commit, in both C++ (`custom.cpp:76-84`) and Rust (`custom.rs:181-191`).
  - The comparison is `pending.timestamp == rec.timestamp && pending.user == rec.username`.
  - Nothing else participates. A new field would not break grouping unless it were added to the key, which it should not be.
  - The Rust parser never inspects the pending commit's extra data.
  - A malformed line ends the commit in progress (`custom.rs:173-179`).
- **Two real commits can merge (verified from the rule).** If one author has two commits with the same `%ct`, git emits them as separate blocks but the custom log groups them into one commit. Co-author data must tolerate a "commit" that merges several real ones. Taking the union of co-authors, or the first line's value, would both work.
- **Writer choice.** The writer emits one line per file, so a commit's co-author field would repeat on every line.
  - *Inferred:* it is cheaper to write the field only on the first line of each commit and have the parser read it from that line only. A later line's field would be ignored or unioned.
  - The cost of repeating it is bounded by `max_log_changes` (3M, `config.rs:74`).
- **Safe encoding for the new field.**
  - Keep the line as `ts|user|A|path|<colour or empty>|<coauthor-field>`.
  - The server already sanitizes `|` to `_`, and CR/LF are removed. Trailer values can contain TAB (verified with git: `Tab^IPerson <t@x>`), so a field separator other than TAB is needed, for example `;` or `\x1f`.
  - Names containing `;` would need escaping or the same `_` treatment.
- **Extraction (verified locally with git 2.43).** `%(trailers:key=Co-authored-by,valueonly,separator=%x1f)` followed by `%x1e` produced `Claude Opus 4.5 <noreply@anthropic.com>^_Bob Old <bob@old.example>^^`.
  - Key matching is case-insensitive. `Co-Authored-By:` and `co-authored-by:` both matched.
  - Docs say `key=` is case-insensitive and implies `only` (https://git-scm.com/docs/git-log).
  - GitHub's co-author syntax is `Co-authored-by: NAME <EMAIL>`, one trailer per co-author (https://docs.github.com/en/pull-requests/committing-changes-to-your-project/creating-and-editing-commits/creating-a-commit-with-multiple-authors).
  - The extra line must never be empty and must contain no TAB. `GitRawParser` ends a commit on a blank line (`git.rs:250-253`) and treats any TAB line as a file line (`git.rs:274-281`). The `%x1e` terminator in my test keeps the line non-empty.
  - `GitRawParser` would also need a new state for this line, because the line after the timestamp is currently consumed as a file line or skipped (`git.rs:239-248`).
  - The `.mailmap` blob is **not** applied to trailers. In my test the trailer stayed `Bob Old <bob@old.example>` while `%aN` was remapped.
  - The docs fetch implied a `mailmap` option for `%(trailers)`. Local git 2.43 prints `%(trailers:...,mailmap)` literally, so that option is unsupported there. The git version in the server image was not checked (unverified). Treat it as unavailable unless the pinned git version is raised.
  - Email addresses in trailers are unmapped. Classification should match on email or name patterns, not on mailmapped identity.
- **AI-actor classification is not covered by the code.** *Inferred:* the parser should emit raw `name <email>` and let a separate classifier map identities to an actor kind. That keeps the log format neutral and lets user-defined styles stay a client concern.
- **Other trailers are not read by anything.** A grep for `co-author`/`trailer` found only an unrelated hit in `logstream.rs`.

## 4. Minimal changes to carry co-authors from git trailers to `EventLog`

| Layer | Change | Anchor |
|---|---|---|
| git format | Append `%n%(trailers:key=Co-authored-by,valueonly,separator=%x1f)%x1e` (or similar) to `--pretty` | `pipeline.rs:749` |
| `GitRawParser` | New state after the timestamp line to read the trailer line; handle the empty-merge case; keep the existing `user:` / timestamp invariants | `git.rs:239-264` |
| `ParsedCommit` | Add something like `coauthors: &str` (a separator-joined string) | `lib.rs:52-60` |
| `CustomLogWriter` | Emit the (possibly empty) colour slot then the field; only on the commit's first line | `writer.rs:193-208` |
| `BoundedWriter` | Include the field length in the byte bound | `pipeline.rs:608`, `624-631` |
| `CustomLogParser` | Parse the 6th field from the first line of each commit; keep grouping unchanged | `custom.rs:314-329`, `181-191` |
| `BatchEncoder` and `apply_batch` | Add a per-commit co-author string id; bump the batch `VERSION` | `batch.rs:89`, `197-200`, `428` |
| `EventLog` | Add the co-author id to `CommitRecord` (or a side table) and to `push_commit` | `events.rs:67-75`, `140-163` |
| `EventLogSink` | Intern the new strings the same way, in the same order as the encoder | `sink.rs:112-148` |
| Cache | Bump `KEY_VERSION` | `cache.rs:20` |

- **Batch codec (verified).** The wire format is the `GEB` magic, `VERSION=1`, and a 24-byte header (`batch.rs:30-37`, `88-90`).
  - Per commit it carries timestamp zigzag, user varint, offset zigzag, and change count (`batch.rs:41-54`).
  - Per change it carries a path varint, a flags byte, and optional RGB. Flag bits 3-7 are zero (`batch.rs:50-52`).
  - Version mismatch is a hard reject: `if r.u8()? != VERSION { return err("unsupported version") }` (`batch.rs:428-430`).
  - The worker and the page ship in the same bundle, so a version bump is safe. A new commit-level field can be added with a bump to 2. Alternatively, a spare commit-level flag would need a new byte, because none exists today.
- **`EventLogSink` parity.** The module comment says string ids must equal the encoder's (`sink.rs:89-94`). Both must intern co-author strings in the same order.

## 5. Cache invalidation (verified)

- The key is `sha256(KEY_VERSION, host, lowercased path for GitHub/GitLab, branch, head, since, until, subdir)`, with fields separated by `\0` (`cache.rs:49-87`).
- `KEY_VERSION = "gource-log-v1"` has the comment "Bumped when the log format or the git command changes" (`cache.rs:19-20`).
- The variant key uses `"{KEY_VERSION}-variant"` (`cache.rs:68-72`).
  - **Consequence:** bumping the version changes every variant hash. The "delete older heads of the same variant" cleanup (`cache.rs:91-100`) will not match old-version entries. They are removed only by LRU eviction against `Budget` (`cache.rs:102-117`).
  - That is acceptable but worth noting. No test pins the version string (grep found only `cache.rs`).
- `CacheMeta` has `#[serde(default)]` optional fields (`cache.rs:23-35`). A new optional field, such as a `format` marker, is backward-compatible, but the key bump alone is sufficient.
- Old cached logs without the 6th field still parse in the new client, because the field is optional.
- A new server serving an old log (not possible after the bump) would simply show no co-authors.

## 6. Memory at about 100k commits (verified, plus estimates)

- `EventLog` stores `strings: Vec<Box<str>>` and `lookup: HashMap<Box<str>, StrId>` (`events.rs:88-96`).
  - `push_string` clones the boxed string into both (`events.rs:119-125`), so **each distinct string is allocated twice**.
  - The worker's `BatchEncoder` keeps a third copy, `HashMap<Box<str>, u32>` (`batch.rs:100`, `185`). That one lives in the worker.
- **Records.** `CommitRecord` is about 32 bytes (*inferred* from field sizes: i64 + 3×u32 + u64 plus padding). 100k commits is roughly 3.2 MB.
  - `FileChange` is about 12 bytes (*inferred*).
- **Co-author strings.** Intern the full `name <email>` string, or a per-identity id, rather than the raw joined field.
  - A repository with AI assistance typically has a handful of distinct identities, such as `Claude <noreply@anthropic.com>`, so the extra table cost is negligible.
  - Interning the joined list per commit would multiply strings by the number of distinct combinations. Per-identity ids with a small id list per commit are cheaper. A `u32` per commit is the minimum.
- **Strings at scale.** Paths dominate the string table, not authors. Co-author strings add little.
- **Log size.** The log-size budget (256 MiB, 3M changes) is about line counts, not commits. Writing the field on only the first line of each commit keeps the growth to roughly one short field per commit.

## Gaps and risks

- The git version bundled in the server image was not verified. `%(trailers:key=…,valueonly,separator=…)` works in git 2.43.0 here.
- A commit with several real commits sharing author and timestamp collapses into one custom-log commit (see section 3).
- Squash and merge commits can carry multiple co-author trailers, including ones copied from the squashed commits. De-duplication per commit is *inferred* to be needed.
- Privacy: co-author emails would flow into the cached log and to every client. Whether to keep or drop emails after classification is a spec decision.