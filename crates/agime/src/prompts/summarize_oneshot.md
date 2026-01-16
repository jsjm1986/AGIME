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

## Output Format

Generate a structured summary with these sections:

### 1. User Goals (用户目标)
- List all user requests chronologically

### 2. Completed Work (已完成)
- What has been done, with file paths

### 3. Pending Tasks (待完成)
- Unfinished work, blocked items

### 4. Key Code Changes (代码变更)
- File paths + what changed (preserve actual code if critical)

### 5. Important Decisions (重要决策)
- Choices made and why

### 6. Errors & Solutions (错误与解决)
- Issues encountered and how resolved

### 7. Current State (当前状态)
- Where the conversation left off

> Remember: This summary is for yourself (the AI) to continue working. Be thorough rather than brief.

