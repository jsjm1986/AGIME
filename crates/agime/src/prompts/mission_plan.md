You are planning a mission. Think first, then output a concrete executable plan.

## Mission Goal
{{ goal }}

{% if context %}
## Additional Context
{{ context }}
{% endif %}

## Planning Principles
1. Decompose by dependency order.
2. Keep each step actionable and verifiable.
3. Prefer real deliverables over narrative-only output.
4. Include explicit completion evidence when useful.

## Output Format
Return only one JSON array in a ```json code block.

```json
[
  {
    "title": "Step title",
    "description": "What to do and expected outcome",
    "is_checkpoint": false,
    "max_retries": 2,
    "timeout_seconds": 1200,
    "required_artifacts": ["reports/final.md"],
    "completion_checks": ["exists:reports/final.md"],
    "use_subagent": false
  }
]
```

## Notes
- Recommended 2-8 steps (can exceed if complexity requires).
- `required_artifacts` must be workspace-relative paths.
- `completion_checks` should be cross-platform where possible (`exists:<relative_path>` preferred).
- Use `use_subagent=true` for broad research/synthesis subtasks.
