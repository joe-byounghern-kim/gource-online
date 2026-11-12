# Custom-log edge-case fixtures

Small inputs in Gource's pipe-delimited custom log format
(`timestamp|username|A/M/D|path[|RRGGBB]`), one scenario per file, for parser
and simulation tests of the Rust port. They complement the larger scenario
logs in `tests/logs/` used by the C++ reference build.

Origin: extracted unchanged from `tests/fixtures/*.log` in commit `ad869c9`
of https://github.com/PhilosophiMoonbeam/Gource (GPL-3.0-or-later). The
accompanying `*.expect.json` documents were not taken; expectations for these
inputs are defined by this port's own tests.

`ai-coauthors.log` and `ai-coauthors-stripped.log` are this port's own
(GPL-3.0-or-later): the first carries the optional meta field after the
colour slot (`timestamp|username|A/M/D|path|[RRGGBB]|ai=...`): an assisted
human, an agent author, a reserved `bot=` key, several credits on one
commit and over one commit's lines, unordered timestamps and malformed
fields; the second is the same events without the field, for the
"stripped equals hidden" parity tests.
