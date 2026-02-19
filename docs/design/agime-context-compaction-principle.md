# AGIME Context Compaction Principle

This document explains how context compaction works in local desktop AGIME and why a session can require a new chat after repeated compactions.

## 1. Two compaction entry points

There are two distinct compaction paths in runtime:

1. Pre-reply compaction in `Agent::reply(...)`
- File: `crates/agime/src/agents/agent.rs`
- Auto compaction is checked before normal reply generation.
- Manual compaction is triggered by fixed phrases.

2. Recovery compaction in `Agent::reply_internal(...)`
- File: `crates/agime/src/agents/agent.rs`
- If provider returns `ContextLengthExceeded` while generating a reply, the agent compacts and retries.

## 2. Trigger conditions

### Auto compaction
- Checked by `check_if_compaction_needed(...)` in `crates/agime/src/context_mgmt/mod.rs`.
- Default threshold is `DEFAULT_COMPACTION_THRESHOLD = 0.8` (80% context usage).
- Runtime config key: `AGIME_AUTO_COMPACT_THRESHOLD`.
- If threshold is `<= 0.0` or `>= 1.0`, auto compaction is disabled.

### Manual compaction
- Trigger phrases (backend): `["Please compact this conversation", "/compact", "/summarize"]`.
- File: `crates/agime/src/agents/agent.rs`.
- Desktop frontend button sends `"Please compact this conversation"`.
- File: `ui/desktop/src/components/ChatInput.tsx`.

## 3. Compaction algorithm (segmented strategy)

Implemented in `compact_messages(...)` at `crates/agime/src/context_mgmt/mod.rs`.

### Segmentation constants
- `KEEP_FIRST_MESSAGES = 3`
- `KEEP_LAST_MESSAGES = 10`
- `MIN_MESSAGES_TO_COMPACT = 20` (for non-manual compaction)

### What the algorithm does
1. Keep the first `N` messages unchanged (anchor context).
2. Keep the last `M` messages unchanged (recent context).
3. Summarize the middle section using `do_compact(...)`.
4. Mark original middle messages as `agent_invisible` (still visible in history UI, hidden from model context).
5. Insert summary message as `agent_only`.
6. Insert a continuation instruction as `agent_only`.
7. Rebuild conversation and replace session history.

## 4. Summarization fallback behavior

`do_compact(...)` progressively removes tool-response-heavy middle content if summarization itself still overflows context:

- Removal attempts: `[0, 10, 20, 50, 100]` percent of tool responses.
- If all attempts still hit `ContextLengthExceeded`, compaction fails with error.

This avoids immediate hard failure when tool outputs dominate token usage.

## 5. Why "3 compactions then new chat"

This is an explicit guard in recovery compaction flow:

- `MAX_COMPACTION_ATTEMPTS = 3`
- File: `crates/agime/src/agents/agent.rs`
- In `reply_internal(...)`, on each `ProviderError::ContextLengthExceeded`, agent:
  - increments `compaction_count`
  - compacts and retries
- If `compaction_count >= MAX_COMPACTION_ATTEMPTS`, agent emits:
  - "Reached maximum compaction attempts. Please start a new session to continue."
  - and exits the loop.

So the "3 times then restart" behavior is by design (anti-infinite-loop safety), not random model behavior.

## 6. Practical implications

If repeated recovery compaction happens in one reply cycle, common causes are:

1. Very large recent messages/tool outputs remain in the preserved tail.
2. Current task inherently requires oversized context each turn.
3. Summarized context plus required new context still exceeds model limit.
4. Aggressive tool usage produces large non-compressible working context.

## 7. Tuning options

1. Raise context limit model (if available).
2. Lower workload per turn (smaller requests, fewer giant tool outputs).
3. Adjust `AGIME_AUTO_COMPACT_THRESHOLD` to compact earlier.
4. Start a fresh session for new large phases to avoid hitting the 3-attempt recovery cap.

