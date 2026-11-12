# Opt-in final-line source receipts

The native converter has an explicit source mode for producers that need to bind final custom-log lines to source commits. This is a low-level conversion contract, not repository ownership proof, authentication, email matching or a public hosted service. Those decisions remain with the producer.

Ordinary `gource-rs convert` behavior is unchanged. Source mode requires both flags:

```sh
gource-rs convert --from git \
  --source-actors /private-staging/actors.tsv \
  --source-receipts /private-staging/receipts.jsonl \
  /private-staging/history.raw - > /private-staging/history.log
```

Use fresh private staging and discard both outputs unless the git producer and converter exit successfully and both output streams have been drained, closed and validated. Receipts do not authenticate an input stream forged by an untrusted producer. Never publish a partially written log or receipt file.

## Actor map and input grammar

The UTF-8 actor file contains one LF-terminated TSV row per commit:

```text
0123456789abcdef0123456789abcdef01234567\tperson_a\n
```

The displayed `\t` and `\n` represent a literal TAB and LF. Commit IDs must be full lowercase hexadecimal SHA-1 or SHA-256 values, exactly 40 or 64 characters. Labels are opaque `[A-Za-z0-9_-]` strings, 1 to 128 characters. Duplicate SHAs are rejected. Different commits may share a label. Maps are bounded to 150,000 entries and 32 MiB, with bounded rows. Known GitHub token markers are not permitted in labels.

Source-mode git input is the converter's raw git format with an additional `source:<full-lowercase-SHA>` header immediately after the integer nonzero timestamp, or immediately after the optional `m1` metadata line. Headers must occur in the order `user`, timestamp, optional `m1`, `source`, then actual raw file records. Hosted producers must supply `PRETTY_AI`-equivalent `m1` metadata evidence even when AI detection is disabled, plus the source SHA, while omitting commit bodies. `m1` is the F1 metadata encoding, not a merge marker. Missing, duplicate or misplaced source headers, including at fileless commits and EOF, fail the whole conversion. Arbitrary body text resembling a raw header is not trusted as a file record.

An emitted commit must be present in the actor map. Identity is parsed even when AI detection is disabled. The parser keeps the original mailmap-resolved author name through normal F1 classification. The final CLI sink then substitutes only the opaque username, preserving classified attributes. Replacing the username earlier could change Agent versus Assist classification and is not equivalent.

## Receipt schema

Each JSONL receipt has exactly the source binding and final line interval:

```json
{"schema":1,"kind":"final-custom-line-span","sha":"0123456789abcdef0123456789abcdef01234567","label":"person_a","start":0,"end":3}
```

`start` and `end` are zero-based half-open counts of final custom-log LF writes accepted by the output sink. They refer to actual final output, not raw input offsets, parsed file counts or bytes. Empty merge spans are not fabricated. A successful exit is necessary but not sufficient for hosted acceptance: the producer must also validate the exact SHA/label map, ordered nonoverlapping spans, line count, log schema and the paired output identity before atomic publication.

Any source-mode log or receipt write/flush failure, including BrokenPipe, produces a nonzero exit. Ordinary pipelines such as conversion into `head` retain their existing BrokenPipe behavior. Source errors are projected and do not echo raw source headers, actor-map values or path contents.

## Bounds and supported paths

Source mode additionally imposes hard ceilings of 256 MiB converted output, three million changes, 200,000 files per commit and 32 MiB retained commit data. Existing lower requested limits still apply. It is git-only and cannot be combined with pull-request conversion.

Source mode fails closed on decoded/unquoted unsafe paths before lossy filtering: invalid UTF-8, known GitHub token markers, `|`, backslash, and empty, `.` or `..` path components are unsupported. Rejected control characters are C0 U+0000–001F, DEL/C1 U+007F–009F, U+061C, U+200E/F, U+202A–E and U+2066–9. Model attributes and labels reject the same characters and markers, and also `@`. A path may hold `@` (`logo@2x.png`, `node_modules/@types`): it is the repository's own file name, so an email-shaped file name is emitted as it is, while an author's address never reaches the output because the label replaces the author. Unsupported data rejects the entire conversion rather than dropping credits, redacting paths or inventing collisions. This deliberately narrows supported repository paths. Marker checks do not establish that arbitrary filenames are universally secret-free. Producers must independently validate their output privacy policy.

Input, actor-map, receipt and output aliases are rejected, including existing hardlinks and newly created case-folding or Unicode-normalization aliases. The receipt path must be fresh. Creation/cleanup guards precede output truncation so two names resolving to one inode cannot yield an apparently valid mixed pair.

## Verification scope

Native regression tests cover streaming chunk boundaries, real root and merge records, forged body headers, missing/duplicate source IDs, map limits, F1/mailmap behavior with AI detection on and off, unsafe decoded paths/model markers, option-like ordinary filenames, aliasing, partial accepted writes and write/flush/BrokenPipe errors. Native tests and receipts do not replace a producer's cancellation, authority, quota, pinned-head, acquisition-parity or cross-instance acceptance tests.
