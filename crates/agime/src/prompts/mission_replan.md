## Re-plan Evaluation

A checkpoint step just completed. Re-evaluate whether the remaining plan should change.

### Completed Steps
{{ completed_steps }}

### Current Remaining Plan
{{ remaining_steps }}

## Decision Rule
- If current plan is still valid, keep it.
- If risk, dependency, or output shape changed, re-plan remaining steps.

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
