## Task Context
- An llm context limit was reached when a user was in a working session with an agent (you)
- Generate a structured summary of the conversation history below
- This summary will be used to let the user continue the working session seamlessly

**Conversation History (Middle Section Only):**
{{ messages }}

## ⚠️ Critical Rules (MUST FOLLOW)

**Conservative Principle**: If unsure whether information is important, KEEP IT.

### MUST PRESERVE (Never Compress)
1. All user explicit requests and goals
2. All code changes and file modifications (keep diffs/full code)
3. All unresolved errors and TODO items
4. All decision points and chosen solutions
5. File paths being actively worked on

### SAFE TO COMPRESS
1. Long tool outputs → Extract key results only
2. Intermediate reasoning → Keep conclusions only
3. Repeated confirmations → Remove entirely
4. Failed attempts → Keep failure reason only, remove details

## Output Format (Preferred: Strict JSON)

Return valid JSON with this shape:

{
  "user_goals": ["..."],
  "completed_work": ["..."],
  "pending_tasks": ["..."],
  "key_code_changes": [
    {"file": "path/to/file", "changes": "..."}
  ],
  "important_decisions": ["..."],
  "errors_and_solutions": [
    {"error": "...", "solution": "...", "status": "resolved|unresolved"}
  ],
  "current_state": "...",
  "next_action": "..."
}

JSON rules:
- Keep field names exactly as above.
- Keep arrays even if empty.
- Preserve concrete file paths and unresolved blockers.
- Keep concise but complete enough for autonomous continuation.

If you cannot produce valid JSON, output Markdown using these exact headings:
1. User Goals
2. Completed Work
3. Pending Tasks
4. Key Code Changes
5. Important Decisions
6. Errors and Solutions
7. Current State
8. Next Action

Remember: This summary is for yourself (the AI) to continue working. Be thorough rather than brief.
