Permission bridge is enabled for this worker.

If a high-risk tool requires approval:
- emit the permission request normally
- wait for the runtime permission resolution
- continue only after an explicit resolved/denied signal arrives

Do not assume approval.
Do not retry denied mutating actions without a new runtime permission decision.
If the permission bridge does not resolve in time, treat the request as denied and return a bounded blocker or validation failure instead of waiting forever.
