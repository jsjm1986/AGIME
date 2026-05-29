# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Repository Layout

AGIME is a Rust + Electron monorepo. Workspace crates live under `crates/`; the desktop frontend lives under `ui/desktop/`; an additional React admin panel is embedded inside `crates/agime-team-server/web-admin/`.

```
crates/
  agime/             core library — agent loop, providers, extensions, prompts, MCP plumbing
  agime-mcp/         built-in MCP servers (developer, computer-controller, memory, tutorial, language parsing via tree-sitter)
  agime-cli/         agime.exe — interactive CLI; depends on agime + agime-mcp + agime-bench
  agime-server/      agimed.exe — HTTP/SSE backend that the Electron app talks to; also produces generate_schema.exe
  agime-team-server/ team collaboration backend (MongoDB + SQLite, Axum HTTP, embedded React admin)
  agime-team/        team-server domain logic
  agime-bench/       benchmarking framework (registry-based eval suites)
  agime-test/        capture.exe — record/playback harness for MCP stdio interactions
ui/desktop/          Electron + React 19 + Vite 7 client; ships generated OpenAPI client under src/api/
```

Dependency rule of thumb (see `BUILD-GUIDE.md` for full matrix):
`agime` ← `agime-mcp`, `agime-bench` ← `agime-cli`, `agime-server` (which both also depend on `agime` and `agime-mcp`).

## Where new functionality goes (important)

This is the project's canonical workflow — non-trivial features should follow it:

1. Implement core logic in the `agime` crate.
2. Wire CLI surface in `agime-cli` (commands live under `crates/agime-cli/src/commands/`).
3. For desktop-visible features, add an Axum route under `crates/agime-server/src/routes/` and register it in `routes/mod.rs`.
4. Regenerate the OpenAPI schema and TypeScript client; the Electron app calls it through `ui/desktop/src/api/`.
5. For Team Server features, the equivalent server is `crates/agime-team-server/` with its own admin UI under `web-admin/`.

The schema regeneration step in `.goosehints` references `just generate-openapi`, but **there is no justfile in the repo today**. The actual generator binary is `agime-server`'s `generate_schema` bin (`cargo run -p agime-server --bin generate_schema`); CI verifies it via `scripts/check-openapi-schema.sh`, which diffs `ui/desktop/openapi.json` and `ui/desktop/src/api/`. On the desktop side, `npm run generate-api` (in `ui/desktop/`) runs `openapi-ts` against the committed `openapi.json` to regenerate `src/api/`.

## Common Commands

### Rust (run from repo root)

```bash
# Format / lint / test (matches CI)
cargo fmt --check
cargo fmt                              # auto-format
./scripts/clippy-lint.sh               # full lint (strict + baseline); --fix to auto-fix
cargo clippy --all-targets --jobs 2 -- -D warnings

cargo test                             # full test suite
cargo test -p agime                    # tests for one crate
cargo test -p agime some_test_name     # single test by name
cargo test -- --skip scenario_tests::scenarios::tests    # CI runs scenario tests serially
cargo test --jobs 1 scenario_tests::scenarios::tests     # ...then runs them with -j1

# Build (see BUILD-GUIDE.md for cross-compile details)
cargo build --release --workspace -j 4
cargo build --release -p agime-cli -j 4
cargo build --release -p agime-server -j 4
cargo build --release -p agime-team-server -j 4
```

The Rust toolchain is pinned by `rust-toolchain.toml` (currently 1.92.0). `rustls` is configured workspace-wide to use the `ring` backend (no C compiler needed) — don't introduce features that pull in `aws-lc-rs`. `scripts/check-no-native-tls.sh` enforces no `native-tls` deps.

### Electron desktop (`ui/desktop/`)

```bash
npm ci                       # CI uses ci, not install
npm run lint:check           # typecheck + ESLint (CI gate, --max-warnings 25)
npm run lint                 # auto-fix
npm run typecheck            # tsc --noEmit only
npm run test:run             # vitest run (CI gate)
npm run test                 # vitest watch
npm run generate-api         # regenerate src/api/ from openapi.json
npm run start-gui            # generate-api + electron-forge start
npm run build:web            # web build (vite.config.web.mts)
npm run test-e2e             # playwright (requires generate-api)
```

`npm run start` in `package.json` references `just run-ui`, which doesn't exist in this repo — use `npm run start-gui` instead.

### Team-server admin UI (`crates/agime-team-server/web-admin/`)

```bash
npm run dev          # vite dev server
npm run typecheck
npm run build        # also runs check:external-runtime + check:i18n-targeted
```

## Architecture Notes

