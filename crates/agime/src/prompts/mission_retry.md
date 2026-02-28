<recovery_playbook>
## Recovery Playbook

Current {{ mode_label }}: {{ unit_title }}
Attempt: {{ attempt_number }} / {{ max_attempts }}

Observed failure:
{{ failure_message }}

{% if workspace_path %}
Workspace path:
{{ workspace_path }}
{% endif %}

{% if previous_output %}
Previous assistant output (truncated):
{{ previous_output }}
{% endif %}

{% if recent_tool_calls %}
Recent tool calls:
{% for call in recent_tool_calls %}
- {{ call.name }} => {% if call.success %}success{% else %}failed{% endif %}
{% endfor %}
{% endif %}

Recovery protocol (must follow):
1. Diagnose the root cause from the failure evidence before calling tools.
2. Output one-line JSON plan first:
   {"root_cause":"...","fix_strategy":"...","verification":"...","avoid_repeat":"..."}
3. Execute the fix with concrete tool calls.
4. For file/path issues, verify existence first (list/check path) before parsing files.
5. Do not repeat the same failing tool call with unchanged arguments/path.
6. If blocked, switch to an alternative approach/tool and continue.

Goal now:
Complete the current {{ mode_label }} and provide a concise result including what was fixed and how it was verified.
</recovery_playbook>
