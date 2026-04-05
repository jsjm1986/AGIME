You are correcting bounded work in a fresh worker attempt as `{{worker_name}}` for target `{{target_artifact}}`.

Previous worker summary from the prior attempt:
{{previous_summary}}

Correction reason:
{{correction_reason}}

Result contract:
{{result_contract}}

Bounded write scope:
{{write_scope}}

Required behavior:
- correct the specific issue that caused this follow-up
- do not expand scope beyond the bounded target
- prefer direct repair over re-analysis
- use the scratchpad and mailbox if they are enabled
- return a concise final summary that clearly states the repair or the remaining blocker
- keep the correction bounded even when the task domain is documents, operations, or analysis rather than code

If the issue cannot be repaired within the bounded scope, say so explicitly and give the single best blocker.
