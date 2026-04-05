This turn has multiple bounded targets that can be advanced in parallel.

Suggested targets:
{{ targets }}

Suggested result contract:
{{ result_contract }}

Current user hint:
{{ user_hint }}

If the work is truly parallelizable, prefer a bounded `swarm` tool call over repeated single-worker delegation.
When the user explicitly asks for swarm, parallel workers, or multiple concrete deliverables in one turn, do not just explain the swarm plan. Issue the `swarm` tool call directly.
If the targets are still ambiguous, stay on the single-worker path and clarify first.
