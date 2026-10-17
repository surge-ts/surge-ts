## Comment Policy

- Do not add obvious, redundant, or filler comments.
- Avoid comments that merely repeat what the code already says.
- Prefer clear names, small functions, and straightforward control flow over explanatory comments.
- Add comments only when they explain non-obvious intent, edge cases, compatibility constraints, performance tradeoffs, or safety-sensitive behavior.
- Do not add large header comments, decorative section comments, or generated-looking comment blocks unless explicitly requested.
- When modifying existing files, do not increase comment noise. Remove stale or misleading comments if they are directly related to the edited code.
- Comments should justify themselves; if a comment does not explain why the code exists or why it is written that way, do not add it.

## Search

This repository is indexed by [graft](graft/INDEX.md): a prebuilt graph of every
symbol, its exact `file:line` span, and who calls what. Reach for it *before*
grepping or reading source files — one call usually replaces several file reads,
and the graph refreshes before each query, so it reflects uncommitted edits.

Pick the one tool that fits and act on its answer; most tasks need a single
call. Do not re-ask the same question reworded.

- `graft ask "<task>" --source` — locate + understand. Ranked nodes with the
  code inlined at each `file:line` (the crux; `--full` for the whole span).
  The default for "how does X work" / "where does Y live".
- `graft grep "<literal>"` — exhaustive find, grouped by enclosing symbol. Use
  when you need *every* occurrence (`ask` is ranked top-N and will miss some).
- `graft skeleton <file>` — a file's whole API in ~200 tokens, every signature
  plus span. Far cheaper than reading a 2,000-line checker module.
- `graft callers <sym> [--direction out] [--depth N|all]` — exact edges. Run it
  before changing or renaming a symbol; `--depth all` maps the blast radius.
  Editing the primary file and stopping is the classic miss in this workspace,
  where a checker change usually has siblings in `program/`, `infer/`, `flow/`,
  and the crate's `tests/`.
- `graft map` — orientation only, for an unfamiliar corner of the tree.

Scope to one crate with `--in crates/<crate>/`.

Fall back to plain text search when graft cannot answer — non-code files
(fixtures, `oracle/` expectations, `Cargo.toml`, docs), or a literal that is not
a symbol:

- Prefer `rg` (ripgrep) over `grep` for such searches. It respects `.gitignore`, is faster, and handles binary files safely.
- Fall back to `grep` only when `rg` is unavailable or a POSIX-compatible invocation is strictly required (e.g. inside a shell script that must be portable).

## Verification

- Behaviour tests (checker diagnostics, CLI): vitest against the release
  binary, comparing surge with tsc 7.0.2 live — see [tests/README.md](tests/README.md).

  ```sh
  pnpm test          # release build, then vitest run
  pnpm test:vitest   # vitest run against the existing target/release/surge
  ```

  Scope with a path (`pnpm test:vitest tests/smoke.test.ts`) or a name filter
  (`-t <pattern>`). New checker behaviour tests are fixture files under
  `tests/checker/<suite>/`, not Rust `#[test]`s.
- Rust unit tests (crate internals): `cargo test --workspace`, or `-p <crate>`
  with a name filter. A unit test that sets environment gates or reads the
  process-global counters must isolate itself with
  `test_support::run_in_own_process` (surge-ts-checker); `cargo test` shares one
  process across tests.
- Oracle harness tests: `pnpm run oracle:test`.
- Single-target oracle check: `pnpm run oracle:compare -- --project <preset|tsconfig>`
  (or `--file <source.ts>`) to spot-check one fixture or project.
- Oracle compatibility sweep: after changes that can affect diagnostics, run
  `pnpm run oracle:sweep -- --all --maxDiagnostics 200` (or a targeted
  `pnpm run oracle:sweep -- --filter <group> --maxDiagnostics 200`, or
  `--discover <dir>` for projects outside the preset registry). A target fails
  the gate only on diagnostic code-count or file/code/line mismatch;
  message-text and span/column drift are reported but non-gating unless you pass
  `--strictMessages` / `--strictSpans`.
- Optional — benchmark harness tests: `pnpm run bench:test` (run when touching
  the benchmark harness).
- Do not edit fixtures, expected output, or checker semantics to make the sweep
  pass; report real regressions honestly instead.

## Memory-Lifetime Rules

