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
5. For engineering work, prefer a lightweight quality pass and evidence bundle over unverified "it should work" claims.

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
- If the goal involves building an app/service/API/UI, deployment, pm2, ports, or live verification, do not return a single all-in-one step. Prefer 4-8 dependency-ordered steps.
- For app/service/API/UI goals, include a local quality/verification step before deployment and a final evidence-bundle step after deployment.
- `required_artifacts` must be workspace-relative paths.
- `completion_checks` should be cross-platform where possible (`exists:<relative_path>` preferred).
- Use `use_subagent=true` for broad research/synthesis subtasks.
- Quality checks should stay pragmatic: prefer the strongest available evidence (build/lint/typecheck/test/smoke/code-review/runtime health), and if something is unavailable, record the skip reason instead of blocking the whole mission.
