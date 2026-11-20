---
name: crap
description: Run a CRAP report (cyclomatic complexity × test coverage) over Hush's Rust code with crap4rust, and turn functions scoring above 16 into GitHub issues. Use when asked for a CRAP report, risky or under-tested Rust code, complexity hotspots, or "what should we test or split next".
---

# CRAP report for Hush

`CRAP = CC² × (1 − coverage)³ + CC` per Rust function: high when a function is
both complex and under-tested. The tool is `cargo crap4rs` from
[crap4rust](../../../../crap4rust) (`E:\Projects\crap4rust`). It runs the tests
under `cargo llvm-cov`, reads the LCOV line coverage and scores every function in
`src-tauri/src` (tests and `#[cfg(test)]` code are skipped).

**Threshold: 16.** Anything above it gets an issue.

## Prerequisites (once per machine)

```powershell
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov
cargo install --path E:\Projects\crap4rust     # again whenever crap4rust changes
```

## Run

From the repo root in **PowerShell** (Git Bash's `perl` breaks the OpenSSL build):

```powershell
cargo crap4rs --manifest-path src-tauri/Cargo.toml --threshold 16 > crap-report.txt
```

- The first run is a full instrumented rebuild into `src-tauri/target/llvm-cov-target` (about 12 GB, several minutes). Later runs are incremental.
- Test output goes to stderr; stdout (`crap-report.txt`) is only the report. The full list is also written to `src-tauri/.metrics/crap.json` (name, module, file, line, complexity, coverage, crap).
- Exit status 2 means something scored above 16. That's the expected case here, not a failure. Status 1 is a real error (build or coverage failed).
- To look at one area again without re-running the tests: `cargo crap4rs --manifest-path src-tauri/Cargo.toml --use-existing-coverage db import`.

**Until #205 is fixed** (`no_plaintext_on_disk` fails on Windows), a plain run fails. Use this instead, which writes the report even when a test fails:

```powershell
cargo crap4rs --manifest-path src-tauri/Cargo.toml --threshold 16 -- --ignore-run-fail > crap-report.txt
```

Once #205 is closed, check whether the plain command works and remove this section.

## Reading the report

- **Cov% 0.0**: compiled, never run by a test. Includes Tauri `#[tauri::command]` wrappers and code reached only by `#[ignore]`d tests (real model, GPU, benchmarks). Those don't run here, so their code shows as uncovered.
- **N/A**: not compiled on this platform (the `cfg(unix)` / `cfg(not(windows))` variants). Ignore these.
- **CC ≥ 17**: scores above 16 *even at 100% coverage*. More tests can't fix it; only splitting the function can.
- Control flow inside macros (`assert!`, `matches!`, `format!` arguments) isn't counted.

## Turning findings into issues

One issue per group, not per function:

1. **Prerequisites first.** If the run needed workarounds (a test target that won't build, a failing test), file or reuse one issue per workaround. Other CRAP issues say `Depends on #<n>` only when they genuinely need it, for example when the test they ask for would live in a target that doesn't build.
2. **"Add tests" issues**: functions above 16 whose CC is 16 or less, so tests alone can bring them under. Group them by area (module prefix: `commands`, `embed`, `import`, `db`, …), one issue per area.
3. **"Split" issues**: functions with CC ≥ 17. One issue per function, because each split is its own design job.

Before filing, `gh issue list --state open --search "CRAP in:title"` and update an existing issue rather than file a duplicate. Close CRAP issues whose functions are now at 16 or below.

**Title**: `CRAP: add tests for <area> (<n> functions)` or `CRAP: split <function> (CC <cc>)`.

**Body**, in the style of the other issues:

```markdown
## Problem

<One paragraph: what the report found and why it matters (complex code that tests don't cover is the riskiest to change).>

Measured with crap4rust at <short commit sha>, threshold 16:

| Function | File | CC | Coverage | CRAP |
|---|---|---|---|---|
| `Vault::open` | `src-tauri/src/db/mod.rs:120` | 9 | 0.0% | 90.0 |

## What to do

<For add-tests: what each function does and which behaviours a test should pin down (read the code first; name the branches). Note which ones are Tauri command wrappers, where the better move may be to test the function they call and keep the wrapper trivial.>
<For split: the parts the function does today and a suggested seam. The target is CC ≤ 16 for every resulting function at its current coverage.>

## Done when

- `cargo crap4rs --manifest-path src-tauri/Cargo.toml --use-existing-coverage <module filter>` after a fresh run shows every listed function at or below 16.
```

No labels unless the user asks for them. List the filed issue numbers to the user at the end.