Background: `crates/surge-ts-checker/MEMORY_REGIONS.md` ("Memory-lifetime
program") and `docs/perf/MEMORY-OPTIMIZATION-REPORT.md`.

- MUST NOT retain canonical type-store payloads strongly without measured
  justification; the stores use `Weak` retention with monotonic, never-reused
  IDs.
- MUST NOT capture declaration span maps, value tables, diagnostics, flow
  state, or checker context in type declaration environments; environments
  hold stamp-deduplicated `Arc` table snapshots only.
- MUST NOT prune or shorten expansion-cache lifetimes (including
  `program_instantiations`) without full oracle evidence; cache lifetime is
  semantically load-bearing and pruning has measurably drifted zod
  diagnostics. Only true-death reclamation is approved.
- MUST NOT share resolution results keyed only on declaration identity when
  the result can depend on analysis pass, lexical/module/type-parameter
  scope, import or augmentation generation, recursion state, or resolution
  mode.
- MUST NOT use `git stash` for large cross-cutting memory work; commit and
  validate each memory stage independently.

## Documentation rules

Two failure modes have repeatedly made this repository's docs misleading: a
stale number presented as current, and a historical note read as a support
statement. These rules exist to prevent both.

- **[CURRENT_STATUS.md](CURRENT_STATUS.md) is the single source of truth for
  current state.** Volatile values — oracle preset count, workspace test count,
  real-project parity, benchmark medians — are recorded there and **nowhere
  else**. Other documents link to it rather than copying the number.
- **MUST NOT record a measured number you did not measure.** If you are
  carrying forward someone else's result, say so explicitly and cite the source
  document, commit, and date. "Last recorded" is an acceptable claim;
  presenting it as current is not.
- **MUST NOT let a skipped gate read as a passing gate.** The real-project
  gates skip when the checkout or the `typescript` package is absent. Record
  *not measured*, not the previous number.
- **MUST measure from a clean checkout.** Do not record gate results or
  benchmark numbers produced by a dirty working tree; build from a clean
  worktree at the commit you are citing and point the harness at it with
  `SURGE_TS_BIN`.
- **MUST label historical content unmistakably.** Point-in-time optimization
  reports belong under [docs/perf/](docs/perf/), and any other historical note
  under an explicit `Historical` heading, with a banner saying it does not
  describe current behavior. Version-tagged milestone notes and superseded
  support lists are not kept at all; git history has them. Do not delete engineering investigations —
  a rejected design with its measurement attached is what stops the idea being
  re-proposed.
- **MUST NOT carry a limitation forward unverified.** Before repeating a
  "not supported" claim, reproduce it against the oracle. Several long-standing
  entries turned out to have been fixed. When a limitation is confirmed or
  refuted, record the date and the commit alongside the verdict.
- **`v0.x` / `v1.x` labels are internal milestone markers**, not releases,
  tags, or crate versions. Do not synchronize them with the Cargo workspace
  version. See [CURRENT_STATUS.md § Versioning](CURRENT_STATUS.md#versioning).
- **[PUBLIC_API.md](PUBLIC_API.md) is a contract, not a status board.** It
  lists the stable API and the exact-parity feature areas; it carries no
  volatile counts.
- **Never generalize a fixture result into a compatibility claim.** A green
  preset means parity on that fixture. Real-project 0/0 corpora are
  false-positive regression gates. `trpc` is a measured workload, never a
  parity claim.

## Performance and correctness guardrails

See [ARCHITECTURE.md](ARCHITECTURE.md) and
[docs/PERFORMANCE_INVARIANTS.md](docs/PERFORMANCE_INVARIANTS.md) for rationale.

- MUST NOT introduce any pattern in the "Prohibited patterns" list of
  docs/PERFORMANCE_INVARIANTS.md (deep-cloned `CheckerOptions`, per-file
  `CheckerContext` clones, `getenv` in hot loops, uninterned persistent type
  payloads, consumer-local lookup before dependency lexical scope, …).
- MUST NOT add environment-insensitive cross-pass or cross-module caches;
  cache keys must capture declaration, arguments, and environment identity,
  and preliminary-pass results must never install first-wins global state.
- MUST NOT cache degraded (`had_error`) results, fallback `Unknown`, or
  recursion-in-progress results program-wide; overload order and duplicates
  must be preserved exactly.
- REQUIRES BENCHMARK: changes to `crates/surge-ts-types/src/store.rs`,
  `crates/surge-ts-checker/src/context/`, hashing (`fx.rs`, hasher choices),
  caching, or canonicalization need an interleaved before/after benchmark on a
  real project (`pnpm real:trpc` style; single runs are noise — see
  MEMORY_REGIONS.md) plus the full oracle sweep.
- REQUIRES ORACLE PARITY: any change that can affect emitted diagnostics needs
  `pnpm run oracle:sweep -- --all --maxDiagnostics 200` before landing.
- New per-file checker state MUST join the `begin_file_check` reset; new
  program-lifetime caches MUST join the end-of-run teardown
  (`clear_program_type_caches`).
