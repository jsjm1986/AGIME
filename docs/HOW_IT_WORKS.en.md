# How AGIME Works

> This document explains how AGIME works in simple terms to help you better understand and use it.

## Table of Contents

- [Simple Understanding](#simple-understanding)
- [How Conversations Are Processed](#how-conversations-are-processed)
- [How AI Executes Tasks](#how-ai-executes-tasks)
- [Memory System](#memory-system)
- [Recipes and Automation](#recipes-and-automation)
- [Security Mechanisms](#security-mechanisms)

---

## Simple Understanding

Think of AGIME as a smart assistant you've hired:

```
┌─────────────────────────────────────────────────────┐
│                                                     │
│   You ──────► AGIME ──────► Your Computer          │
│  (speak)    (understand)     (execute)             │
│                  │                                  │
│                  ▼                                  │
│              AI Brain                               │
│          (cloud or local)                           │
│                                                     │
└─────────────────────────────────────────────────────┘
```

**Workflow**:
1. **You speak** - Tell AGIME what you want in natural language
2. **It understands** - AGIME sends your words to an AI model to understand intent
3. **It plans** - AI thinks about what steps are needed to complete the task
4. **It executes** - AGIME performs these steps on your computer
5. **It reports** - Tells you the results

---

## How Conversations Are Processed

### Step 1: You Send a Message

When you type and send in the input box:

```
You: Help me organize the photos on my desktop into folders by date
```

### Step 2: Building Context

AGIME packages the following information to send to the AI:

```
┌─────────────────────────────────────────┐
│ Content Sent to AI                       │
├─────────────────────────────────────────┤
│ 1. System settings (AI's role)           │
│ 2. Available tools (what AI can call)    │
│ 3. Chat history (previous conversation)  │
│ 4. Your memories (preferences & habits)  │
│ 5. What you just said                    │
└─────────────────────────────────────────┘
```

### Step 3: AI Thinks and Responds

After receiving the information, the AI model thinks and responds:

```
AI's thought process:
1. User wants to organize desktop photos
2. Need to first see what photos are on desktop
3. Then get the date of each photo
4. Finally create folders by date and move files

AI's response:
"Okay, let me help you organize. First let me see what photos
are on your desktop..."
[Call tool: list desktop files]
```

### Step 4: Execute Tools

When AI decides to use a tool, AGIME will:

```
┌─────────────────────────────────────────┐
│         Tool Execution Flow              │
├─────────────────────────────────────────┤
│                                         │
│  AI request: List desktop files          │
│      │                                  │
│      ▼                                  │
│  AGIME checks permissions                │
│      │                                  │
│      ▼                                  │
│  [Smart mode] Read operation, auto-allow │
│      │                                  │
│      ▼                                  │
│  Execute: Read ~/Desktop directory       │
│      │                                  │
│      ▼                                  │
│  Return results to AI                    │
│                                         │
└─────────────────────────────────────────┘
```

### Step 5: Loop Until Complete

After getting tool results, AI continues thinking about next steps until done:

```
Loop 1: List files → Found 50 photos
Loop 2: Read photo dates → Got date information
Loop 3: Create folders → Created 6 folders by month
Loop 4: Move files → Moved photos to corresponding folders
Done: "Finished! 50 photos organized into 6 folders by month."
```

---

## How AI Executes Tasks

### Tool System

AGIME lets AI operate your computer through "tools":

```
┌─────────────────────────────────────────────────────┐
│                  Available Tools                     │
├─────────────────────────────────────────────────────┤
│                                                     │
│  📁 File Operations   🌐 Network        💻 System   │
│  ├─ Read files        ├─ Browse web     ├─ Run apps │
│  ├─ Write files       ├─ Download       ├─ Commands │
│  ├─ Move files        └─ API requests   └─ Screenshot│
│  └─ Delete files                                    │
│                                                     │
│  🔧 Dev Tools         📊 Data Process   🧠 Memory   │
│  ├─ Code analysis     ├─ Spreadsheets   ├─ Save     │
│  ├─ Run scripts       ├─ Charts         └─ Recall   │
│  └─ Git operations    └─ PDF handling               │
│                                                     │
└─────────────────────────────────────────────────────┘
```

### Tool Call Example

When you say "Open my weekly report":

```
You: Open my weekly report

AI thinks: User wants to open weekly report, I need to:
1. First find the document
2. Then open it with default program

AI action:
┌────────────────────────────────────────┐
│ Tool call 1: search_files              │
│ Args: pattern="*weekly*report*"        │
│ Result: Found ~/Documents/report.docx  │
├────────────────────────────────────────┤
│ Tool call 2: open_file                 │
│ Args: path="~/Documents/report.docx"   │
│ Result: Opened                         │
└────────────────────────────────────────┘

AI: I've opened ~/Documents/report.docx for you
```

---

## Memory System

### Why Memory Matters

Regular AI conversations are "stateless"—each time it doesn't remember who you are. AGIME's memory system changes this.

### Memory Types

```
┌─────────────────────────────────────────────────────┐
│                   Memory System                      │
├─────────────────────────────────────────────────────┤
│                                                     │
│  📝 Short-term Memory (within session)              │
│     Current conversation context, cleared on close  │
│                                                     │
│  💾 Long-term Memory (across sessions)              │
│     Your preferences, habits, common information    │
│     Examples:                                       │
│     - "User prefers concise report format"          │
│     - "User's project directory is ~/Projects"      │
│     - "User prefers VS Code for coding"             │
│                                                     │
└─────────────────────────────────────────────────────┘
```

### How Memory Works

```
Conversation 1:
You: My projects are all in D:/Projects
AI: Got it, I'll remember that. [Save to memory]

... one week later ...

Conversation 2:
You: Help me check my recent projects
AI: [Read memory: Projects directory is D:/Projects]
    Sure, let me look at projects in D:/Projects...
```

---

## Recipes and Automation

### What Are Recipes

Recipes are "reusable workflows." Like cooking recipes, they record the steps to complete a task.

### Recipe Example

```yaml
# Weekly Report Recipe
name: Generate Weekly Report
description: Auto-collect this week's work and generate report

steps:
  - Read this week's Git commits
  - Read this week's meeting notes
  - Read this week's completed tasks
  - Summarize and generate report document
  - Save to specified location
```

### Recipe + Schedule = Automation

```
┌─────────────────────────────────────────┐
│           Automation Flow                │
├─────────────────────────────────────────┤
│                                         │
│  Recipe: Daily Backup                    │
│    │                                    │
│    ▼                                    │
│  Schedule: Every day at 18:00            │
│    │                                    │
│    ▼                                    │
│  ┌─────────────────────────┐           │
│  │ 18:00 arrived!           │           │
│  │ AGIME auto-runs recipe   │           │
│  │ → Backup important files │           │
│  │ → Send completion notice │           │
│  └─────────────────────────┘           │
│                                         │
│  You don't need to do anything ✓        │
│                                         │
└─────────────────────────────────────────┘
```

---

## Security Mechanisms

### Four Work Modes

AGIME provides four modes to balance convenience and security:

```
┌─────────────────────────────────────────────────────┐
│                                                     │
│  🟢 Autonomous Mode                                 │
│     AI can freely execute any operation             │
│     For: Fully trusted repetitive tasks             │
│                                                     │
│  🟡 Smart Mode ⭐ Recommended                       │
│     Low-risk ops auto-execute, high-risk need OK    │
│     For: Daily use                                  │
│                                                     │
│  🔴 Manual Mode                                     │
│     Every operation needs your confirmation         │
│     For: Sensitive operations, learning             │
│                                                     │
│  ⚪ Chat Mode                                       │
│     Conversation only, no execution                 │
│     For: Pure Q&A                                   │
│                                                     │
└─────────────────────────────────────────────────────┘
```

### Risk Levels

Different operations have different risk levels:

| Risk Level | Operation Type | In Smart Mode |
|------------|----------------|---------------|
| 🟢 Low | Read files, view info | Auto-execute |
| 🟡 Medium | Create files, network requests | Auto-execute |
| 🟠 Higher | Modify files, run commands | Needs confirmation |
| 🔴 High | Delete files, system operations | Must confirm |

### Confirmation Dialog

When confirmation is needed, you'll see:

```
┌─────────────────────────────────────────┐
│  ⚠️ Operation Confirmation               │
├─────────────────────────────────────────┤
│                                         │
│  AI wants to perform this operation:     │
│                                         │
│  📄 Delete file                          │
│     Path: ~/Downloads/temp.txt           │
│                                         │
│  ┌─────────┐  ┌─────────┐              │
│  │  Allow  │  │  Deny   │              │
│  └─────────┘  └─────────┘              │
│                                         │
│  □ Don't ask again for this type        │
│                                         │
└─────────────────────────────────────────┘
```

---

## Data Security

### Local-First

```
┌─────────────────────────────────────────┐
│          Data Storage Location           │
├─────────────────────────────────────────┤
│                                         │
│  ✅ Stored on your computer              │
│     - Chat history                       │
│     - Recipe configs                     │
│     - Memory data                        │
│     - All settings                       │
│                                         │
│  🔄 Sent to AI service (only when used)  │
│     - Current conversation               │
│     - Only for AI understanding          │
│     - Discarded after processing         │
│                                         │
│  ❌ We do NOT collect                    │
│     - Any personal data                  │
│     - Any usage records                  │
│     - Any file contents                  │
│                                         │
└─────────────────────────────────────────┘
```

### Fully Offline Option

For higher privacy requirements, use local models:

```
Using Ollama + Local Model
     │
     ▼
All data processed on your computer
     │
     ▼
No data leaves your device
     │
     ▼
100% Privacy Protection ✓
```

---

## Summary

```
┌─────────────────────────────────────────────────────┐
│                                                     │
│  How AGIME Works - Summary                          │
│                                                     │
│  1. You speak → AGIME understands your intent       │
│  2. AI thinks → Plans steps to complete task        │
│  3. Tools execute → Performs actions on computer    │
│  4. Loop iteration → Until task is complete         │
│  5. Learn & remember → Gets smarter over time       │
│                                                     │
│  Security guarantees:                               │
│  - Four work modes, flexible control                │
│  - Risk levels, high-risk needs confirmation        │
│  - Local data storage, privacy first                │
│                                                     │
└─────────────────────────────────────────────────────┘
```

---

<p align="center">
  <a href="./ARCHITECTURE.en.md">Technical Architecture →</a>
</p>

<p align="center">
  <a href="../README.en.md">← Back to Home</a>
</p>
