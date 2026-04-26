## Task Context
An AGIME agent reached its context limit while executing a user task. Generate a durable continuation summary from the conversation history below. The next agent will rely on this summary as the source of truth for continuing the same work.

**Conversation History (Middle Section Only):**
{{ messages }}

## Critical Rules

- Produce summary text only. Do not ask questions, call tools, or mention that you are compacting context.
- Preserve every explicit user request, constraint, correction, and success criterion.
- Preserve concrete file paths, APIs, data records, command results, test outcomes, deployed services, and artifact names that affect continuation.
- Preserve errors, failed attempts, root causes, fixes already tried, and whether each issue is resolved.
- Preserve current working state exactly: what is finished, what is partially done, what is blocked, and what should happen next.
- If unsure whether a detail matters, keep it.

## What To Compress

- Long tool outputs: keep exact command name, key lines, error messages, and final result only.
- Repeated confirmations or status chatter: omit unless they changed the task direction.
- Internal reasoning: keep decisions, tradeoffs, and conclusions, not the full reasoning transcript.
- Provider/runtime noise: keep only user-visible impact and actionable diagnostics.

## Required Coverage

Your summary must cover:

1. User intent and active goals.
2. Important constraints and preferences from the user.
3. Files, artifacts, documents, APIs, database records, or services involved.
4. Completed work and verified results.
5. Code/config/runtime changes made or still pending.
6. Errors encountered, root causes, attempted fixes, and current status.
7. Decisions and architecture/methodology choices.
8. Current state at the exact handoff point.
9. Next best action with enough detail for autonomous continuation.

## Output Format

Return valid JSON with this exact shape:

{
  "user_goals": ["..."],
  "constraints_and_preferences": ["..."],
  "completed_work": ["..."],
  "pending_tasks": ["..."],
  "files_and_artifacts": [
    {"path_or_name": "...", "relevance": "..."}
  ],
  "key_code_changes": [
    {"file": "path/to/file", "changes": "..."}
  ],
  "important_decisions": ["..."],
  "errors_and_solutions": [
    {"error": "...", "root_cause": "...", "solution": "...", "status": "resolved|unresolved|unknown"}
  ],
  "current_state": "...",
  "next_action": "..."
}

JSON rules:
- Keep field names exactly as above.
- Keep arrays even if empty.
- Use concise but information-dense strings.
- Do not invent paths, statuses, or results.
- Do not replace specific file paths or command outputs with vague descriptions.

If valid JSON is impossible, output Markdown using these exact headings:
1. User Goals
2. Constraints and Preferences
3. Completed Work
4. Pending Tasks
5. Files and Artifacts
6. Key Code Changes
7. Important Decisions
8. Errors and Solutions
9. Current State
10. Next Action
