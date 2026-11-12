# CI and security readiness

Pull requests target **main**. CI and Security run on every pull request and on
pushes to main. A green PR head is not evidence for the resulting merge SHA.
Check the corresponding base run before treating that base as verified.

## Security coverage is explicit

- Cargo audit checks the locked native dependency graph. The reviewed rustls
  security remediation is 0.23.46, addressing RUSTSEC-2026-0346 through 0349.
- CodeQL's current job condition excludes private repositories. Its skipped
  status is **not passed scanning coverage**, and must not be counted toward
  beta acceptance. Enabling it later requires the appropriate repository
  visibility or licensed security configuration and actual successful runs.
- Scheduled parser fuzzing runs each target for **3600 seconds**. Native
  regression tests or a short smoke run do not replace that duration. Keep the
  scheduled fuzz acceptance open until a successful instrumented run exists.
  Concurrency groups include the event name, so pushes and manual smoke runs
  cannot cancel scheduled acceptance. Superseded runs of the same PR still
  cancel each other. Each fuzz job has a 90-minute timeout, leaving 30 minutes
  beyond the fuzz budget for toolchain installation, instrumented compilation
  and cleanup. This is headroom, not evidence of a completed scheduled run.

## Custom-log fuzz rejection regression

`0|||a||pr-log=` is a compact malformed coverage carrier. The production parser
accepts the custom-log envelope and marks PR coverage Unknown. Rich custom export
cannot faithfully represent that state and deliberately returns InvalidInput.
This is an exporter policy rejection, not a parser crash.

The custom-log fuzz property compares both successful output and error
kind/message, together with parser statistics, across chunkings. It must not
unwrap the deliberate rejection or silently ignore differences between errors.
The property is shared by the instrumented target and a native regression test.
The committed seed and byte-deletion check make the reproducer independently
repeatable without installing fuzz tools.

Before a release, run the full commands in `fuzz/README.md`, keep the actual results, and report an
omitted or unavailable check as not run, never as passed.