**Agent loop (`agime`).** The core agent runs an LLM turn loop with tool calls; provider abstraction lives in `crates/agime/src/providers/` (OpenAI-compatible format under `providers/formats/openai.rs`). System prompt in `crates/agime/src/prompts/system.md`. Harness flow is in `crates/agime/src/agents/harness/` (notably `provider_turn.rs`, `finalize.rs`).

**Extensions / MCP.** Three extension transports are supported: stdio (local subprocess), remote HTTP, streamable HTTP. Built-in MCP servers ship inside `agime-mcp` and are launched in-process. The MCP protocol library is `rmcp 0.15` (workspace-pinned in root `Cargo.toml`).

**HTTP server (`agime-server`).** Axum-based, SSE for streaming chat replies (`routes/reply.rs`). State is held in `state.rs` (`AppState` — `agent_manager`, recipe cache, session tracking). Auth middleware in `auth.rs`. Has an optional `team` cargo feature that pulls in SQLite via sqlx and integrates with `agime-team`.

**Team server (`agime-team-server`).** Separate Axum binary with MongoDB (preferred) and SQLite back-ends. The bulk of the surface area is under `crates/agime-team-server/src/agent/` (chat executor, capability policy, document tools, skill registry, workspace service, etc.). Web admin is a separate Vite+React app served from `web-admin/`.

**Sessions.** Session ID + persistent message store is shared across CLI and server. Lead/worker provider mode and provider fallbacks are supported.

**Benchmarking (`agime-bench`).** Registry pattern — evaluations register via `eval_suites/factory.rs` and implement the `Evaluation` trait. Runners under `runners/` orchestrate suites; metric aggregation in `metric_aggregator.rs`. Suites split into `core/` and `vibes/`.

**Test capture (`agime-test`).** `capture.exe` records and replays MCP stdio interactions to JSON for deterministic testing.

## Project Conventions

From `.github/copilot-instructions.md` and the codebase:

- **Errors:** use `anyhow::Result`; avoid `unwrap()` in production code. Don't add `.context("Failed to do X")` when the inner error already says it failed.
- **Async:** tokio everywhere; don't block in async contexts.
- **Comments:** the codebase prefers fewer comments and less logging, not more — don't add comments that just restate the code, and don't add logging unless it's for errors or security events.
- **Don't add defensive checks** for things that can't happen, and don't make fields `Option<bool>` when they should be `bool` defaulting to false.
- **MCP code paths warrant extra scrutiny** — bugs there have wide blast radius.

## CI Gates (must pass before pushing)

From `.github/workflows/ci.yml`:

- `cargo fmt --check`
- `cargo test` (with the scenario_tests two-phase split shown above), run from `crates/`
- `./scripts/clippy-lint.sh` (run after `source ./bin/activate-hermit` and uninstalling rustup, per CI — locally just run the script directly)
- `scripts/check-openapi-schema.sh` — fails if `ui/desktop/openapi.json` or `ui/desktop/src/api/` are stale relative to the Rust types
- In `ui/desktop/`: `npm ci`, `npm run lint:check`, `npm run test:run`

If you change Axum route types or shared DTOs, regenerate the OpenAPI schema and the TS client and commit both — CI will block the PR otherwise.

## Workspace-specific quirks

- The `crunchy` patch for Windows cross-compilation is currently **disabled** in `Cargo.toml` (commented out). Re-enable only if doing cross-compile work — see comment block in the workspace `Cargo.toml`.
- `rust-analyzer.toml` exists at the root — respect its settings when configuring tooling.
- Husky hooks are installed via `ui/desktop/`'s `prepare` script; if `npm install` is skipped, hooks won't fire locally but CI will still catch issues.
- The repo has `node_modules/` at the **root** as well as in `ui/desktop/` and `crates/agime-team-server/web-admin/` — don't be surprised by it.

## Desktop harness env knobs

The desktop chat path (`agime-server` with the default `desktop_harness_host` feature) routes turns through core `run_harness_host`. Two env vars tune coordinator delegation; both are read in `crates/agime-server/src/desktop_harness_host.rs` and default to a modest fan-out so sub-agent/swarm delegation is enabled out of the box:

- `AGIME_DESKTOP_PARALLELISM_BUDGET` (default `2`) — per-turn cap on concurrently running coordinator workers.
- `AGIME_DESKTOP_SWARM_BUDGET` (default `2`) — per-turn cap on swarm workers.

Set either to `0` to disable that form of delegation (reverts to the pre-`v2` `None` behavior). Unparseable values fall back to the default, so a typo can't silently disable the capability. The budget is only a ceiling — the core coordinator/planner still decides *whether* to delegate based on the turn's content, so simple single-shot chats won't spawn workers.
