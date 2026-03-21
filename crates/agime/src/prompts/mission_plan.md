You are planning a mission. Think first, then output the smallest result-oriented executable plan.

## Mission Goal
{{ goal }}

{% if context %}
## Additional Context
{{ context }}
{% endif %}

## Planning Principles
1. Decompose by dependency order around the final usable deliverables.
2. Keep each step actionable and verifiable.
3. Prefer real deliverables over narrative-only output.
4. Include explicit completion evidence only when it strengthens the final result.
5. For engineering work, prefer a lightweight quality pass and evidence bundle over unverified "it should work" claims.
6. The earliest step should materially create or advance a requested asset, or the strongest reusable evidence artifact. Do not start with pure orientation, workspace confirmation, or narration.
7. Required artifacts and completion checks should name core deliverables or strong evidence, not planning notes or bookkeeping files.
8. Do not create standalone "confirm workspace", "repeat contract", "write a planning note", or "summarize next steps" steps unless that work is itself a requested deliverable or a reusable artifact that clearly reduces execution risk.

## Output Format
Return only one JSON array in a ```json code block.

```json
[
  {
    "title": "Step title",
    "description": "What to do and expected outcome",
    "is_checkpoint": false,
    "required_artifacts": ["reports/final.md"],
    "completion_checks": ["exists:reports/final.md"],
    "use_subagent": false
  }
]
```

## Notes
- Recommended 1-5 steps (can exceed only if complexity clearly requires).
- For tiny goals that only ask for one file or one compact deliverable, prefer 1-3 steps total.
- For tiny goals, do not spend the first step only on orientation or narration. Start producing the requested asset as early as possible.
- For research or comparison work, the first step should usually create a reusable result artifact such as a source note, comparison table, evidence card, or draft report section instead of a generic exploration step.
- For multi-deliverable work, you may use one early specification/contract step only if it lands a reusable artifact that downstream steps will consume. Otherwise, fold the planning into the first production step.
- If the goal involves building a runtime surface such as an app/service/API/UI, deployment, ports, background processes, or live verification, do not return a single all-in-one step. Prefer 4-8 dependency-ordered steps.
- For runtime-facing goals, include a local quality/verification step before deployment and a final evidence-bundle step after deployment.
- `required_artifacts` must be workspace-relative paths.
- `completion_checks` should be cross-platform where possible (`exists:<relative_path>` preferred).
- Use `use_subagent=true` for broad research/synthesis subtasks.
- Do not add `timeout_seconds` or `max_retries` unless a task truly has a hard external limit that the worker cannot infer at runtime.
- Quality checks should stay pragmatic: prefer the strongest available evidence (build/lint/typecheck/test/smoke/code-review/runtime health), and if something is unavailable, record the skip reason instead of blocking the whole mission.