## 8. Strategy switch (legacy vs CFPM)

Runtime now supports strategy switching via config key:

- `AGIME_CONTEXT_COMPACTION_STRATEGY = "legacy_segmented"` (default)
- `AGIME_CONTEXT_COMPACTION_STRATEGY = "cfpm_memory_v1"`

`legacy_segmented` keeps the previous summarize-middle behavior.

`cfpm_memory_v1` builds a deterministic memory checkpoint from stable facts (goals, verified successful actions, key paths), suppresses transient error noise, and then applies visibility-based context reduction. This path is intended to reduce repeated re-discovery and avoid depending on model-generated summarization for each compaction.

## 9. CFPM runtime memory lifecycle (latest)

To avoid stale/noisy memory reducing model quality, CFPM now has explicit lifecycle controls:

1. Per-turn refresh
- On each turn, recent assistant/tool messages are parsed into `MemoryFactDraft`.
- Accepted drafts are merged into `memory_facts` (source: `cfpm_auto`).

2. Candidate audit trail
- Every draft decision is persisted into `memory_candidates` with:
  - `decision`: `accepted` / `rejected`
  - `reason`: deterministic rejection code (for observability and debugging)

3. Automatic prune when no new drafts
- Even if a turn yields no new draft, backend performs a CFPM auto-fact prune pass.
- It removes invalid or low-value active `cfpm_auto` facts (for example date-only artifacts like `2024/7/19`) and deduplicates to cap memory growth.

4. Runtime injection guard
- Before model generation, only active/pinned facts are injected.
- Artifact injection is additionally filtered to path-like, non-date content to avoid re-injecting log/date noise.

5. Runtime visibility for conversation stream
- Config key: `AGIME_CFPM_RUNTIME_VISIBILITY`
- Values:
  - `off`: no CFPM runtime inline notification in chat
  - `brief` (default): one summary line per checkpoint
  - `debug`: detailed line including mode/reason and rejected reason breakdown
- Runtime message protocol:
  - Inline message prefix: `[CFPM_RUNTIME_V1]`
  - Suffix payload: JSON (`verbosity`, `acceptedCount`, `rejectedCount`, `prunedCount`, `factCount`, etc.)
  - Frontend parses this payload for localized rendering and optional debug detail expansion.
- This visibility channel is a UI/runtime observability stream and is not used as model tool input.

## 10. Pre-tool memory gate (command rewrite)

To reduce repeated "path probing" loops (for example repeatedly discovering Desktop path), runtime now includes a deterministic pre-tool rewrite stage in `Agent::dispatch_tool_call(...)`:

1. Scope
- Only active for `AGIME_CONTEXT_COMPACTION_STRATEGY=cfpm_memory_v1`.
- Controlled by `AGIME_CFPM_PRE_TOOL_GATE` (`on` by default, `off/false/0` disables).
- Applies only to shell-like tools and only when command contains profile-based folder probes.

2. What it rewrites
- Rewrites probe patterns such as:
  - `$env:USERPROFILE/Desktop`
  - `%USERPROFILE%\\Desktop`
  - `[Environment]::GetFolderPath('Desktop')`
- Same logic is applied for `Desktop`, `Documents`, and `Downloads`.
- Runtime message protocol:
  - Inline message prefix: `[CFPM_TOOL_GATE_V1]`
  - JSON payload includes `tool`, `target`, `path`, `originalCommand`, `rewrittenCommand`.

3. Rewrite source
- Uses active/pinned CFPM `memory_facts` artifact/path items as authoritative known folder paths.
- If a known folder path exists, probe tokens are replaced with that path before actual tool execution.

4. Safety guards
- Does not run when command already contains explicit absolute paths.
- Does not run if latest user message explicitly asks for re-verification.
- If memory lookup fails or no match exists, tool call executes unchanged.

This gate is designed to avoid redundant trial-and-error while preserving real task execution (rewrite-and-run, not hard blocking).
