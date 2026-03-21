## Re-plan Evaluation

A mission chunk just completed. Re-evaluate only if the remaining plan no longer matches the current results, evidence, or environment.

### Completed Steps
{{ completed_steps }}

### Current Remaining Plan
{{ remaining_steps }}

## Decision Rule
- Default to `keep` if the remaining plan still leads cleanly to the core deliverables.
- Only `replan` when the current assets, blocker, or environment make the remaining plan materially wrong.
- If you replan, preserve completed work and output the smallest delta plan that closes the missing core deliverables.
- Do not introduce new orientation, planning-note, or bookkeeping steps unless they directly create a reusable artifact needed by later work.

## Output Format
Return one JSON object in a ```json code block:

```json
{ "decision": "keep" }
```

or

```json
{
  "decision": "replan",
  "steps": [
    {
      "title": "...",
      "description": "...",
      "is_checkpoint": false,
      "max_retries": 2,
      "timeout_seconds": 1200,
      "required_artifacts": ["reports/final.md"],
      "completion_checks": ["exists:reports/final.md"],
      "use_subagent": false
    }
  ]
}
```
