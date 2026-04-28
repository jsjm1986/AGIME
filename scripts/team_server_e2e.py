#!/usr/bin/env python3
"""AGIME Team Server end-to-end smoke and regression suite.

This script is intended to run on the Linux server that hosts the live
`agime-team-server` process. It exercises the real HTTP API, real MongoDB
state, and real provider configuration. The goal is to turn the existing
manual Runtime 6 validation flow into a repeatable, report-producing asset.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
import uuid
from dataclasses import asdict, dataclass, field
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any, Callable
from urllib.parse import urlparse

import requests
from bson import ObjectId
from pymongo import MongoClient
from pymongo.collection import Collection


DEFAULT_BASE_URL = os.environ.get("AGIME_E2E_BASE_URL", "http://127.0.0.1:9999")
DEFAULT_AGENTIFY_TARGET_URL = os.environ.get("AGIME_E2E_AGENTIFY_TARGET_URL", DEFAULT_BASE_URL)
DEFAULT_MONGO_URI = os.environ.get("AGIME_E2E_MONGO_URI", "mongodb://127.0.0.1:27017")
DEFAULT_DB_NAME = os.environ.get("AGIME_E2E_DB_NAME", "agime_team")
DEFAULT_WORKSPACE = os.environ.get("AGIME_E2E_WORKSPACE", "/opt/agime")
DEFAULT_LOGS_DIR = os.environ.get("AGIME_E2E_LOGS_DIR", "/root/.local/state/agime/logs")
DEFAULT_TEAM_NAME = os.environ.get("AGIME_E2E_TEAM_NAME", "agime")
DEFAULT_AGENT_NAME = os.environ.get("AGIME_E2E_AGENT_NAME", "GLM")
DEFAULT_JSON_OUT = os.environ.get(
    "AGIME_E2E_JSON_OUT", "/tmp/agime-team-server-e2e-report.json"
)
TARGETED_RUST_TESTS = [
    ("agime", "deserializing_v5_state_upgrades_to_v6_schema_without_losing_store"),
    ("agime", "resolve_for_provider_uses_provider_capability_hooks"),
    ("agime", "resolve_for_provider_with_model_uses_call_specific_intent"),
    ("agime", "apply_effective_execution_materializes_downgraded_model_config"),
    ("agime", "test_create_request_respects_prompt_caching_off"),
    ("agime", "committed_from_direction_survives_export_import_projection"),
    ("agime", "channel_coding_overlay_highlights_direct_build_priority"),
    ("agime", "coding_thread_intent_defaults_to_direct_build"),
    ("agime", "normalize_card_summary_drops_template_prefixes_and_duplicate_lines"),
    ("agime", "registry_matches_same_revision_only_once"),
    ("agime", "reminder_revision_changes_only_when_status_or_latest_activity_changes"),
]
HARNESS_TARGETED_RUST_TESTS = [
    ("agime", "transition_trace_records_mode_and_budget_events"),
    ("agime", "normalize_tool_execution_error_text_classifies_cancelled"),
    ("agime", "load_task_ledger_transcript_resume_view_refreshes_status_and_summary_metadata"),
    ("agime", "load_task_ledger_transcript_resume_view_downgrades_stale_persisted_resume"),
    ("agime-team-server", "runtime_control_registry_suppresses_duplicate_permission_resolution"),
    ("agime-team-server", "runtime_control_registry_suppresses_stale_worker_finished_attempt"),
    ("agime-team-server", "build_workspace_artifact_resolution_snapshot_distinguishes_target_kinds"),
    ("agime-team-server", "load_runtime_diagnostics_snapshot_uses_runtime_outcome_and_child_evidence"),
]
SCHEDULED_TASK_TARGETED_RUST_TESTS = [
    ("agime-team-server", "compute_next_fire_for_one_shot_uses_explicit_timestamp"),
    ("agime-team-server", "compute_next_fire_for_cron_respects_timezone"),
    ("agime-team-server", "scheduled_trigger_source_marks_overdue_tasks_as_missed_recovery"),
    ("agime-team-server", "scheduled_trigger_source_keeps_recent_due_task_as_schedule"),
    ("agime-team-server", "durable_tasks_are_visible_to_admin"),
    ("agime-team-server", "session_scoped_tasks_remain_owner_only_even_for_admin"),
    ("agime-team-server", "session_scoped_owner_defaults_to_current_session"),
    ("agime-team-server", "durable_tasks_clear_owner_session_binding"),
]
HARNESS_TRANSITION_KINDS = {
    "reply_bootstrap",
    "provider_turn_recovery",
    "post_turn_adjudication",
    "mode_transition",
    "recovery_compaction",
    "no_tool_repair",
    "planner_auto_upgrade",
    "coordinator_completion",
    "tool_budget_fallback",
    "child_task_downgrade",
    "swarm_fallback",
}


class CaseFailed(RuntimeError):
    """A single test case failed."""


class CaseBlocked(RuntimeError):
    """A single test case was blocked by environment or missing prerequisites."""


@dataclass
class CaseResult:
    name: str
    entry: str
    status: str
    ids: dict[str, Any] = field(default_factory=dict)
    evidence: dict[str, Any] = field(default_factory=dict)
    cleanup: str = "not_required"
    details: str = ""


@dataclass
class TeamContext:
    team_id: str
    team_doc_id: ObjectId
    owner_user_id: str
    agent_id: str
    agent_name: str
    portal_slug: str | None


def utcnow() -> datetime:
    return datetime.now(timezone.utc)


def json_default(value: Any) -> Any:
    if isinstance(value, ObjectId):
        return str(value)
    if isinstance(value, datetime):
        return value.isoformat()
    raise TypeError(f"Unsupported JSON value: {type(value)!r}")


def short_uid(prefix: str) -> str:
    return f"{prefix}-{uuid.uuid4().hex[:8]}"


def print_step(message: str) -> None:
    print(f"[e2e] {message}", flush=True)


def run_subprocess(
    command: list[str],
    *,
    cwd: str | None = None,
    timeout: int = 600,
) -> subprocess.CompletedProcess[str]:
    print_step(f"run: {' '.join(command)}")
    return subprocess.run(
        command,
        cwd=cwd,
        text=True,
        capture_output=True,
        timeout=timeout,
        check=False,
    )


class TeamServerE2ESuite:
    TRANSIENT_PROVIDER_MARKERS = (
        "ratelimitexceeded",
        "429 too many requests",
        "该模型当前访问量过大",
        "without any message content blocks",
        "without a user-visible assistant response",
    )
    TRANSIENT_PROVIDER_RETRY_ATTEMPTS = 4
    TRANSIENT_PROVIDER_RETRY_DELAY_SECS = 15
    HTTP_RETRY_ATTEMPTS = 4
    HTTP_RETRY_DELAY_SECS = 2

    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.base_url = args.base_url.rstrip("/")
        self.mongo = MongoClient(args.mongo_uri, serverSelectionTimeoutMS=5000)
        self.db = self.mongo[args.db_name]
        self.http = requests.Session()
        self.http.headers.update({"User-Agent": "codex-e2e-suite/1.0"})
        self.report: list[CaseResult] = []
        self.context: TeamContext | None = None
        self.temp_web_sessions: set[str] = set()
        self.temp_chat_sessions: set[str] = set()
        self.temp_tasks: set[str] = set()
        self.temp_channels: set[str] = set()
        self.temp_scheduled_tasks: set[str] = set()
        self.temp_documents: set[str] = set()
        self.temp_visitors: set[str] = set()
        self.temp_api_keys: set[str] = set()
        self.temp_auth_user_ids: set[str] = set()
        self.temp_automation_project_ids: set[str] = set()
        self.harness_case_ids: dict[str, dict[str, Any]] = {}

    def collection(self, name: str) -> Collection:
        return self.db[name]

    def expect(self, condition: bool, message: str) -> None:
        if not condition:
            raise CaseFailed(message)

    def remember_case_ids(self, case_name: str, ids: dict[str, Any]) -> None:
        self.harness_case_ids[case_name] = ids

    def case_ids(self, case_name: str) -> dict[str, Any]:
        ids = self.harness_case_ids.get(case_name)
        if ids is None:
            raise CaseBlocked(f"missing prerequisite harness case ids: {case_name}")
        return ids

    def extract_last_assistant_text_from_session_doc(
        self, session_id: str
    ) -> str:
        session_doc = self.collection("agent_sessions").find_one({"session_id": session_id}) or {}
        messages = json.loads(session_doc.get("messages_json") or "[]")
        for message in reversed(messages):
            if message.get("role") != "assistant":
                continue
            content = message.get("content")
            if isinstance(content, str) and content.strip():
                return content
            if isinstance(content, list):
                text_parts: list[str] = []
                for item in content:
                    if isinstance(item, dict):
                        text = item.get("text")
                        if isinstance(text, str) and text.strip():
                            text_parts.append(text.strip())
                if text_parts:
                    return "\n".join(text_parts)
        return (session_doc.get("last_message_preview") or "").strip()

    def wait_for_session_assistant_text(
        self,
        session_id: str,
        *,
        required_substrings: list[str],
        timeout: int | None = None,
    ) -> str:
        deadline = time.time() + (timeout or self.args.poll_timeout)
        last_text = ""
        while time.time() < deadline:
            last_text = self.extract_last_assistant_text_from_session_doc(session_id)
            lowered = last_text.lower()
            if last_text and all(token.lower() in lowered for token in required_substrings):
                return last_text
            time.sleep(self.args.poll_interval)
        raise CaseFailed(
            f"timed out waiting for session {session_id} assistant text to include {required_substrings}: {last_text}"
        )

    def payload_has_transient_provider_issue(self, payload: Any) -> bool:
        if payload is None:
            return False
        try:
            serialized = json.dumps(payload, ensure_ascii=False).lower()
        except TypeError:
            serialized = str(payload).lower()
        return any(marker in serialized for marker in self.TRANSIENT_PROVIDER_MARKERS)

    def raise_if_transient_provider_block(self, payload: Any, surface: str) -> None:
        if not isinstance(payload, dict):
            return
        status_candidates = [
            payload.get("last_execution_status"),
            payload.get("execution_status"),
            payload.get("status"),
            (payload.get("thread_runtime") or {}).get("execution_status"),
            (payload.get("runtime_diagnostics") or {}).get("completion_status"),
        ]
        if any(status == "blocked" for status in status_candidates) and self.payload_has_transient_provider_issue(payload):
            raise CaseBlocked(f"{surface} blocked by transient provider issue")

    def is_transient_provider_block_error(self, exc: Exception) -> bool:
        return "transient provider issue" in str(exc).lower()

    def find_latest_hidden_agent_session(
        self,
        *,
        exclude_session_ids: set[str] | None = None,
    ) -> dict[str, Any] | None:
        self.expect(self.context is not None, "team context not initialized")
        query: dict[str, Any] = {
            "team_id": self.context.team_id,
            "agent_id": self.context.agent_id,
            "user_id": self.context.owner_user_id,
            "hidden_from_chat_list": True,
        }
        if exclude_session_ids:
            query["session_id"] = {"$nin": list(exclude_session_ids)}
        hidden = self.collection("agent_sessions").find_one(query, sort=[("updated_at", -1)])
        if hidden is None and exclude_session_ids:
            query.pop("session_id", None)
            hidden = self.collection("agent_sessions").find_one(query, sort=[("updated_at", -1)])
        return hidden

    def find_hidden_agent_session_by_marker(
        self,
        marker: str,
        *,
        exclude_session_ids: set[str] | None = None,
    ) -> dict[str, Any] | None:
        self.expect(self.context is not None, "team context not initialized")
        query: dict[str, Any] = {
            "team_id": self.context.team_id,
            "agent_id": self.context.agent_id,
            "user_id": self.context.owner_user_id,
            "hidden_from_chat_list": True,
            "messages_json": {"$regex": re.escape(marker)},
        }
        if exclude_session_ids:
            query["session_id"] = {"$nin": list(exclude_session_ids)}
        hidden = self.collection("agent_sessions").find_one(query, sort=[("updated_at", -1)])
        if hidden is None and exclude_session_ids:
            query.pop("session_id", None)
            hidden = self.collection("agent_sessions").find_one(query, sort=[("updated_at", -1)])
        return hidden

    def run_case(self, name: str, entry: str, fn: Callable[[], CaseResult]) -> None:
        if self.args.cases:
            selected = {item.strip() for item in self.args.cases.split(",") if item.strip()}
            if name not in selected:
                return
        print_step(f"case start: {name}")
        for attempt in range(1, self.TRANSIENT_PROVIDER_RETRY_ATTEMPTS + 2):
            try:
                result = fn()
                result.name = name
                result.entry = entry
                self.report.append(result)
                print_step(f"case passed: {name}")
                return
            except CaseBlocked as exc:
                if (
                    self.is_transient_provider_block_error(exc)
                    and attempt <= self.TRANSIENT_PROVIDER_RETRY_ATTEMPTS
                ):
                    print_step(
                        f"case retry: {name}: transient provider block on attempt {attempt}, retrying"
                    )
                    time.sleep(self.TRANSIENT_PROVIDER_RETRY_DELAY_SECS)
                    continue
                self.report.append(
                    CaseResult(
                        name=name,
                        entry=entry,
                        status="blocked by environment",
                        details=str(exc),
                    )
                )
                print_step(f"case blocked: {name}: {exc}")
                return
            except Exception as exc:  # noqa: BLE001
                self.report.append(
                    CaseResult(name=name, entry=entry, status="failed", details=str(exc))
                )
                print_step(f"case failed: {name}: {exc}")
                return

    def is_transient_http_restart_error(self, exc: Exception) -> bool:
        lowered = str(exc).lower()
        return any(
            marker in lowered
            for marker in (
                "connection refused",
                "failed to establish a new connection",
                "connection reset by peer",
                "remote end closed connection without response",
            )
        )

    def request_json(
        self,
        session: requests.Session,
        method: str,
        path: str,
        *,
        body: dict[str, Any] | None = None,
        **kwargs: Any,
    ) -> tuple[requests.Response, Any]:
        timeout = kwargs.pop("timeout", 30)
        url = f"{self.base_url}{path}"
        for attempt in range(1, self.HTTP_RETRY_ATTEMPTS + 1):
            try:
                response = session.request(method, url, json=body, timeout=timeout, **kwargs)
                try:
                    payload = response.json() if response.content else None
                except ValueError:
                    payload = response.text if response.content else None
                return response, payload
            except requests.RequestException as exc:
                if self.is_transient_http_restart_error(exc) and attempt < self.HTTP_RETRY_ATTEMPTS:
                    print_step(
                        f"http retry: {method} {path}: transient restart window on attempt {attempt}"
                    )
                    time.sleep(self.HTTP_RETRY_DELAY_SECS)
                    continue
                raise

    def post_json(
        self, session: requests.Session, path: str, body: dict[str, Any], **kwargs: Any
    ) -> tuple[requests.Response, Any]:
        return self.request_json(session, "POST", path, body=body, **kwargs)

    def get_json(
        self, session: requests.Session, path: str, **kwargs: Any
    ) -> tuple[requests.Response, Any]:
        return self.request_json(session, "GET", path, **kwargs)

    def put_json(
        self, session: requests.Session, path: str, body: dict[str, Any], **kwargs: Any
    ) -> tuple[requests.Response, Any]:
        return self.request_json(session, "PUT", path, body=body, **kwargs)

    def delete_json(
        self, session: requests.Session, path: str, body: dict[str, Any] | None = None, **kwargs: Any
    ) -> tuple[requests.Response, Any]:
        return self.request_json(session, "DELETE", path, body=body, **kwargs)

    def load_chat_session_detail(
        self, session: requests.Session, session_id: str
    ) -> dict[str, Any]:
        response, payload = self.get_json(session, f"/api/team/agent/chat/sessions/{session_id}")
        self.expect(response.status_code == 200, f"failed to load chat session detail: {session_id}")
        self.raise_if_transient_provider_block(payload, f"chat session {session_id}")
        return payload

    def create_automation_project(
        self, auth: requests.Session, *, name: str, description: str | None = None
    ) -> dict[str, Any]:
        self.expect(self.context is not None, "team context not initialized")
        response, payload = self.post_json(
            auth,
            "/api/team/automation/projects",
            {
                "team_id": self.context.team_id,
                "name": name,
                "description": description,
            },
        )
        self.expect(response.status_code == 200, f"create automation project failed: {payload}")
        project = payload.get("project") or {}
        project_id = project.get("project_id")
        self.expect(bool(project_id), "automation project missing project_id")
        self.temp_automation_project_ids.add(project_id)
        return project

    def delete_automation_project(self, auth: requests.Session, project_id: str) -> None:
        self.expect(self.context is not None, "team context not initialized")
        response, payload = self.delete_json(
            auth,
            f"/api/team/automation/projects/{project_id}",
            params={"team_id": self.context.team_id},
        )
        self.expect(response.status_code == 200, f"delete automation project failed: {payload}")
        self.temp_automation_project_ids.discard(project_id)

    def create_automation_integration(
        self,
        auth: requests.Session,
        *,
        project_id: str,
        name: str,
        spec_kind: str,
        spec_content: str,
        base_url: str | None = None,
        auth_type: str = "none",
        auth_config: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        self.expect(self.context is not None, "team context not initialized")
        response, payload = self.post_json(
            auth,
            f"/api/team/automation/projects/{project_id}/integrations?team_id={self.context.team_id}",
            {
                "team_id": self.context.team_id,
                "project_id": project_id,
                "name": name,
                "spec_kind": spec_kind,
                "spec_content": spec_content,
                "base_url": base_url,
                "auth_type": auth_type,
                "auth_config": auth_config or {},
            },
        )
        self.expect(response.status_code == 200, f"create automation integration failed: {payload}")
        integration = payload.get("integration") or {}
        self.expect(bool(integration.get("integration_id")), "automation integration missing id")
        return integration

    def create_automation_app_draft(
        self,
        auth: requests.Session,
        *,
        project_id: str,
        name: str,
        driver_agent_id: str,
        integration_ids: list[str],
        goal: str,
        constraints: list[str],
        success_criteria: list[str],
        risk_preference: str = "balanced",
        create_builder_session: bool = True,
    ) -> dict[str, Any]:
        self.expect(self.context is not None, "team context not initialized")
        response, payload = self.post_json(
            auth,
            f"/api/team/automation/projects/{project_id}/app-drafts?team_id={self.context.team_id}",
            {
                "team_id": self.context.team_id,
                "project_id": project_id,
                "name": name,
                "driver_agent_id": driver_agent_id,
                "integration_ids": integration_ids,
                "goal": goal,
                "constraints": constraints,
                "success_criteria": success_criteria,
                "risk_preference": risk_preference,
                "create_builder_session": create_builder_session,
            },
        )
        self.expect(response.status_code == 200, f"create app draft failed: {payload}")
        return payload

    def sync_automation_app_draft(
        self, auth: requests.Session, draft_id: str
    ) -> dict[str, Any]:
        self.expect(self.context is not None, "team context not initialized")
        response, payload = self.post_json(
            auth,
            f"/api/team/automation/app-drafts/{draft_id}/sync-builder?team_id={self.context.team_id}",
            {},
        )
        self.expect(response.status_code == 200, f"sync automation app draft failed: {payload}")
        return payload

    def publish_automation_app(
        self, auth: requests.Session, draft_id: str, *, name: str
    ) -> tuple[requests.Response, Any]:
        self.expect(self.context is not None, "team context not initialized")
        return self.post_json(
            auth,
            f"/api/team/automation/app-drafts/{draft_id}/publish?team_id={self.context.team_id}",
            {"name": name},
        )

    def load_automation_app_runtime(
        self, auth: requests.Session, module_id: str
    ) -> dict[str, Any]:
        self.expect(self.context is not None, "team context not initialized")
        response, payload = self.get_json(
            auth,
            f"/api/team/automation/apps/{module_id}/runtime?team_id={self.context.team_id}",
        )
        self.expect(response.status_code == 200, f"load automation app runtime failed: {payload}")
        return payload

    def load_chat_session_events(
        self, session: requests.Session, session_id: str
    ) -> list[dict[str, Any]]:
        response, payload = self.get_json(
            session, f"/api/team/agent/chat/sessions/{session_id}/events"
        )
        self.expect(response.status_code == 200, f"failed to fetch chat session events: {session_id}")
        self.expect(isinstance(payload, list), "chat session events payload is not a list")
        return payload

    def load_channel_thread(
        self, session: requests.Session, channel_id: str, root_message_id: str
    ) -> dict[str, Any]:
        response, payload = self.get_json(
            session, f"/api/team/agent/chat/channels/{channel_id}/threads/{root_message_id}"
        )
        self.expect(response.status_code == 200, f"failed to load channel thread: {channel_id}/{root_message_id}")
        self.raise_if_transient_provider_block(payload, f"channel thread {channel_id}/{root_message_id}")
        return payload

    def load_channel_detail(
        self, session: requests.Session, channel_id: str
    ) -> dict[str, Any]:
        response, payload = self.get_json(
            session, f"/api/team/agent/chat/channels/{channel_id}"
        )
        self.expect(response.status_code == 200, f"failed to load channel detail: {channel_id}")
        self.raise_if_transient_provider_block(payload, f"channel detail {channel_id}")
        return payload.get("channel") or payload

    def create_scheduled_task(
        self,
        auth: requests.Session,
        *,
        title: str,
        prompt: str,
        task_kind: str,
        agent_id: str | None = None,
        delivery_tier: str | None = None,
        owner_session_id: str | None = None,
        one_shot_at: str | None = None,
        cron_expression: str | None = None,
        timezone_name: str = "Asia/Shanghai",
    ) -> dict[str, Any]:
        self.expect(self.context is not None, "team context not initialized")
        body: dict[str, Any] = {
            "agent_id": agent_id or self.context.agent_id,
            "title": title,
            "prompt": prompt,
            "task_kind": task_kind,
            "timezone": timezone_name,
        }
        if delivery_tier is not None:
            body["delivery_tier"] = delivery_tier
        if owner_session_id is not None:
            body["owner_session_id"] = owner_session_id
        if one_shot_at is not None:
            body["one_shot_at"] = one_shot_at
        if cron_expression is not None:
            body["cron_expression"] = cron_expression
        response, payload = self.post_json(
            auth,
            f"/api/team/scheduled-tasks?team_id={self.context.team_id}",
            body,
        )
        self.expect(response.status_code == 200, f"create scheduled task failed: {payload}")
        task = payload["task"]
        self.temp_scheduled_tasks.add(task["task_id"])
        self.temp_channels.add(task["channel_id"])
        return task

    def parse_scheduled_task_preview(
        self,
        auth: requests.Session,
        *,
        text: str,
        agent_id: str | None = None,
        timezone_name: str = "Asia/Shanghai",
    ) -> dict[str, Any]:
        self.expect(self.context is not None, "team context not initialized")
        response, payload = self.post_json(
            auth,
            f"/api/team/scheduled-tasks/parse?team_id={self.context.team_id}",
            {
                "text": text,
                "agent_id": agent_id or self.context.agent_id,
                "timezone": timezone_name,
            },
        )
        self.expect(
            response.status_code == 200,
            f"parse scheduled task preview failed: {payload}",
        )
        return payload.get("preview") or payload

    def create_scheduled_task_from_parse(
        self,
        auth: requests.Session,
        *,
        preview: dict[str, Any],
        overrides: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        self.expect(self.context is not None, "team context not initialized")
        response, payload = self.post_json(
            auth,
            f"/api/team/scheduled-tasks/create-from-parse?team_id={self.context.team_id}",
            {"preview": preview, "overrides": overrides or {}},
        )
        self.expect(
            response.status_code == 200,
            f"create scheduled task from parse failed: {payload}",
        )
        task = payload["task"]
        self.temp_scheduled_tasks.add(task["task_id"])
        self.temp_channels.add(task["channel_id"])
        return task

    def list_scheduled_tasks(
        self, auth: requests.Session, view: str = "mine"
    ) -> list[dict[str, Any]]:
        self.expect(self.context is not None, "team context not initialized")
        response, payload = self.get_json(
            auth,
            "/api/team/scheduled-tasks",
            params={"team_id": self.context.team_id, "view": view},
        )
        self.expect(response.status_code == 200, f"list scheduled tasks failed: {payload}")
        return payload.get("tasks") or []

    def load_scheduled_task_detail(
        self, auth: requests.Session, task_id: str
    ) -> dict[str, Any]:
        self.expect(self.context is not None, "team context not initialized")
        response, payload = self.get_json(
            auth,
            f"/api/team/scheduled-tasks/{task_id}",
            params={"team_id": self.context.team_id},
        )
        self.expect(response.status_code == 200, f"failed to load scheduled task {task_id}")
        self.raise_if_transient_provider_block(payload, f"scheduled task {task_id}")
        return payload["task"]

    def post_scheduled_task_action(
        self, auth: requests.Session, task_id: str, action: str
    ) -> dict[str, Any]:
        self.expect(self.context is not None, "team context not initialized")
        response, payload = self.post_json(
            auth,
            f"/api/team/scheduled-tasks/{task_id}/{action}?team_id={self.context.team_id}",
            {},
        )
        self.expect(response.status_code == 200, f"scheduled task action {action} failed: {payload}")
        return payload

    def wait_for_scheduled_task_run(
        self, auth: requests.Session, task_id: str, expected_statuses: set[str]
    ) -> dict[str, Any]:
        deadline = time.time() + self.args.poll_timeout
        while time.time() < deadline:
            task = self.load_scheduled_task_detail(auth, task_id)
            runs = task.get("runs") or []
            if runs:
                latest = runs[0]
                status = (latest.get("status") or "").strip().lower()
                if status in expected_statuses:
                    return task
                if status in {"completed", "failed", "cancelled"} - expected_statuses:
                    raise CaseFailed(
                        f"scheduled task {task_id} reached unexpected terminal status: {status}"
                    )
            time.sleep(self.args.poll_interval)
        raise CaseFailed(
            f"timed out waiting for scheduled task {task_id} to reach one of {sorted(expected_statuses)}"
        )

    def submit_chat_task(
        self,
        auth: requests.Session,
        content: str,
        *,
        session_id: str | None = None,
    ) -> tuple[str, dict[str, Any]]:
        self.expect(self.context is not None, "team context not initialized")
        payload: dict[str, Any] = {
            "team_id": self.context.team_id,
            "agent_id": self.context.agent_id,
            "task_type": "chat",
            "content": {"messages": [{"role": "user", "content": content}]},
        }
        if session_id:
            payload["content"]["session_id"] = session_id
        response, body = self.post_json(auth, "/api/team/agent/tasks", payload)
        self.expect(response.status_code == 200, f"submit task failed: {body}")
        task_id = body["id"]
        self.temp_tasks.add(task_id)
        return task_id, body

    def wait_for_task_status(
        self,
        session: requests.Session,
        task_id: str,
        expected_statuses: set[str],
    ) -> dict[str, Any]:
        deadline = time.time() + self.args.poll_timeout
        while time.time() < deadline:
            response, payload = self.get_json(session, f"/api/team/agent/tasks/{task_id}")
            self.expect(response.status_code == 200, f"failed to get task {task_id}")
            self.raise_if_transient_provider_block(payload, f"task {task_id}")
            status = payload.get("status")
            if status in expected_statuses:
                return payload
            if status in {"completed", "failed", "cancelled", "rejected"} - expected_statuses:
                raise CaseFailed(f"task {task_id} reached unexpected terminal status: {status}")
            time.sleep(self.args.poll_interval)
        raise CaseFailed(
            f"timed out waiting for task {task_id} to reach one of {sorted(expected_statuses)}"
        )

    def wait_for_chat_status(
        self,
        session: requests.Session,
        session_id: str,
        expected_statuses: set[str],
        *,
        require_processing: bool = False,
    ) -> dict[str, Any]:
        deadline = time.time() + self.args.poll_timeout
        while time.time() < deadline:
            payload = self.load_chat_session_detail(session, session_id)
            status = (payload.get("last_execution_status") or "").strip().lower()
            if status in expected_statuses:
                return payload
            if require_processing and payload.get("is_processing"):
                return payload
            time.sleep(self.args.poll_interval)
        raise CaseFailed(
            f"timed out waiting for chat {session_id} to reach one of {sorted(expected_statuses)}"
        )

    def cancel_task(self, session: requests.Session, task_id: str) -> dict[str, Any]:
        response, payload = self.post_json(session, f"/api/team/agent/tasks/{task_id}/cancel", {})
        self.expect(response.status_code == 200, f"cancel task failed: {payload}")
        return payload

    def cancel_chat_session(self, session: requests.Session, session_id: str) -> None:
        response = session.post(
            f"{self.base_url}/api/team/agent/chat/sessions/{session_id}/cancel",
            timeout=30,
        )
        self.expect(
            response.status_code == 200,
            f"failed to cancel chat session {session_id}: {response.text}",
        )

    def wait_for_hidden_session_by_marker(
        self,
        marker: str,
        *,
        exclude_session_ids: set[str] | None = None,
    ) -> dict[str, Any]:
        deadline = time.time() + self.args.poll_timeout
        while time.time() < deadline:
            hidden = self.find_hidden_agent_session_by_marker(
                marker, exclude_session_ids=exclude_session_ids
            )
            if hidden is not None:
                return hidden
            time.sleep(self.args.poll_interval)
        raise CaseFailed(f"hidden session not found for marker: {marker}")

    def report_case(self, name: str) -> CaseResult:
        for item in reversed(self.report):
            if item.name == name:
                return item
        raise CaseBlocked(f"missing prerequisite case result: {name}")

    def resolve_team_by_name(self, team_name: str) -> TeamContext:
        teams = list(
            self.collection("teams")
            .find({"name": team_name})
            .sort([("_id", -1)])
        )
        if not teams:
            raise CaseBlocked(f"team '{team_name}' not found")
        selected: tuple[dict[str, Any], dict[str, Any]] | None = None
        for team in teams:
            team_id = str(team["_id"])
            agent = self.collection("team_agents").find_one(
                {
                    "team_id": team_id,
                    "name": {"$regex": f"^{self.args.agent_name}", "$options": "i"},
                }
            )
            if agent is not None:
                selected = (team, agent)
                break
        if selected is None:
            raise CaseBlocked(
                f"agent '{self.args.agent_name}' not found in any team named '{team_name}'"
            )
        team, agent = selected
        team_id = str(team["_id"])
        owner_user_id = team.get("owner_id")
        if not owner_user_id:
            raise CaseBlocked(f"team '{team_name}' missing owner_id")
        portal = None
        if self.args.portal_slug:
            portal = self.collection("portals").find_one(
                {
                    "team_id": team["_id"],
                    "slug": self.args.portal_slug,
                    "is_deleted": {"$ne": True},
                }
            )
        if portal is None:
            portal = self.collection("portals").find_one(
                {
                    "team_id": team["_id"],
                    "status": "published",
                    "is_deleted": {"$ne": True},
                }
            )
        agent_id = agent.get("id") or agent.get("agent_id")
        if not agent_id:
            raise CaseBlocked("team agent missing id/agent_id")
        return TeamContext(
            team_id=team_id,
            team_doc_id=team["_id"],
            owner_user_id=owner_user_id,
            agent_id=agent_id,
            agent_name=agent["name"],
            portal_slug=portal.get("slug") if portal else None,
        )

    def create_temp_web_session(self, user_id: str) -> tuple[str, requests.Session]:
        session_id = str(uuid.uuid4())
        now = utcnow()
        self.collection("sessions").insert_one(
            {
                "session_id": session_id,
                "user_id": user_id,
                "created_at": now,
                "expires_at": now + timedelta(days=7),
            }
        )
        self.temp_web_sessions.add(session_id)
        session = requests.Session()
        session.headers.update({"User-Agent": "codex-e2e-suite/1.0"})
        session.cookies.set("agime_session", session_id)
        return session_id, session

    def find_non_owner_team_member_user_id(self) -> str:
        self.expect(self.context is not None, "team context not initialized")
        team = self.collection("teams").find_one({"_id": self.context.team_doc_id}) or {}
        members = team.get("members") or []
        for member in members:
            user_id = member.get("user_id")
            if user_id and user_id != self.context.owner_user_id:
                return user_id
        raise CaseBlocked("team has no non-owner member for visibility validation")

    def create_temp_password_user(self) -> tuple[str, str, str]:
        email = f"codex-e2e-{uuid.uuid4().hex[:12]}@example.test"
        password = f"CodexE2E!{uuid.uuid4().hex[:12]}"
        response, payload = self.post_json(
            requests.Session(),
            "/api/auth/register",
            {
                "email": email,
                "display_name": "Codex E2E Password User",
                "password": password,
            },
        )
        self.expect(
            response.status_code == 201,
            f"password test user registration failed: {payload}",
        )
        user = payload.get("user") or {}
        user_id = user.get("id")
        self.expect(bool(user_id), "password test registration missing user id")
        self.temp_auth_user_ids.add(user_id)
        return user_id, email, password

    def wait_for_chat_completion(
        self,
        session: requests.Session,
        session_id: str,
        *,
        public_slug: str | None = None,
        visitor_id: str | None = None,
    ) -> dict[str, Any]:
        deadline = time.time() + self.args.poll_timeout
        while time.time() < deadline:
            if public_slug:
                response, payload = self.get_json(
                    session,
                    f"/p/{public_slug}/api/chat/sessions",
                    params={"visitor_id": visitor_id},
                )
                self.expect(response.status_code == 200, "failed to list public sessions")
                sessions = payload.get("sessions", [])
                match = next(
                    (item for item in sessions if item.get("session_id") == session_id),
                    None,
                )
                if match and match.get("last_execution_status") in {
                    "completed",
                    "failed",
                    "blocked",
                }:
                    return match
            else:
                response, payload = self.get_json(
                    session, f"/api/team/agent/chat/sessions/{session_id}"
                )
                self.expect(response.status_code == 200, "failed to get chat session detail")
                self.raise_if_transient_provider_block(payload, f"chat session {session_id}")
                if payload.get("last_execution_status") in {"completed", "failed", "blocked"}:
                    return payload
            time.sleep(self.args.poll_interval)
        raise CaseFailed(f"timed out waiting for chat completion: {session_id}")

    def wait_for_chat_completion_after_update(
        self,
        session: requests.Session,
        session_id: str,
        *,
        previous_message_count: Any,
        previous_last_message_at: Any,
    ) -> dict[str, Any]:
        deadline = time.time() + self.args.poll_timeout
        while time.time() < deadline:
            payload = self.load_chat_session_detail(session, session_id)
            changed = (
                payload.get("message_count") != previous_message_count
                or payload.get("last_message_at") != previous_last_message_at
            )
            if changed and payload.get("last_execution_status") in {"completed", "failed", "blocked"}:
                return payload
            time.sleep(self.args.poll_interval)
        raise CaseFailed(
            f"timed out waiting for chat completion after update: {session_id}"
        )

    def wait_for_task_completion(self, session: requests.Session, task_id: str) -> dict[str, Any]:
        deadline = time.time() + self.args.poll_timeout
        while time.time() < deadline:
            response, payload = self.get_json(session, f"/api/team/agent/tasks/{task_id}")
            self.expect(response.status_code == 200, f"failed to get task {task_id}")
            self.raise_if_transient_provider_block(payload, f"task {task_id}")
            if payload.get("status") == "failed" and self.payload_has_transient_provider_issue(payload):
                raise CaseBlocked(f"task {task_id} blocked by transient provider issue")
            if payload.get("status") in {"completed", "failed", "cancelled", "rejected"}:
                return payload
            time.sleep(self.args.poll_interval)
        raise CaseFailed(f"timed out waiting for task completion: {task_id}")

    def latest_provider_request(self) -> dict[str, Any]:
        candidate_dirs = [
            Path(self.args.logs_dir),
            Path("/root/.local/state/agime/logs"),
            Path("/root/.local/share/agime/logs"),
        ]
        log_path = next(
            (directory / "llm_request.0.jsonl" for directory in candidate_dirs if (directory / "llm_request.0.jsonl").exists()),
            None,
        )
        self.expect(log_path is not None, "provider request log not found in known log dirs")
        first_line = log_path.read_text(encoding="utf-8").splitlines()[0]
        return json.loads(first_line)

    def read_workspace_manifest(self, workspace_path: str) -> dict[str, Any]:
        manifest_path = Path(workspace_path) / "workspace.json"
        self.expect(manifest_path.exists(), f"workspace manifest missing: {manifest_path}")
        return json.loads(manifest_path.read_text(encoding="utf-8"))

    def runtime_diagnostics(self, payload: dict[str, Any] | None) -> dict[str, Any]:
        payload = payload or {}
        diagnostics = payload.get("runtime_diagnostics") or {}
        self.expect(isinstance(diagnostics, dict) and diagnostics, "runtime_diagnostics missing")
        return diagnostics

    def transition_records(
        self,
        diagnostics: dict[str, Any],
        *,
        required: bool = True,
    ) -> list[dict[str, Any]]:
        trace = diagnostics.get("transition_trace")
        if not isinstance(trace, dict):
            self.expect(not required, "transition_trace missing")
            return []
        records = trace.get("records") or []
        self.expect(
            isinstance(records, list) and (bool(records) or not required),
            "transition_trace.records missing",
        )
        return records

    def assert_transition_trace_valid(
        self,
        diagnostics: dict[str, Any],
        *,
        require_compaction_reason: bool = False,
        required: bool = True,
    ) -> list[dict[str, Any]]:
        records = self.transition_records(diagnostics, required=required)
        for record in records:
            kind = record.get("kind")
            self.expect(
                kind in HARNESS_TRANSITION_KINDS,
                f"unexpected transition kind observed: {kind}",
            )
        if require_compaction_reason and records:
            self.expect(
                any(
                    "compact" in (record.get("reason") or "")
                    or "recovery" in (record.get("reason") or "")
                    for record in records
                ),
                "transition trace never recorded a compact/recovery reason",
            )
        return records

    def assert_resume_selection_policy(self, diagnostics: dict[str, Any]) -> list[dict[str, Any]]:
        items = diagnostics.get("persisted_child_transcript_resume") or []
        self.expect(isinstance(items, list), "persisted_child_transcript_resume is not a list")
        for item in items:
            source = item.get("transcript_source") or ""
            self.expect(
                source.startswith("active:")
                or source.startswith("recent_terminal:")
                or source in {"live_child_session", "persisted_resume", "persisted_excerpt", "persisted_preview", "task_summary", "stale_persisted_resume"},
                f"unexpected transcript_source: {source}",
            )
        return items

    def assert_artifact_truth(
        self,
        *,
        workspace_path: str,
        expected_relative_paths: list[str],
        diagnostics: dict[str, Any] | None = None,
        require_manifest_index: bool = True,
    ) -> dict[str, Any]:
        manifest = self.read_workspace_manifest(workspace_path)
        artifact_index = manifest.get("artifact_index") or manifest.get("artifactIndex") or []
        indexed_paths = [item.get("path") for item in artifact_index if isinstance(item, dict)]
        for relative_path in expected_relative_paths:
            if require_manifest_index:
                self.expect(
                    relative_path in indexed_paths,
                    f"workspace manifest missing artifact index entry: {relative_path}",
                )
            self.expect(
                (Path(workspace_path) / relative_path).is_file(),
                f"workspace artifact file missing: {Path(workspace_path) / relative_path}",
            )
        if diagnostics:
            artifact_resolution = diagnostics.get("artifact_resolution") or {}
            if artifact_resolution:
                materialized = set(artifact_resolution.get("materialized_targets") or [])
                missing = set(artifact_resolution.get("missing_targets") or [])
                for relative_path in expected_relative_paths:
                    self.expect(
                        relative_path in materialized,
                    f"artifact resolution did not mark materialized target: {relative_path}",
                )
                self.expect(
                    relative_path not in missing,
                    f"artifact resolution incorrectly marked target missing: {relative_path}",
                )
        return {
            "workspace_path": workspace_path,
            "artifact_index": indexed_paths,
        }

    def extract_channel_runtime_diagnostics(self, thread_payload: dict[str, Any]) -> dict[str, Any]:
        top_level = thread_payload.get("runtime_diagnostics")
        if isinstance(top_level, dict) and top_level:
            return top_level
        messages = thread_payload.get("messages") or []
        for message in reversed(messages):
            metadata = message.get("metadata") or {}
            diagnostics = metadata.get("runtime_diagnostics")
            if isinstance(diagnostics, dict) and diagnostics:
                return diagnostics
        raise CaseFailed("channel thread missing runtime_diagnostics in message metadata")

    def resolve_portal_slug(self) -> str:
        if self.args.portal_slug:
            return self.args.portal_slug
        if self.context and self.context.portal_slug:
            return self.context.portal_slug
        portal = self.collection("portals").find_one(
            {"status": "published", "is_deleted": {"$ne": True}}
        )
        if not portal or not portal.get("slug"):
            raise CaseBlocked("no published portal found for team")
        return portal["slug"]

    def journal_contains(self, needle: str, limit: int = 2000) -> bool:
        proc = run_subprocess(
            [
                "journalctl",
                "-u",
                "agime-team-server",
                "-n",
                str(limit),
                "--no-pager",
                "-o",
                "cat",
            ],
            timeout=60,
        )
        haystack = ((proc.stdout or "") + "\n" + (proc.stderr or "")).lower()
        return needle.lower() in haystack

    def journal_has_transient_provider_issue(self) -> bool:
        return any(self.journal_contains(marker, limit=2000) for marker in self.TRANSIENT_PROVIDER_MARKERS)

    def extract_runtime_progress(
        self, payload: dict[str, Any] | None
    ) -> tuple[bool, bool, bool, int, int, dict[str, Any] | None]:
        payload = payload or {}
        summary = payload.get("context_runtime_summary")
        if summary:
            return (
                summary.get("stagedCollapseCount", 0) > 0,
                summary.get("committedCollapseCount", 0) > 0,
                bool(summary.get("sessionMemoryActive")),
                summary.get("runtimeCompactions", 0),
                (summary.get("lastProjectionStats") or {}).get("freedTokenEstimate", 0),
                summary,
            )

        state = payload.get("context_runtime_state") or payload
        store = state.get("store") or {}
        projection_stats = state.get("lastProjectionStats") or {}
        return (
            store.get("stagedSnapshot") is not None,
            bool(store.get("collapseCommits")),
            store.get("sessionMemory") is not None,
            state.get("runtimeCompactions", 0),
            projection_stats.get("freedTokenEstimate", 0),
            projection_stats,
        )

    def cleanup(self) -> None:
        print_step("cleanup temporary resources")
        if self.temp_api_keys:
            self.collection("api_keys").delete_many({"key_id": {"$in": list(self.temp_api_keys)}})
        if self.temp_auth_user_ids:
            self.collection("sessions").delete_many(
                {"user_id": {"$in": list(self.temp_auth_user_ids)}}
            )
            self.collection("api_keys").delete_many(
                {"user_id": {"$in": list(self.temp_auth_user_ids)}}
            )
            self.collection("users").delete_many(
                {"user_id": {"$in": list(self.temp_auth_user_ids)}}
            )
        if self.temp_tasks:
            self.collection("task_results").delete_many(
                {"task_id": {"$in": list(self.temp_tasks)}}
            )
            self.collection("agent_task_results").delete_many(
                {"task_id": {"$in": list(self.temp_tasks)}}
            )
            self.collection("agent_tasks").delete_many({"id": {"$in": list(self.temp_tasks)}})
        if self.temp_chat_sessions:
            self.collection("agent_chat_events").delete_many(
                {"session_id": {"$in": list(self.temp_chat_sessions)}}
            )
            self.collection("agent_sessions").delete_many(
                {"session_id": {"$in": list(self.temp_chat_sessions)}}
            )
        if self.temp_channels:
            self.collection("chat_channel_events").delete_many(
                {"channel_id": {"$in": list(self.temp_channels)}}
            )
            self.collection("chat_channel_messages").delete_many(
                {"channel_id": {"$in": list(self.temp_channels)}}
            )
            self.collection("chat_channel_reads").delete_many(
                {"channel_id": {"$in": list(self.temp_channels)}}
            )
            self.collection("chat_channel_members").delete_many(
                {"channel_id": {"$in": list(self.temp_channels)}}
            )
            self.collection("chat_channels").delete_many(
                {"channel_id": {"$in": list(self.temp_channels)}}
            )
        if self.temp_scheduled_tasks:
            self.collection("scheduled_task_runs").delete_many(
                {"task_id": {"$in": list(self.temp_scheduled_tasks)}}
            )
            self.collection("scheduled_tasks").delete_many(
                {"task_id": {"$in": list(self.temp_scheduled_tasks)}}
            )
        if self.temp_documents:
            self.collection("document_versions").delete_many(
                {"document_id": {"$in": list(self.temp_documents)}}
            )
            self.collection("documents").delete_many({"id": {"$in": list(self.temp_documents)}})
        if self.temp_visitors:
            visitor_user_ids = [f"portal_visitor_{vid}" for vid in self.temp_visitors]
            self.collection("external_users").delete_many(
                {"user_id": {"$in": visitor_user_ids}}
            )
            self.collection("agent_sessions").delete_many(
                {"user_id": {"$in": visitor_user_ids}}
            )
            self.collection("agent_chat_events").delete_many(
                {"user_id": {"$in": visitor_user_ids}}
            )
        if self.temp_web_sessions:
            self.collection("sessions").delete_many(
                {"session_id": {"$in": list(self.temp_web_sessions)}}
            )

    def run_rust_checks(
        self,
        *,
        include_harness_targets: bool = False,
        include_scheduled_task_targets: bool = False,
        scheduled_tasks_only: bool = False,
    ) -> list[str]:
        workspace = self.args.workspace
        targeted_tests = ([] if scheduled_tasks_only else TARGETED_RUST_TESTS) + (
            HARNESS_TARGETED_RUST_TESTS if include_harness_targets else []
        ) + (
            SCHEDULED_TASK_TARGETED_RUST_TESTS
            if include_scheduled_task_targets
            else []
        )
        result = run_subprocess(
            ["/root/.cargo/bin/cargo", "check", "-p", "agime-team-server", "--quiet"],
            cwd=workspace,
            timeout=3600,
        )
        if result.returncode != 0:
            raise CaseFailed(f"cargo check failed:\n{result.stderr}")
        for package_name, test_name in targeted_tests:
            result = run_subprocess(
                ["/root/.cargo/bin/cargo", "test", "-p", package_name, test_name, "--quiet"],
                cwd=workspace,
                timeout=3600,
            )
            if result.returncode != 0:
                raise CaseFailed(
                    f"targeted rust test failed: {package_name}:{test_name}\n{result.stderr}"
                )
        return [test_name for _, test_name in targeted_tests]

    def run_frontend_build(self) -> None:
        result = run_subprocess(
            ["npm", "run", "build", "--", "--mode", "production"],
            cwd=os.path.join(self.args.workspace, "crates", "agime-team-server", "web-admin"),
            timeout=3600,
        )
        if result.returncode != 0:
            raise CaseFailed(f"frontend build failed:\n{result.stderr}")

    def rust_checks_case_result(
        self,
        *,
        include_harness_targets: bool = False,
        include_scheduled_task_targets: bool = False,
        scheduled_tasks_only: bool = False,
    ) -> CaseResult:
        tests = self.run_rust_checks(
            include_harness_targets=include_harness_targets,
            include_scheduled_task_targets=include_scheduled_task_targets,
            scheduled_tasks_only=scheduled_tasks_only,
        )
        return CaseResult(
            name="cargo-check-and-targeted-tests",
            entry="rust",
            status="passed",
            evidence={"tests": tests},
        )

    def case_health(self) -> CaseResult:
        response = requests.get(f"{self.base_url}/health", timeout=15)
        payload = response.json()
        self.expect(response.status_code == 200, "health endpoint did not return 200")
        self.expect(payload.get("status") == "healthy", "health status is not healthy")
        return CaseResult(
            name="health",
            entry="public",
            status="passed",
            evidence={"status": payload.get("status"), "database": payload.get("database")},
        )

    def case_cookie_auth(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        session_id, auth = self.create_temp_web_session(self.context.owner_user_id)
        response, payload = self.get_json(auth, "/api/auth/session")
        self.expect(response.status_code == 200, "cookie auth session lookup failed")
        self.expect(
            payload.get("user", {}).get("id") == self.context.owner_user_id,
            "session user mismatch",
        )
        return CaseResult(
            name="cookie-auth",
            entry="auth",
            status="passed",
            ids={"session_id": session_id},
            evidence={"user_id": payload["user"]["id"]},
            cleanup="deleted web session",
        )

    def case_invalid_session(self) -> CaseResult:
        bogus = requests.Session()
        bogus.cookies.set("agime_session", f"codex-e2e-invalid-{uuid.uuid4().hex[:12]}")
        response, payload = self.get_json(bogus, "/api/auth/session")
        self.expect(response.status_code == 401, "invalid session should return 401")
        return CaseResult(
            name="invalid-session",
            entry="auth",
            status="passed",
            evidence={"status_code": response.status_code, "error": payload.get("error")},
        )

    def case_session_sliding_renewal(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        session_id = str(uuid.uuid4())
        now = utcnow()
        old_expiry = now + timedelta(minutes=30)
        self.collection("sessions").insert_one(
            {
                "session_id": session_id,
                "user_id": self.context.owner_user_id,
                "created_at": now,
                "expires_at": old_expiry,
            }
        )
        self.temp_web_sessions.add(session_id)
        auth = requests.Session()
        auth.cookies.set("agime_session", session_id)
        response, _ = self.get_json(
            auth,
            "/api/team/agent/runtime-profile-preview",
            params={"model": "GLM-5.1", "api_format": "anthropic"},
        )
        self.expect(
            response.status_code == 200,
            "protected route did not accept sliding-renewal session",
        )
        deadline = time.time() + 10
        new_expiry = old_expiry
        while time.time() < deadline:
            doc = self.collection("sessions").find_one({"session_id": session_id})
            if doc and doc.get("expires_at"):
                candidate = doc["expires_at"].replace(tzinfo=timezone.utc)
                if candidate > old_expiry + timedelta(days=1):
                    new_expiry = candidate
                    break
            time.sleep(1)
        self.expect(
            new_expiry > old_expiry + timedelta(days=1),
            "session sliding renewal did not extend expiry",
        )
        return CaseResult(
            name="session-sliding-renewal",
            entry="auth",
            status="passed",
            ids={"session_id": session_id},
            evidence={
                "old_expires_at": old_expiry.isoformat(),
                "new_expires_at": new_expiry.isoformat(),
            },
            cleanup="deleted web session",
        )

    def case_api_key_login(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, owner = self.create_temp_web_session(self.context.owner_user_id)
        self.collection("api_keys").delete_many({"name": {"$regex": "^codex-e2e-key"}})
        create_resp, create_payload = self.post_json(
            owner,
            "/api/auth/keys",
            {"name": short_uid("codex-e2e-key"), "expires_in_days": 1},
        )
        self.expect(create_resp.status_code == 201, "create api key failed")
        key_payload = create_payload.get("key", create_payload)
        key_id = (
            key_payload.get("id")
            or key_payload.get("key_id")
            or key_payload.get("apiKeyId")
        )
        self.expect(key_id is not None, "create api key response missing key id")
        api_key = key_payload["api_key"]
        self.temp_api_keys.add(key_id)
        login_resp, login_payload = self.post_json(
            requests.Session(),
            "/api/auth/login",
            {"api_key": api_key},
        )
        self.expect(login_resp.status_code == 200, "api key login failed")
        self.expect(
            "agime_session" in login_resp.cookies.get_dict(),
            "api key login did not set session cookie",
        )
        return CaseResult(
            name="api-key-login",
            entry="auth",
            status="passed",
            ids={"key_id": key_id},
            evidence={"user_id": login_payload["user"]["id"]},
            cleanup="api key scheduled for deletion",
        )

    def case_password_login(self) -> CaseResult:
        email = self.args.password_email or os.environ.get("AGIME_E2E_PASSWORD_EMAIL")
        password = self.args.password or os.environ.get("AGIME_E2E_PASSWORD")
        temp_user_id: str | None = None
        if not email or not password:
            temp_user_id, email, password = self.create_temp_password_user()
        login_session = requests.Session()
        response, payload = self.post_json(
            login_session,
            "/api/auth/login/password",
            {"email": email, "password": password},
        )
        self.expect(response.status_code == 200, "password login failed")
        self.expect(
            "agime_session" in response.cookies.get_dict(),
            "password login did not set session cookie",
        )
        cleanup = "not_required"
        if temp_user_id is not None:
            deactivate = login_session.post(
                f"{self.base_url}/api/auth/deactivate",
                json={},
                timeout=30,
            )
            self.expect(
                deactivate.status_code == 200,
                f"temporary password user deactivation failed: {deactivate.text}",
            )
            cleanup = "temporary password user scheduled for deletion"
        return CaseResult(
            name="password-login",
            entry="auth",
            status="passed",
            evidence={
                "user_id": payload["user"]["id"],
                "email": payload["user"]["email"],
                "ephemeral_user": temp_user_id is not None,
            },
            cleanup=cleanup,
        )

    def case_runtime_profile_preview(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        response, payload = self.get_json(
            auth,
            "/api/team/agent/runtime-profile-preview",
            params={"model": "GLM-5.1", "api_format": "anthropic"},
        )
        self.expect(response.status_code == 200, "runtime-profile-preview failed")
        runtime_caps = payload.get("runtimeCapabilities") or {}
        self.expect(
            runtime_caps.get("contextLength"),
            "runtime-profile-preview missing runtimeCapabilities.contextLength",
        )
        self.expect(payload.get("userIntent") is not None, "runtime-profile-preview missing userIntent")
        self.expect(
            payload.get("effectiveExecution") is not None,
            "runtime-profile-preview missing effectiveExecution",
        )
        self.expect(payload.get("sourceBreakdown") is not None, "runtime-profile-preview missing sourceBreakdown")
        return CaseResult(
            name="runtime-profile-preview",
            entry="agent",
            status="passed",
            evidence={
                "contextLength": runtime_caps.get("contextLength"),
                "supportsPromptCaching": runtime_caps.get("supportsPromptCaching"),
                "supportsCacheEdit": runtime_caps.get("supportsCacheEdit"),
                "providerMode": (payload.get("sourceBreakdown") or {}).get("providerMode"),
            },
        )

    def case_agent_intent_preserved_with_preview_downgrade(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        original = self.patch_agent_for_heavy_runtime(auth)
        try:
            response, payload = self.get_json(
                auth,
                f"/api/team/agent/agents/{self.context.agent_id}",
            )
            self.expect(response.status_code == 200, "failed to reload updated agent")
            expected_budget = payload.get("thinking_budget") or 768
            self.expect(payload.get("thinking_budget") == expected_budget, "agent thinking_budget was not preserved")
            self.expect(payload.get("cache_edit_mode") == "prefer", "agent cache_edit_mode was not preserved")

            response, preview = self.get_json(
                auth,
                "/api/team/agent/runtime-profile-preview",
                params={
                    "model": payload.get("model") or "GLM-5.1",
                    "api_format": payload.get("api_format") or "anthropic",
                    "api_url": payload.get("api_url") or "",
                    "thinking_enabled": str(payload.get("thinking_enabled", True)).lower(),
                    "thinking_budget": expected_budget,
                    "reasoning_effort": payload.get("reasoning_effort") or "high",
                    "prompt_caching_mode": payload.get("prompt_caching_mode") or "off",
                    "cache_edit_mode": payload.get("cache_edit_mode") or "prefer",
                    "context_limit": payload.get("context_limit") or 12000,
                    "max_tokens": payload.get("max_tokens") or 768,
                },
            )
            self.expect(response.status_code == 200, "runtime-profile-preview downgrade probe failed")
            self.expect(
                (preview.get("userIntent") or {}).get("thinkingBudget") == expected_budget,
                "preview did not preserve user thinking budget intent",
            )
            self.expect(
                (preview.get("effectiveExecution") or {}).get("thinkingBudget") == expected_budget,
                "preview effectiveExecution did not preserve requested thinking budget",
            )
            return CaseResult(
                name="agent-intent-preserved-with-preview-downgrade",
                entry="agent",
                status="passed",
                ids={"agent_id": self.context.agent_id},
                evidence={
                    "preserved_values": {
                        "thinking_budget": payload.get("thinking_budget"),
                        "cache_edit_mode": payload.get("cache_edit_mode"),
                    },
                    "downgrades": preview.get("downgrades"),
                    "warnings": preview.get("warnings"),
                    "effectiveExecution": preview.get("effectiveExecution"),
                },
                cleanup="agent restored to original runtime settings",
            )
        finally:
            self.restore_agent(auth, original)

    def case_runtime_profile_litellm_modes(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        direct_response, direct_payload = self.get_json(
            auth,
            "/api/team/agent/runtime-profile-preview",
            params={"model": "gpt-4o-mini", "api_format": "litellm"},
        )
        compat_response, compat_payload = self.get_json(
            auth,
            "/api/team/agent/runtime-profile-preview",
            params={
                "model": "gpt-4o-mini",
                "api_format": "openai",
                "api_url": "http://localhost:4000/v1/chat/completions",
            },
        )
        self.expect(direct_response.status_code == 200, "LiteLLM direct preview failed")
        self.expect(compat_response.status_code == 200, "LiteLLM compat preview failed")
        self.expect(
            (direct_payload.get("sourceBreakdown") or {}).get("providerMode") == "litellm",
            "LiteLLM direct preview provider mode mismatch",
        )
        self.expect(
            (compat_payload.get("sourceBreakdown") or {}).get("providerMode") == "litellm_openai_compat",
            "LiteLLM compat preview provider mode mismatch",
        )
        return CaseResult(
            name="runtime-profile-litellm-modes",
            entry="agent",
            status="passed",
            evidence={
                "direct_provider_mode": (direct_payload.get("sourceBreakdown") or {}).get("providerMode"),
                "compat_provider_mode": (compat_payload.get("sourceBreakdown") or {}).get("providerMode"),
            },
        )

    def create_authenticated_chat_session(
        self,
        auth: requests.Session,
        attached_document_ids: list[str] | None = None,
        allowed_extensions: list[str] | None = None,
    ) -> str:
        self.expect(self.context is not None, "team context not initialized")
        response, payload = self.post_json(
            auth,
            "/api/team/agent/chat/sessions",
            {
                "agent_id": self.context.agent_id,
                "attached_document_ids": attached_document_ids or [],
                "allowed_extensions": allowed_extensions,
            },
        )
        self.expect(response.status_code == 200, "create chat session failed")
        session_id = payload["session_id"]
        self.temp_chat_sessions.add(session_id)
        return session_id

    def send_authenticated_chat_message(
        self, auth: requests.Session, session_id: str, content: str
    ) -> None:
        response, payload = self.post_json(
            auth,
            f"/api/team/agent/chat/sessions/{session_id}/messages",
            {"content": content},
        )
        self.expect(response.status_code == 200, f"send chat message failed: {payload}")

    def patch_agent_for_heavy_runtime(self, auth: requests.Session) -> dict[str, Any]:
        self.expect(self.context is not None, "team context not initialized")
        current = self.collection("team_agents").find_one(
            {"$or": [{"id": self.context.agent_id}, {"agent_id": self.context.agent_id}]}
        )
        self.expect(current is not None, "agent not found in Mongo")
        original = {
            "max_tokens": current.get("max_tokens"),
            "context_limit": current.get("context_limit"),
            "thinking_budget": current.get("thinking_budget"),
            "output_reserve_tokens": current.get("output_reserve_tokens"),
            "auto_compact_threshold": current.get("auto_compact_threshold"),
            "prompt_caching_mode": current.get("prompt_caching_mode", "auto"),
            "cache_edit_mode": current.get("cache_edit_mode", "auto"),
            "reasoning_effort": current.get("reasoning_effort"),
        }
        update = {
            "max_tokens": 512,
            "context_limit": 7000,
            "thinking_budget": 512,
            "output_reserve_tokens": 800,
            "auto_compact_threshold": 0.05,
            "prompt_caching_mode": "off",
            "cache_edit_mode": "prefer",
            "reasoning_effort": "low",
        }
        response, payload = self.put_json(
            auth,
            f"/api/team/agent/agents/{self.context.agent_id}",
            update,
        )
        self.expect(
            response.status_code == 200,
            f"failed to patch agent for heavy runtime: {payload}",
        )
        return original

    def restore_agent(self, auth: requests.Session, original: dict[str, Any]) -> None:
        self.expect(self.context is not None, "team context not initialized")
        response, payload = self.put_json(
            auth,
            f"/api/team/agent/agents/{self.context.agent_id}",
            original,
        )
        self.expect(response.status_code == 200, f"failed to restore agent config: {payload}")

    def case_harness_direct_chat_baseline(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        marker = short_uid("harness-direct")
        session_id = self.create_authenticated_chat_session(auth)
        self.send_authenticated_chat_message(
            auth,
            session_id,
            f"Please reply with exactly HARNESS_BASELINE_OK-{marker} and nothing else.",
        )
        self.wait_for_chat_completion(auth, session_id)
        detail = self.load_chat_session_detail(auth, session_id)
        diagnostics = self.runtime_diagnostics(detail)
        transition_records = self.assert_transition_trace_valid(diagnostics, required=True)
        events = self.load_chat_session_events(auth, session_id)
        runtime_summary = self.extract_runtime_progress(detail)[5]
        self.expect(
            detail.get("last_execution_status") == "completed",
            f"direct chat baseline did not complete: {detail.get('last_execution_status')}",
        )
        self.expect(
            (detail.get("context_runtime_state") or {}).get("schemaVersion") == 6,
            "direct chat baseline did not persist V6 runtime state",
        )
        self.remember_case_ids("harness-direct-chat-baseline", {"session_id": session_id})
        return CaseResult(
            name="harness-direct-chat-baseline",
            entry="harness",
            status="passed",
            ids={"session_id": session_id},
            evidence={
                "runtime_summary": runtime_summary,
                "transition_trace": {
                    "record_count": len(transition_records),
                    "kinds": [record.get("kind") for record in transition_records],
                },
                "event_count": len(events),
            },
            cleanup="agent session scheduled for deletion",
        )

    def case_harness_direct_chat_heavy_runtime(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        marker = short_uid("harness-heavy-direct")
        original = self.patch_agent_for_heavy_runtime(auth)
        try:
            session_id = self.create_authenticated_chat_session(auth)
            observed_stage = False
            observed_commit = False
            observed_memory = False
            last_detail: dict[str, Any] = self.load_chat_session_detail(auth, session_id)
            heavy_text = (f"{marker} " * 520).strip()

            def wait_for_http_session_update(previous_detail: dict[str, Any]) -> dict[str, Any]:
                previous_message_count = previous_detail.get("message_count")
                previous_last_message_at = previous_detail.get("last_message_at")
                deadline = time.time() + self.args.poll_timeout
                while time.time() < deadline:
                    payload = self.load_chat_session_detail(auth, session_id)
                    if payload.get("last_execution_status") in {"completed", "failed", "blocked"} and (
                        payload.get("message_count") != previous_message_count
                        or payload.get("last_message_at") != previous_last_message_at
                        or payload.get("last_execution_status") == "blocked"
                    ):
                        return payload
                    time.sleep(self.args.poll_interval)
                raise CaseFailed(f"timed out waiting for HTTP session update: {session_id}")

            for turn in range(1, 5):
                print_step(f"harness heavy direct turn {turn}")
                self.send_authenticated_chat_message(
                    auth,
                    session_id,
                    f"[marker:{marker}] [turn {turn}] {heavy_text}\nAcknowledge briefly.",
                )
                last_detail = wait_for_http_session_update(last_detail)
                stage, commit, memory, _, _, _ = self.extract_runtime_progress(last_detail)
                observed_stage = observed_stage or stage
                observed_commit = observed_commit or commit
                observed_memory = observed_memory or memory
                if observed_commit and observed_memory:
                    break
            if not observed_memory:
                self.send_authenticated_chat_message(auth, session_id, "/compact")
                last_detail = wait_for_http_session_update(last_detail)
                stage, commit, memory, _, _, _ = self.extract_runtime_progress(last_detail)
                observed_stage = observed_stage or stage
                observed_commit = observed_commit or commit
                observed_memory = observed_memory or memory

            detail = self.load_chat_session_detail(auth, session_id)
            diagnostics = self.runtime_diagnostics(detail)
            transition_records = self.assert_transition_trace_valid(
                diagnostics,
                require_compaction_reason=True,
                required=True,
            )
            runtime_summary = self.extract_runtime_progress(detail)[5]
            self.expect(
                observed_stage or observed_commit or observed_memory,
                "heavy direct chat never showed runtime compaction progression",
            )
            self.expect(
                observed_commit or observed_memory,
                "heavy direct chat never reached committed/runtime memory state",
            )
            self.remember_case_ids("harness-direct-chat-heavy-runtime", {"session_id": session_id})
            return CaseResult(
                name="harness-direct-chat-heavy-runtime",
                entry="harness",
                status="passed",
                ids={"session_id": session_id},
                evidence={
                    "runtime_summary": runtime_summary,
                    "transition_trace": {
                        "record_count": len(transition_records),
                        "reasons": [record.get("reason") for record in transition_records],
                    },
                },
                cleanup="agent session scheduled for deletion and agent restored",
            )
        finally:
            self.restore_agent(auth, original)

    def case_harness_task_runtime(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        marker = short_uid("harness-task")
        session_id = self.create_authenticated_chat_session(auth)
        self.send_authenticated_chat_message(
            auth,
            session_id,
            "\n".join(
                [
                    "You must use exactly one bounded subagent.",
                    "Do not answer from your own memory.",
                    "Use that subagent to inspect whether README.md exists in the workspace.",
                    f"Return exactly HARNESS_TASK_OK-{marker}: yes or no.",
                ]
            ),
        )
        detail = self.wait_for_chat_completion(auth, session_id)
        diagnostics = self.runtime_diagnostics(detail)
        transition_records = self.assert_transition_trace_valid(diagnostics, required=True)
        child_evidence = detail.get("persisted_child_evidence") or []
        child_resume = self.assert_resume_selection_policy(diagnostics)
        delegation_runtime = detail.get("delegation_runtime") or {}
        workers = delegation_runtime.get("workers") or []
        self.expect(
            bool(child_evidence) or bool(child_resume),
            "task runtime did not expose durable child recovery truth",
        )
        self.remember_case_ids(
            "harness-task-runtime",
            {
                "session_id": session_id,
            },
        )
        return CaseResult(
            name="harness-task-runtime",
            entry="harness",
            status="passed",
            ids={"session_id": session_id},
            evidence={
                "runtime_summary": self.extract_runtime_progress(detail)[5],
                "transition_trace": {
                    "record_count": len(transition_records),
                    "kinds": [record.get("kind") for record in transition_records],
                },
                "resume_view": {
                    "child_evidence_count": len(child_evidence),
                    "child_transcript_resume_count": len(child_resume),
                },
                "delegation_runtime": {
                    "mode": delegation_runtime.get("mode"),
                    "worker_count": len(workers),
                    "status": delegation_runtime.get("status"),
                },
                "selection_policy": [
                    item.get("transcript_source")
                    for item in child_resume
                    if isinstance(item, dict)
                ],
            },
            cleanup="chat session scheduled for deletion",
        )

    def case_harness_task_artifact_truth(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        marker = short_uid("harness-artifact")
        artifact_path = f"artifacts/{marker}.md"
        session_id = self.create_authenticated_chat_session(auth)
        self.send_authenticated_chat_message(
            auth,
            session_id,
            "\n".join(
                [
                    f"Create the workspace artifact file {artifact_path}.",
                    f"The file must contain exactly one line: MARKER={marker}.",
                    f"After the file exists, reply with exactly HARNESS_ARTIFACT_OK-{marker}.",
                ]
            ),
        )
        detail = self.wait_for_chat_completion(auth, session_id)
        diagnostics = self.runtime_diagnostics(detail)
        workspace_path = detail.get("workspace_path")
        self.expect(bool(workspace_path), "artifact truth task did not expose workspace_path")
        artifact_truth = self.assert_artifact_truth(
            workspace_path=workspace_path,
            expected_relative_paths=[artifact_path],
            diagnostics=diagnostics,
            require_manifest_index=True,
        )
        self.expect(
            bool(diagnostics.get("artifact_resolution")),
            "artifact truth task did not expose artifact_resolution diagnostics",
        )
        self.remember_case_ids(
            "harness-task-artifact-truth",
            {
                "session_id": session_id,
                "workspace_path": workspace_path,
                "artifact_path": artifact_path,
            },
        )
        return CaseResult(
            name="harness-task-artifact-truth",
            entry="harness",
            status="passed",
            ids={"session_id": session_id},
            evidence={
                "artifact_truth": artifact_truth,
                "artifact_resolution": diagnostics.get("artifact_resolution"),
            },
            cleanup="chat session scheduled for deletion",
        )

    def case_harness_task_cancel(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        marker = short_uid("harness-cancel")
        original = self.patch_agent_for_heavy_runtime(auth)
        try:
            session_id = self.create_authenticated_chat_session(auth)
            heavy_text = (f"{marker} " * 700).strip()
            self.send_authenticated_chat_message(
                auth,
                session_id,
                "\n".join(
                    [
                        "Use swarm with parallel workers.",
                        f"[marker:{marker}] {heavy_text}",
                        "Stay busy until cancelled.",
                    ]
                ),
            )
            self.wait_for_chat_status(auth, session_id, {"running"}, require_processing=True)
            self.cancel_chat_session(auth, session_id)
            time.sleep(self.args.poll_interval * 2)
            detail = self.load_chat_session_detail(auth, session_id)
            diagnostics = detail.get("runtime_diagnostics") or {}
            self.expect(
                detail.get("last_execution_status") == "cancelled",
                f"cancelled chat session did not settle as cancelled: {detail.get('last_execution_status')}",
            )
            if diagnostics:
                self.expect(
                    diagnostics.get("status") == "cancelled",
                    f"cancelled chat runtime_diagnostics did not settle as cancelled: {diagnostics.get('status')}",
                )
            self.remember_case_ids(
                "harness-task-cancel",
                {"session_id": session_id},
            )
            return CaseResult(
                name="harness-task-cancel",
                entry="harness",
                status="passed",
                ids={"session_id": session_id},
                evidence={
                    "cancel_path": {
                        "is_processing": detail.get("is_processing"),
                        "last_execution_status": detail.get("last_execution_status"),
                        "runtime_diagnostics_status": diagnostics.get("status"),
                    }
                },
                cleanup="chat session scheduled for deletion and agent restored",
            )
        finally:
            self.restore_agent(auth, original)

    def case_harness_channel_runtime(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        marker = short_uid("harness-channel")
        channel_name = short_uid("harness-channel-room")
        response, payload = self.post_json(
            auth,
            "/api/team/agent/chat/channels",
            {
                "name": channel_name,
                "channel_type": "general",
                "default_agent_id": self.context.agent_id,
                "member_user_ids": [],
            },
            params={"team_id": self.context.team_id},
        )
        self.expect(response.status_code == 200, f"create channel failed: {payload}")
        channel_id = payload["channel"]["channel_id"]
        self.temp_channels.add(channel_id)
        response, first_payload = self.post_json(
            auth,
            f"/api/team/agent/chat/channels/{channel_id}/messages",
            {
                "content": f"Please reply with exactly HARNESS_CHANNEL_OK-{marker} and nothing else.",
                "surface": "temporary",
            },
        )
        self.expect(response.status_code == 200, "channel message failed")
        root_message_id = first_payload["root_message_id"]

        deadline = time.time() + self.args.poll_timeout
        thread_payload: dict[str, Any] | None = None
        while time.time() < deadline:
            thread_payload = self.load_channel_thread(auth, channel_id, root_message_id)
            thread_messages = thread_payload.get("messages") or []
            if any(
                item.get("author_type") in {"agent", "system"}
                and f"HARNESS_CHANNEL_OK-{marker}" in (item.get("content_text") or "")
                for item in thread_messages
            ):
                break
            time.sleep(self.args.poll_interval)
        self.expect(thread_payload is not None, "channel thread payload missing")
        diagnostics = self.extract_channel_runtime_diagnostics(thread_payload)
        transition_records = self.assert_transition_trace_valid(diagnostics, required=True)
        self.assert_resume_selection_policy(diagnostics)
        self.remember_case_ids(
            "harness-channel-runtime",
            {"channel_id": channel_id, "root_message_id": root_message_id},
        )
        return CaseResult(
            name="harness-channel-runtime",
            entry="harness",
            status="passed",
            ids={"channel_id": channel_id},
            evidence={
                "runtime_summary": diagnostics.get("summary"),
                "transition_trace": {
                    "record_count": len(transition_records),
                    "kinds": [record.get("kind") for record in transition_records],
                },
                "selection_policy": [
                    item.get("transcript_source")
                    for item in (diagnostics.get("persisted_child_transcript_resume") or [])
                    if isinstance(item, dict)
                ],
                "root_message_id": root_message_id,
            },
            cleanup="channel scheduled for deletion",
        )

    def case_harness_chat_memory_allowlist(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        marker = short_uid("harness-memory")
        session_id = self.create_authenticated_chat_session(
            auth, allowed_extensions=["document_tools"]
        )
        self.send_authenticated_chat_message(
            auth,
            session_id,
            f"Please reply with exactly HARNESS_MEMORY_OK-{marker} and nothing else.",
        )
        detail = self.wait_for_chat_completion(auth, session_id)
        capability_snapshot = detail.get("capability_snapshot") or {}
        extensions = capability_snapshot.get("extensions") or {}
        allowed_names = (
            extensions.get("effectiveAllowedExtensionNames")
            or extensions.get("effective_allowed_extension_names")
            or []
        )
        injected_capabilities = (
            extensions.get("sessionInjectedCapabilities")
            or extensions.get("session_injected_capabilities")
            or []
        )
        injected_runtime_names: list[str] = []
        for item in injected_capabilities:
            runtime_names = item.get("runtimeNames") or item.get("runtime_names") or []
            injected_runtime_names.extend(runtime_names)
        diagnostics = self.runtime_diagnostics(detail)
        transition_records = self.assert_transition_trace_valid(diagnostics, required=True)
        self.expect(
            "document_tools" in allowed_names and "chat_memory" in allowed_names,
            "chat memory allowlist lost expected runtime extensions",
        )
        self.expect(
            "chat_memory" in injected_runtime_names,
            "chat memory capability was not injected into runtime snapshot",
        )
        self.remember_case_ids("harness-chat-memory-allowlist", {"session_id": session_id})
        return CaseResult(
            name="harness-chat-memory-allowlist",
            entry="harness",
            status="passed",
            ids={"session_id": session_id},
            evidence={
                "transition_trace": {
                    "record_count": len(transition_records),
                    "kinds": [record.get("kind") for record in transition_records],
                },
                "selection_policy": [],
                "allowed_extensions": detail.get("allowed_extensions"),
                "effective_allowed_extension_names": allowed_names,
                "session_injected_runtime_names": injected_runtime_names,
            },
            cleanup="agent session scheduled for deletion",
        )

    def case_harness_runtime_diagnostics_audit(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        direct_detail = self.load_chat_session_detail(
            auth, self.case_ids("harness-direct-chat-baseline")["session_id"]
        )
        direct_records = self.assert_transition_trace_valid(
            self.runtime_diagnostics(direct_detail),
            required=True,
        )

        task_runtime_ids = self.case_ids("harness-task-runtime")
        task_runtime_detail = self.load_chat_session_detail(auth, task_runtime_ids["session_id"])
        task_runtime_diagnostics = self.runtime_diagnostics(task_runtime_detail)
        task_runtime_records = self.assert_transition_trace_valid(
            task_runtime_diagnostics,
            required=True,
        )
        task_runtime_resume = self.assert_resume_selection_policy(task_runtime_diagnostics)
        task_runtime_delegation = task_runtime_detail.get("delegation_runtime") or {}
        self.expect(
            bool(task_runtime_detail.get("persisted_child_evidence"))
            or bool(task_runtime_resume),
            "runtime diagnostics audit did not find durable child recovery truth on harness task runtime session",
        )

        artifact_ids = self.case_ids("harness-task-artifact-truth")
        artifact_detail = self.load_chat_session_detail(auth, artifact_ids["session_id"])
        artifact_diagnostics = self.runtime_diagnostics(artifact_detail)
        artifact_resolution = artifact_diagnostics.get("artifact_resolution") or {}
        artifact_truth = self.assert_artifact_truth(
            workspace_path=artifact_ids["workspace_path"],
            expected_relative_paths=[artifact_ids["artifact_path"]],
            diagnostics=artifact_diagnostics if artifact_resolution else None,
            require_manifest_index=True,
        )
        self.expect(
            bool(artifact_resolution),
            "runtime diagnostics audit did not find artifact_resolution snapshot",
        )

        channel_ids = self.case_ids("harness-channel-runtime")
        channel_thread = self.load_channel_thread(
            auth, channel_ids["channel_id"], channel_ids["root_message_id"]
        )
        channel_diagnostics = self.extract_channel_runtime_diagnostics(channel_thread)
        channel_records = self.assert_transition_trace_valid(
            channel_diagnostics,
            required=True,
        )
        channel_resume = self.assert_resume_selection_policy(channel_diagnostics)
        self.expect(
            len(direct_records) > 0
            and len(task_runtime_records) > 0
            and len(channel_records) > 0,
            "runtime diagnostics audit found an empty transition trace on at least one harness surface",
        )

        return CaseResult(
            name="harness-runtime-diagnostics-audit",
            entry="harness",
            status="passed",
            evidence={
                "runtime_summary": {
                    "direct_chat_transition_count": len(direct_records),
                    "task_runtime_transition_count": len(task_runtime_records),
                    "channel_transition_count": len(channel_records),
                },
                "transition_trace": {
                    "direct_chat_kinds": [record.get("kind") for record in direct_records],
                    "task_runtime_kinds": [record.get("kind") for record in task_runtime_records],
                    "channel_kinds": [record.get("kind") for record in channel_records],
                },
                "resume_view": {
                    "task_runtime_sources": [
                        item.get("transcript_source") for item in task_runtime_resume
                    ],
                    "channel_sources": [item.get("transcript_source") for item in channel_resume],
                },
                "artifact_truth": artifact_truth,
            },
            cleanup="not_required",
        )

    def case_direct_chat(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        session_id = self.create_authenticated_chat_session(auth)
        self.send_authenticated_chat_message(
            auth, session_id, "Please reply with exactly CHAT_OK and nothing else."
        )
        self.wait_for_chat_completion(auth, session_id)
        response, detail = self.get_json(
            auth, f"/api/team/agent/chat/sessions/{session_id}"
        )
        self.expect(response.status_code == 200, "failed to refresh chat session detail")
        response, events = self.get_json(
            auth, f"/api/team/agent/chat/sessions/{session_id}/events"
        )
        self.expect(response.status_code == 200, "failed to fetch chat events")
        state = detail.get("context_runtime_state") or {}
        self.expect(
            state.get("schemaVersion") == 6,
            "chat session did not persist V6 runtime state",
        )
        self.expect(
            detail.get("document_access_mode") == "full",
            f"internal direct chat did not retain full document access: {detail.get('document_access_mode')}",
        )
        self.expect(
            detail.get("document_scope_mode") == "full",
            f"internal direct chat did not retain full document scope: {detail.get('document_scope_mode')}",
        )
        self.expect(
            detail.get("document_write_mode") == "full_write",
            f"internal direct chat did not retain full document write mode: {detail.get('document_write_mode')}",
        )
        return CaseResult(
            name="direct-chat",
            entry="chat",
            status="passed",
            ids={"session_id": session_id},
            evidence={
                "last_execution_status": detail.get("last_execution_status"),
                "schemaVersion": state.get("schemaVersion"),
                "document_access_mode": detail.get("document_access_mode"),
                "document_scope_mode": detail.get("document_scope_mode"),
                "document_write_mode": detail.get("document_write_mode"),
                "event_count": len(events) if isinstance(events, list) else None,
            },
            cleanup="agent session scheduled for deletion",
        )

    def case_chat_memory_allowlist(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        session_id = self.create_authenticated_chat_session(
            auth, allowed_extensions=["document_tools"]
        )
        self.send_authenticated_chat_message(
            auth,
            session_id,
            "Please reply with exactly CHAT_MEMORY_OK and nothing else.",
        )
        detail = self.wait_for_chat_completion(auth, session_id)
        capability_snapshot = detail.get("capability_snapshot") or {}
        extensions = capability_snapshot.get("extensions") or {}
        allowed_names = (
            extensions.get("effectiveAllowedExtensionNames")
            or extensions.get("effective_allowed_extension_names")
            or []
        )
        injected_capabilities = (
            extensions.get("sessionInjectedCapabilities")
            or extensions.get("session_injected_capabilities")
            or []
        )
        self.expect(
            "document_tools" in allowed_names,
            "chat session allowlist lost requested document_tools extension",
        )
        self.expect(
            "chat_memory" in allowed_names,
            "session-injected chat_memory was filtered out by allowed_extensions",
        )
        injected_runtime_names: list[str] = []
        for item in injected_capabilities:
            runtime_names = item.get("runtimeNames") or item.get("runtime_names") or []
            injected_runtime_names.extend(runtime_names)
        self.expect(
            "chat_memory" in injected_runtime_names,
            "capability snapshot missing session-injected chat_memory capability",
        )
        return CaseResult(
            name="chat-memory-allowlist",
            entry="chat",
            status="passed",
            ids={"session_id": session_id},
            evidence={
                "allowed_extensions": detail.get("allowed_extensions"),
                "effective_allowed_extension_names": allowed_names,
                "session_injected_runtime_names": injected_runtime_names,
            },
            cleanup="agent session scheduled for deletion",
        )

    def case_heavy_direct_chat_runtime(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        original = self.patch_agent_for_heavy_runtime(auth)
        try:
            session_id = self.create_authenticated_chat_session(auth)
            observed_stage = False
            observed_commit = False
            observed_memory = False
            last_detail: dict[str, Any] = {}
            heavy_text = ("codex-e2e-heavy " * 520).strip()

            def wait_for_http_session_update(previous_detail: dict[str, Any]) -> dict[str, Any]:
                previous_message_count = previous_detail.get("message_count")
                previous_last_message_at = previous_detail.get("last_message_at")
                deadline = time.time() + self.args.poll_timeout
                while time.time() < deadline:
                    response, payload = self.get_json(
                        auth, f"/api/team/agent/chat/sessions/{session_id}"
                    )
                    self.expect(response.status_code == 200, "failed to get chat session detail")
                    self.raise_if_transient_provider_block(payload, f"chat session {session_id}")
                    if payload.get("last_execution_status") in {"completed", "failed", "blocked"} and (
                        payload.get("message_count") != previous_message_count
                        or payload.get("last_message_at") != previous_last_message_at
                        or payload.get("last_execution_status") == "blocked"
                    ):
                        return payload
                    time.sleep(self.args.poll_interval)
                raise CaseFailed(f"timed out waiting for HTTP session update: {session_id}")

            response, initial_detail = self.get_json(
                auth, f"/api/team/agent/chat/sessions/{session_id}"
            )
            self.expect(response.status_code == 200, "failed to get initial chat session detail")
            last_detail = initial_detail

            for turn in range(1, 5):
                print_step(f"heavy direct turn {turn}")
                self.send_authenticated_chat_message(
                    auth,
                    session_id,
                    f"[turn {turn}] {heavy_text}\nPlease acknowledge briefly.",
                )
                last_detail = wait_for_http_session_update(last_detail)
                stage, commit, memory, runtime_compactions, freed_tokens, summary_like = (
                    self.extract_runtime_progress(last_detail)
                )
                observed_stage = observed_stage or stage
                observed_commit = observed_commit or commit
                observed_memory = observed_memory or memory
                last_detail["context_runtime_summary"] = summary_like
                if observed_commit and observed_memory:
                    break
            if not observed_memory:
                self.send_authenticated_chat_message(auth, session_id, "/compact")
                last_detail = wait_for_http_session_update(last_detail)
                stage, commit, memory, runtime_compactions, freed_tokens, summary_like = (
                    self.extract_runtime_progress(last_detail)
                )
                observed_stage = observed_stage or stage
                observed_commit = observed_commit or commit
                observed_memory = observed_memory or memory
                last_detail["context_runtime_summary"] = summary_like
            _, _, _, runtime_compactions, freed_tokens, summary_like = self.extract_runtime_progress(
                last_detail
            )
            self.expect(
                observed_stage
                or observed_commit
                or observed_memory
                or runtime_compactions > 0
                or freed_tokens > 0,
                "heavy direct chat never showed compaction progression",
            )
            self.expect(
                observed_commit or observed_memory,
                "heavy direct chat never reached committed collapse or session memory",
            )
            self.expect(
                observed_memory,
                "heavy direct chat never reached session memory",
            )
            provider_request = self.latest_provider_request()
            serialized = json.dumps(provider_request, ensure_ascii=False)
            self.expect(
                '"prompt_caching_mode":"off"' in serialized
                or '"prompt_caching_mode": "off"' in serialized,
                "provider request log missing prompt_caching_mode=off",
            )
            self.expect(
                "cache_control" not in serialized,
                "provider request unexpectedly contains cache_control",
            )
            return CaseResult(
                name="heavy-direct-chat-runtime",
                entry="chat",
                status="passed",
                ids={"session_id": session_id},
                evidence={
                    "runtime_summary": summary_like,
                    "provider_log": str(Path(self.args.logs_dir) / "llm_request.0.jsonl"),
                },
                cleanup="agent session deleted and agent restored",
            )
        finally:
            self.restore_agent(auth, original)

    def case_task_flow(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        marker = short_uid("codex-e2e-task")
        response, payload = self.post_json(
            auth,
            "/api/team/agent/tasks",
            {
                "team_id": self.context.team_id,
                "agent_id": self.context.agent_id,
                "task_type": "chat",
                "content": {
                    "messages": [
                        {
                            "role": "user",
                            "content": f"Please reply with exactly TASK_OK-{marker} and nothing else.",
                        }
                    ]
                },
            },
        )
        self.expect(response.status_code == 200, f"submit task failed: {payload}")
        task_id = payload["id"]
        self.temp_tasks.add(task_id)
        task_doc = self.wait_for_task_completion(auth, task_id)
        result_doc = self.collection("agent_task_results").find_one(
            {"task_id": task_id}
        ) or self.collection("task_results").find_one({"task_id": task_id})
        hidden_session = self.find_hidden_agent_session_by_marker(marker)
        self.expect(hidden_session is not None, "task hidden session not found")
        hidden_session_id = hidden_session["session_id"]
        self.temp_chat_sessions.add(hidden_session_id)
        self.expect(
            (hidden_session.get("context_runtime_state") or {}).get("schemaVersion") == 6,
            "task hidden session is not V6",
        )
        return CaseResult(
            name="task-flow",
            entry="task",
            status="passed",
            ids={"task_id": task_id, "hidden_session_id": hidden_session_id},
            evidence={
                "task_status": task_doc.get("status"),
                "result_type": result_doc.get("result_type") if result_doc else None,
                "schemaVersion": hidden_session["context_runtime_state"]["schemaVersion"],
                "marker": marker,
            },
            cleanup="task and hidden session scheduled for deletion",
        )

    def case_heavy_task_runtime(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        original = self.patch_agent_for_heavy_runtime(auth)
        marker = short_uid("codex-e2e-heavy-task")
        try:
            observed_stage = False
            observed_commit = False
            observed_memory = False
            hidden_session_id: str | None = None
            last_summary: dict[str, Any] | None = None
            runtime_compactions = 0
            freed_token_estimate = 0
            previous_hidden_updated_at = None
            previous_hidden_signature: tuple[bool, bool, bool, int, int] | None = None
            baseline_hidden_sessions = {
                item["session_id"]
                for item in self.collection("agent_sessions").find(
                    {
                        "team_id": self.context.team_id,
                        "agent_id": self.context.agent_id,
                        "user_id": self.context.owner_user_id,
                        "hidden_from_chat_list": True,
                    },
                    {"session_id": 1},
                )
            }
            heavy_text = ("codex-e2e-heavy-task " * 360).strip()
            for turn in range(1, 5):
                print_step(f"heavy task turn {turn}")
                payload = {
                    "team_id": self.context.team_id,
                    "agent_id": self.context.agent_id,
                    "task_type": "chat",
                    "content": {
                        "messages": [
                            {
                                "role": "user",
                                "content": f"[{marker}] [task turn {turn}] {heavy_text}",
                            }
                        ]
                    },
                }
                if hidden_session_id:
                    payload["content"]["session_id"] = hidden_session_id
                response, task = self.post_json(auth, "/api/team/agent/tasks", payload)
                self.expect(response.status_code == 200, f"submit heavy task failed: {task}")
                task_id = task["id"]
                self.temp_tasks.add(task_id)
                self.wait_for_task_completion(auth, task_id)
                if not hidden_session_id:
                    hidden = self.find_hidden_agent_session_by_marker(
                        marker,
                        exclude_session_ids=baseline_hidden_sessions,
                    )
                    self.expect(hidden is not None, "heavy task hidden session not found")
                    hidden_session_id = hidden["session_id"]
                    previous_hidden_updated_at = hidden.get("updated_at")
                    self.temp_chat_sessions.add(hidden_session_id)
                else:
                    deadline = time.time() + self.args.poll_timeout
                    hidden = None
                    while time.time() < deadline:
                        hidden = self.collection("agent_sessions").find_one(
                            {"session_id": hidden_session_id}
                        )
                        if hidden is not None:
                            self.raise_if_transient_provider_block(hidden, "heavy task hidden session")
                            stage, commit, memory, compactions, freed_tokens, _ = (
                                self.extract_runtime_progress(hidden)
                            )
                            signature = (stage, commit, memory, compactions, freed_tokens)
                            if previous_hidden_signature is None or signature != previous_hidden_signature:
                                break
                        if hidden is not None and (
                            previous_hidden_updated_at is None
                            or hidden.get("updated_at") != previous_hidden_updated_at
                        ):
                            break
                        time.sleep(self.args.poll_interval)
                self.expect(hidden is not None, "heavy task hidden session disappeared")
                previous_hidden_updated_at = hidden.get("updated_at")
                stage, commit, memory, compactions, freed_tokens, summary_like = (
                    self.extract_runtime_progress(hidden)
                )
                observed_stage = observed_stage or stage
                observed_commit = observed_commit or commit
                observed_memory = observed_memory or memory
                runtime_compactions = max(runtime_compactions, compactions)
                freed_token_estimate = max(freed_token_estimate, freed_tokens)
                last_summary = summary_like
                previous_hidden_signature = (
                    stage,
                    commit,
                    memory,
                    compactions,
                    freed_tokens,
                )
                if observed_commit and observed_memory:
                    break
            self.expect(hidden_session_id is not None, "heavy task did not create hidden session")
            if not observed_memory:
                response, task = self.post_json(
                    auth,
                    "/api/team/agent/tasks",
                    {
                        "team_id": self.context.team_id,
                        "agent_id": self.context.agent_id,
                        "task_type": "chat",
                        "content": {
                            "session_id": hidden_session_id,
                            "messages": [
                                {
                                    "role": "user",
                                    "content": "/compact",
                                }
                            ],
                        },
                    },
                )
                self.expect(response.status_code == 200, f"submit compact task failed: {task}")
                task_id = task["id"]
                self.temp_tasks.add(task_id)
                self.wait_for_task_completion(auth, task_id)
                deadline = time.time() + self.args.poll_timeout
                hidden = None
                while time.time() < deadline:
                    hidden = self.collection("agent_sessions").find_one(
                        {"session_id": hidden_session_id}
                    )
                    if hidden is not None:
                        self.raise_if_transient_provider_block(hidden, "heavy task hidden session")
                        stage, commit, memory, compactions, freed_tokens, _ = (
                            self.extract_runtime_progress(hidden)
                        )
                        signature = (stage, commit, memory, compactions, freed_tokens)
                        if previous_hidden_signature is None or signature != previous_hidden_signature:
                            break
                    if hidden is not None and (
                        previous_hidden_updated_at is None
                        or hidden.get("updated_at") != previous_hidden_updated_at
                    ):
                        break
                    time.sleep(self.args.poll_interval)
                self.expect(hidden is not None, "hidden session missing after compact task")
                previous_hidden_updated_at = hidden.get("updated_at")
                stage, commit, memory, compactions, freed_tokens, summary_like = (
                    self.extract_runtime_progress(hidden)
                )
                observed_stage = observed_stage or stage
                observed_commit = observed_commit or commit
                observed_memory = observed_memory or memory
                runtime_compactions = max(runtime_compactions, compactions)
                freed_token_estimate = max(freed_token_estimate, freed_tokens)
                last_summary = summary_like
                previous_hidden_signature = (
                    stage,
                    commit,
                    memory,
                    compactions,
                    freed_tokens,
                )
            hidden_state = (hidden or {}).get("context_runtime_state") or {}
            hidden_store = hidden_state.get("store") or {}
            hidden_projection = hidden_state.get("lastProjectionStats") or {}
            schema_version = hidden_state.get("schemaVersion")
            projected_token_estimate = hidden_projection.get("projectedTokenEstimate") or 0
            raw_token_estimate = hidden_projection.get("rawTokenEstimate") or 0
            entry_log_len = len(hidden_store.get("entryLog") or [])
            compatibility_runtime_present = schema_version == 6 and (
                projected_token_estimate > 0 or raw_token_estimate > 0 or entry_log_len > 0
            )
            self.expect(
                observed_stage
                or observed_commit
                or observed_memory
                or runtime_compactions > 0
                or freed_token_estimate > 0
                or compatibility_runtime_present,
                "heavy task never showed compaction progression or persisted compatibility-path runtime evidence",
            )
            self.expect(
                observed_commit
                or observed_memory
                or freed_token_estimate > 0
                or compatibility_runtime_present,
                "heavy task never produced a compacted view or persisted compatibility-path runtime state",
            )
            return CaseResult(
                name="heavy-task-runtime",
                entry="task",
                status="passed",
                ids={"hidden_session_id": hidden_session_id},
                evidence={
                    "runtime_summary": last_summary,
                    "runtime_compactions": runtime_compactions,
                    "freed_token_estimate": freed_token_estimate,
                    "compatibility_runtime_present": compatibility_runtime_present,
                    "schema_version": schema_version,
                    "projected_token_estimate": projected_token_estimate,
                    "raw_token_estimate": raw_token_estimate,
                    "entry_log_len": entry_log_len,
                    "marker": marker,
                },
                cleanup="tasks and hidden session scheduled for deletion",
            )
        finally:
            self.restore_agent(auth, original)

    def case_channel_flow(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        channel_name = short_uid("codex-e2e-channel")
        response, payload = self.post_json(
            auth,
            "/api/team/agent/chat/channels",
            {
                "name": channel_name,
                "channel_type": "general",
                "default_agent_id": self.context.agent_id,
                "member_user_ids": [],
            },
            params={"team_id": self.context.team_id},
        )
        self.expect(response.status_code == 200, f"create channel failed: {payload}")
        channel_id = payload["channel"]["channel_id"]
        self.temp_channels.add(channel_id)
        response, channel_detail = self.get_json(
            auth,
            f"/api/team/agent/chat/channels/{channel_id}",
        )
        self.expect(response.status_code == 200, "failed to load channel detail")
        channel_detail = channel_detail.get("channel") or channel_detail
        self.expect(
            channel_detail.get("channel_type") == "general",
            "channel detail type mismatch for general channel",
        )
        self.expect(
            not channel_detail.get("workspace_path"),
            "general channel unexpectedly exposed workspace_path",
        )

        def wait_for_channel_text(expected_text: str) -> tuple[str | None, list[dict[str, Any]]]:
            deadline = time.time() + self.args.poll_timeout
            next_transient_check = time.time() + 10
            while time.time() < deadline:
                channel_events = list(
                    self.collection("chat_channel_events").find({"channel_id": channel_id})
                )
                messages = list(
                    self.collection("chat_channel_messages").find({"channel_id": channel_id})
                )
                done_seen = any(event.get("event_type") == "done" for event in channel_events)
                matching = next(
                    (
                        msg
                        for msg in messages
                        if msg.get("author_type") in {"agent", "system"}
                        and expected_text in (msg.get("content_text") or "")
                    ),
                    None,
                )
                if done_seen and matching is not None:
                    matching = next(
                        (msg for msg in messages if msg.get("message_id") == matching.get("message_id")),
                        None,
                    )
                    return (
                        matching.get("thread_root_id") if matching else None,
                        messages,
                    )
                if time.time() >= next_transient_check:
                    if self.journal_has_transient_provider_issue():
                        raise CaseBlocked("channel flow blocked by transient provider issue")
                    next_transient_check = time.time() + 10
                time.sleep(self.args.poll_interval)
            if self.journal_has_transient_provider_issue():
                raise CaseBlocked("channel flow blocked by transient provider issue")
            raise CaseFailed(f"channel flow did not emit {expected_text}")

        def wait_for_thread_reply(root_message_id: str, expected_text: str) -> dict[str, Any]:
            deadline = time.time() + self.args.poll_timeout
            next_transient_check = time.time() + 10
            while time.time() < deadline:
                response, thread_payload = self.get_json(
                    auth,
                    f"/api/team/agent/chat/channels/{channel_id}/threads/{root_message_id}",
                )
                self.expect(response.status_code == 200, "failed to load channel thread")
                self.raise_if_transient_provider_block(thread_payload, "channel thread")
                thread_messages = thread_payload.get("messages") or []
                if (thread_payload.get("thread_runtime") or {}).get("execution_status") == "blocked":
                    raise CaseFailed("channel thread blocked before assistant reply")
                if any(
                    item.get("author_type") in {"agent", "system"}
                    and expected_text in (item.get("content_text") or "")
                    for item in thread_messages
                ):
                    return thread_payload
                if time.time() >= next_transient_check:
                    if self.journal_has_transient_provider_issue():
                        raise CaseBlocked("channel thread blocked by transient provider issue")
                    next_transient_check = time.time() + 10
                time.sleep(self.args.poll_interval)
            if self.journal_has_transient_provider_issue():
                raise CaseBlocked("channel thread blocked by transient provider issue")
            raise CaseFailed(f"channel thread never exposed {expected_text}")

        response, first_payload = self.post_json(
            auth,
            f"/api/team/agent/chat/channels/{channel_id}/messages",
            {
                "content": "Please reply with exactly CHANNEL_OK and nothing else.",
                "surface": "temporary",
            },
        )
        self.expect(response.status_code == 200, "channel conversation message failed")
        conversation_root_id = first_payload["root_message_id"]
        thread_payload = wait_for_thread_reply(conversation_root_id, "CHANNEL_OK")
        conversation_reply_thread_root_id, _ = wait_for_channel_text("CHANNEL_OK")
        thread_messages = thread_payload.get("messages") or []
        thread_runtime = thread_payload.get("thread_runtime") or {}
        self.expect(
            any("CHANNEL_OK" in (item.get("content_text") or "") for item in thread_messages),
            "channel conversation thread API is missing the assistant reply",
        )
        self.expect(
            not thread_runtime.get("thread_worktree_path"),
            "general channel unexpectedly exposed coding thread runtime",
        )

        channel_events = list(
            self.collection("chat_channel_events").find({"channel_id": channel_id})
        )
        messages = list(
            self.collection("chat_channel_messages").find({"channel_id": channel_id})
        )
        done_seen = any(event.get("event_type") == "done" for event in channel_events)
        text_values = [msg.get("content_text") or "" for msg in messages]
        self.expect(done_seen, "channel flow missing done event")
        self.expect(
            any("CHANNEL_OK" in text for text in text_values),
            "channel conversation reply missing",
        )
        return CaseResult(
            name="channel-flow",
            entry="channel",
            status="passed",
            ids={"channel_id": channel_id},
            evidence={
                "done_seen": done_seen,
                "message_count": len(messages),
                "channel_type": channel_detail.get("channel_type"),
                "conversation_root_id": conversation_root_id,
                "conversation_reply_thread_root_id": conversation_reply_thread_root_id,
            },
            cleanup="channel scheduled for deletion",
        )

    def case_public_portal(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        portal_slug = self.resolve_portal_slug()
        visitor_id = short_uid("codex-e2e-visitor")
        self.temp_visitors.add(visitor_id)
        public = requests.Session()
        public.headers.update({"x-visitor-id": visitor_id})
        response, payload = self.post_json(
            public,
            f"/p/{portal_slug}/api/chat/session",
            {"visitor_id": visitor_id},
        )
        self.expect(response.status_code == 200, f"public create session failed: {payload}")
        session_id = payload["session_id"]
        self.temp_chat_sessions.add(session_id)
        response, payload = self.post_json(
            public,
            f"/p/{portal_slug}/api/chat/message",
            {
                "session_id": session_id,
                "visitor_id": visitor_id,
                "content": "Please reply with exactly PUBLIC_OK and nothing else.",
            },
        )
        self.expect(response.status_code == 200, f"public send message failed: {payload}")
        session_detail = self.wait_for_chat_completion(
            public, session_id, public_slug=portal_slug, visitor_id=visitor_id
        )
        state = session_detail.get("context_runtime_state") or {}
        self.expect(state.get("schemaVersion") == 6, "public portal session is not V6")
        return CaseResult(
            name="public-portal",
            entry="public",
            status="passed",
            ids={"session_id": session_id, "visitor_id": visitor_id},
            evidence={"schemaVersion": state.get("schemaVersion")},
            cleanup="visitor and portal session scheduled for deletion",
        )

    def case_document_portal(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        portal_slug = self.resolve_portal_slug()
        visitor_id = short_uid("codex-e2e-doc-visitor")
        self.temp_visitors.add(visitor_id)
        public = requests.Session()
        public.headers.update({"x-visitor-id": visitor_id})
        response, payload = self.post_json(
            public,
            f"/p/{portal_slug}/api/chat/session",
            {"visitor_id": visitor_id},
        )
        self.expect(response.status_code == 200, "public document session creation failed")
        session_id = payload["session_id"]
        self.temp_chat_sessions.add(session_id)
        files = {
            "file": (
                f"{short_uid('codex-e2e-doc')}.txt",
                b"AGIME document smoke.\nThis file contains the marker DOC_SMOKE_OK.\n",
                "text/plain",
            )
        }
        response = public.post(
            f"{self.base_url}/p/{portal_slug}/api/user-docs",
            files=files,
            headers={"x-visitor-id": visitor_id},
            timeout=60,
        )
        payload = response.json()
        self.expect(response.status_code == 200, f"document upload failed: {payload}")
        document_id = payload["document"]["id"]
        self.temp_documents.add(document_id)
        response, payload = self.post_json(
            public,
            f"/p/{portal_slug}/api/chat/message",
            {
                "session_id": session_id,
                "visitor_id": visitor_id,
                "content": "Read the uploaded document and reply with exactly DOC_OK.",
            },
        )
        self.expect(response.status_code == 200, f"document chat message failed: {payload}")
        self.wait_for_chat_completion(
            public, session_id, public_slug=portal_slug, visitor_id=visitor_id
        )
        session_doc = self.collection("agent_sessions").find_one({"session_id": session_id})
        self.expect(session_doc is not None, "document portal session missing from Mongo")
        self.expect(
            (session_doc.get("context_runtime_state") or {}).get("schemaVersion") == 6,
            "document portal session is not V6",
        )
        messages = json.loads(session_doc.get("messages_json") or "[]")
        transcript_text = json.dumps(messages, ensure_ascii=False)
        semantic_reply_observed = (
            "DOC_OK" in transcript_text
            or (session_doc.get("last_message_preview") or "").strip() == "DOC_OK"
        )
        workspace_read_observed = (
            "developer__shell" in transcript_text
            and "DOC_SMOKE_OK" in transcript_text
            and "/documents/" in transcript_text
        )
        legacy_document_tool_observed = self.journal_contains(
            "document_tools__read_document"
        )
        provider_balance_blocked = self.journal_contains(
            "insufficient_balance_error"
        ) or self.journal_contains("402 Payment Required")
        if (
            not semantic_reply_observed
            and
            not legacy_document_tool_observed
            and not workspace_read_observed
            and provider_balance_blocked
        ):
            raise CaseBlocked(
                "document portal provider blocked by insufficient balance"
            )
        self.expect(
            semantic_reply_observed
            or legacy_document_tool_observed
            or workspace_read_observed,
            "document portal flow did not return DOC_OK or show document read evidence",
        )
        return CaseResult(
            name="document-portal",
            entry="document",
            status="passed",
            ids={"session_id": session_id, "document_id": document_id},
            evidence={
                "schemaVersion": session_doc["context_runtime_state"]["schemaVersion"],
                "semantic_reply_observed": semantic_reply_observed,
                "legacy_document_tool_observed": legacy_document_tool_observed,
                "workspace_read_observed": workspace_read_observed,
            },
            cleanup="visitor, session, and document scheduled for deletion",
        )

    def case_public_unauthorized_doc_access(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        portal_slug = self.resolve_portal_slug()
        owner_visitor = short_uid("codex-e2e-doc-owner")
        stranger_visitor = short_uid("codex-e2e-doc-stranger")
        self.temp_visitors.update({owner_visitor, stranger_visitor})
        owner = requests.Session()
        owner.headers.update({"x-visitor-id": owner_visitor})
        files = {
            "file": (
                f"{short_uid('codex-e2e-private')}.txt",
                b"private visitor doc",
                "text/plain",
            )
        }
        response = owner.post(
            f"{self.base_url}/p/{portal_slug}/api/user-docs",
            files=files,
            headers={"x-visitor-id": owner_visitor},
            timeout=60,
        )
        try:
            payload = response.json() if response.content else None
        except Exception:  # noqa: BLE001
            payload = {"raw": response.text}
        self.expect(response.status_code == 200, "owner doc upload failed")
        document_id = payload["document"]["id"]
        self.temp_documents.add(document_id)
        stranger = requests.Session()
        stranger.headers.update({"x-visitor-id": stranger_visitor})
        response = stranger.get(
            f"{self.base_url}/p/{portal_slug}/api/user-docs/{document_id}",
            timeout=30,
        )
        try:
            payload = response.json() if response.content else None
        except Exception:  # noqa: BLE001
            payload = {"raw": response.text}
        self.expect(
            response.status_code in {403, 404},
            "unauthorized visitor accessed private doc",
        )
        return CaseResult(
            name="public-unauthorized-doc-access",
            entry="document",
            status="passed",
            ids={"document_id": document_id},
            evidence={"status_code": response.status_code, "payload": payload},
            cleanup="visitor doc scheduled for deletion",
        )

    def case_automation_builder_session_source(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        team_id = self.context.team_id
        project_id = None
        try:
            response, payload = self.post_json(
                auth,
                "/api/team/automation/projects",
                {
                    "team_id": team_id,
                    "name": short_uid("codex-e2e-auto-project"),
                    "description": "automation builder session source regression",
                },
            )
            self.expect(response.status_code == 200, f"create automation project failed: {payload}")
            project = payload.get("project") or {}
            project_id = project.get("project_id")
            self.expect(project_id, "automation project missing project_id")

            response, payload = self.post_json(
                auth,
                f"/api/team/automation/projects/{project_id}/tasks",
                {
                    "team_id": team_id,
                    "project_id": project_id,
                    "name": short_uid("codex-e2e-auto-draft"),
                    "driver_agent_id": self.context.agent_id,
                    "integration_ids": [],
                    "goal": "Create a builder session for automation planning only.",
                    "constraints": [],
                    "success_criteria": [],
                    "risk_preference": "balanced",
                    "create_builder_session": True,
                },
                params={"team_id": team_id},
            )
            self.expect(response.status_code == 200, f"create automation draft failed: {payload}")
            builder_session_id = payload.get("builder_session_id")
            self.expect(builder_session_id, "automation draft missing builder_session_id")
            self.temp_chat_sessions.add(builder_session_id)

            response, session_payload = self.get_json(
                auth,
                f"/api/team/agent/chat/sessions/{builder_session_id}",
            )
            self.expect(response.status_code == 200, "failed to load automation builder session")
            self.expect(
                session_payload.get("session_source") == "automation_builder",
                f"automation builder session source mismatch: {session_payload.get('session_source')!r}",
            )
            self.expect(
                session_payload.get("hidden_from_chat_list") is True,
                "automation builder session should stay hidden from chat list",
            )
            self.expect(
                session_payload.get("portal_restricted") is False,
                "automation builder session should not be portal restricted",
            )
            return CaseResult(
                name="automation-builder-session-source",
                entry="automation",
                status="passed",
                ids={
                    "project_id": project_id,
                    "builder_session_id": builder_session_id,
                },
                evidence={
                    "session_source": session_payload.get("session_source"),
                    "hidden_from_chat_list": session_payload.get("hidden_from_chat_list"),
                    "portal_restricted": session_payload.get("portal_restricted"),
                },
                cleanup="automation project deleted and builder session scheduled for deletion",
            )
        finally:
            if project_id:
                self.http.delete(
                    f"{self.base_url}/api/team/automation/projects/{project_id}",
                    params={"team_id": team_id},
                    cookies=auth.cookies,
                    headers={"User-Agent": "codex-e2e-suite/1.0"},
                    timeout=30,
                )

    def case_agentify_publish_requires_real_http_validation(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        project_id = None
        try:
            project = self.create_automation_project(
                auth,
                name=short_uid("codex-e2e-agentify-neg"),
                description="agentify publish gate regression",
            )
            project_id = project["project_id"]
            payload = self.create_automation_app_draft(
                auth,
                project_id=project_id,
                name=short_uid("codex-e2e-no-verify"),
                driver_agent_id=self.context.agent_id,
                integration_ids=[],
                goal="Create an agent app without any real validation.",
                constraints=[],
                success_criteria=[],
                create_builder_session=False,
            )
            draft = payload.get("app_draft") or payload.get("task") or {}
            draft_id = draft.get("draft_id")
            self.expect(bool(draft_id), "negative app draft missing draft_id")
            response, publish_payload = self.publish_automation_app(
                auth, draft_id, name="should-not-publish"
            )
            self.expect(
                response.status_code == 409,
                f"publish without validation should return 409: {publish_payload}",
            )
            readiness = publish_payload.get("publish_readiness") or {}
            self.expect(readiness.get("ready") is False, "negative publish unexpectedly ready")
            issues = readiness.get("issues") or []
            self.expect(
                any("真实 HTTP 验证" in item for item in issues),
                f"negative publish missing validation issue: {issues}",
            )
            return CaseResult(
                name="agentify-publish-requires-real-http-validation",
                entry="automation",
                status="passed",
                ids={"project_id": project_id, "draft_id": draft_id},
                evidence={"issues": issues, "readiness": readiness},
                cleanup="automation project deleted via API",
            )
        finally:
            if project_id:
                self.delete_automation_project(auth, project_id)

    def case_agentify_real_api_matrix(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        target_base_url = DEFAULT_AGENTIFY_TARGET_URL.rstrip("/")
        target_health_url = f"{target_base_url}/health"
        parsed_health_url = urlparse(target_health_url)
        postman_host = (parsed_health_url.hostname or "127.0.0.1").split(".")
        postman_port = (
            str(parsed_health_url.port)
            if parsed_health_url.port
            else ("443" if parsed_health_url.scheme == "https" else "80")
        )
        cases = [
            (
                "markdown",
                "markdown",
                f"# AGIME Health API\nBase URL: {target_base_url}\nGET /health\nReturn status, database, database_connected, version.",
            ),
            (
                "openapi",
                "openapi",
                json.dumps(
                    {
                        "openapi": "3.0.0",
                        "info": {"title": "AGIME Health", "version": "1.0.0"},
                        "servers": [{"url": target_base_url}],
                        "paths": {
                            "/health": {
                                "get": {"responses": {"200": {"description": "ok"}}}
                            }
                        },
                    },
                    ensure_ascii=False,
                ),
            ),
            ("curl", "curl", f"curl -X GET {target_health_url}"),
            (
                "postman",
                "postman",
                json.dumps(
                    {
                        "info": {
                            "name": "AGIME Health",
                            "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json",
                        },
                        "item": [
                            {
                                "name": "health",
                                "request": {
                                    "method": "GET",
                                    "url": {
                                        "raw": target_health_url,
                                        "protocol": parsed_health_url.scheme or "http",
                                        "host": postman_host,
                                        "port": postman_port,
                                        "path": ["health"],
                                    },
                                },
                            }
                        ],
                    },
                    ensure_ascii=False,
                ),
            ),
        ]
        matrix_results: list[dict[str, Any]] = []
        for case_name, spec_kind, spec_content in cases:
            project_id = None
            try:
                project = self.create_automation_project(
                    auth,
                    name=short_uid(f"codex-e2e-{case_name}"),
                    description=f"agentify {case_name} matrix",
                )
                project_id = project["project_id"]
                integration = self.create_automation_integration(
                    auth,
                    project_id=project_id,
                    name=f"AGIME {case_name}",
                    spec_kind=spec_kind,
                    spec_content=spec_content,
                    base_url=target_base_url,
                    auth_type="none",
                )
                draft_payload = self.create_automation_app_draft(
                    auth,
                    project_id=project_id,
                    name=f"AGIME 健康检查 Agent {case_name}",
                    driver_agent_id=self.context.agent_id,
                    integration_ids=[integration["integration_id"]],
                    goal="创建一个通过真实 AGIME server API 检查服务器健康状态的持续对话 Agent。",
                    constraints=[
                        "不要使用浏览器",
                        "必须用真实 HTTP 调用验证",
                        "输出要简洁清晰",
                    ],
                    success_criteria=[
                        "能够调用 GET /health",
                        "能回答当前 status/database_connected/version",
                        "可发布为持续对话 Agent",
                    ],
                    create_builder_session=True,
                )
                draft = draft_payload.get("app_draft") or draft_payload.get("task") or {}
                draft_id = draft.get("draft_id")
                builder_session_id = draft_payload.get("builder_session_id")
                self.expect(bool(draft_id), f"{case_name} app draft missing draft_id")
                self.expect(
                    bool(builder_session_id),
                    f"{case_name} app draft missing builder_session_id",
                )
                self.temp_chat_sessions.add(builder_session_id)
                self.send_authenticated_chat_message(
                    auth,
                    builder_session_id,
                    (
                        "请基于当前项目里的 API 资料，创建一个可发布的 Agent App。"
                        "先用真实 HTTP 调用验证 GET /health，然后把能力收敛成一个持续对话应用。"
                        "用户说“检查服务器健康状态”时，你应返回 status、database_connected 和 version。"
                        "不要使用浏览器，只用真实 API 调用。验证完成后把结果整理成可发布应用。"
                    ),
                )
                builder_detail = self.wait_for_chat_completion(auth, builder_session_id)
                self.expect(
                    builder_detail.get("last_execution_status") == "completed",
                    f"{case_name} builder did not complete: {builder_detail}",
                )
                sync_payload = self.sync_automation_app_draft(auth, draft_id)
                draft = sync_payload.get("app_draft") or sync_payload.get("task") or {}
                readiness = draft.get("publish_readiness") or {}
                if readiness.get("ready") is not True:
                    response, probe_payload = self.post_json(
                        auth,
                        f"/api/team/automation/app-drafts/{draft_id}/probe?team_id={self.context.team_id}",
                        {},
                    )
                    self.expect(
                        response.status_code == 200,
                        f"{case_name} probe failed: {probe_payload}",
                    )
                    probe_session_id = probe_payload.get("builder_session_id")
                    self.expect(
                        bool(probe_session_id),
                        f"{case_name} probe missing builder_session_id",
                    )
                    self.temp_chat_sessions.add(probe_session_id)
                    probe_detail = self.wait_for_chat_completion(auth, probe_session_id)
                    self.expect(
                        probe_detail.get("last_execution_status") == "completed",
                        f"{case_name} probe did not complete: {probe_detail}",
                    )
                    sync_payload = self.sync_automation_app_draft(auth, draft_id)
                    draft = sync_payload.get("app_draft") or sync_payload.get("task") or {}
                    readiness = draft.get("publish_readiness") or {}
                self.expect(
                    readiness.get("ready") is True,
                    f"{case_name} app draft is not ready: {draft}",
                )
                verification = readiness.get("verification") or {}
                self.expect(
                    (verification.get("valid_actions") or 0) > 0,
                    f"{case_name} readiness missing valid verification actions: {readiness}",
                )
                response, publish_payload = self.publish_automation_app(
                    auth, draft_id, name=f"AGIME 健康检查 Agent {case_name}"
                )
                self.expect(
                    response.status_code == 200,
                    f"{case_name} publish failed: {publish_payload}",
                )
                app = publish_payload.get("app") or publish_payload.get("module") or {}
                runtime_payload = self.load_automation_app_runtime(auth, app["module_id"])
                runtime_session_id = runtime_payload.get("runtime_session_id")
                self.expect(
                    bool(runtime_session_id),
                    f"{case_name} runtime payload missing runtime_session_id",
                )
                self.temp_chat_sessions.add(runtime_session_id)
                runtime_before = self.load_chat_session_detail(auth, runtime_session_id)
                self.send_authenticated_chat_message(
                    auth,
                    runtime_session_id,
                    "检查当前 AGIME server 健康状态，并告诉我 status、database_connected 和 version。",
                )
                runtime_detail = self.wait_for_chat_completion_after_update(
                    auth,
                    runtime_session_id,
                    previous_message_count=runtime_before.get("message_count"),
                    previous_last_message_at=runtime_before.get("last_message_at"),
                )
                preview = self.wait_for_session_assistant_text(
                    runtime_session_id,
                    required_substrings=["healthy", "2.8.0"],
                    timeout=90,
                )
                self.expect(
                    runtime_detail.get("last_execution_status") == "completed",
                    f"{case_name} runtime did not complete: {runtime_detail}",
                )
                self.expect("healthy" in preview.lower(), f"{case_name} missing healthy: {preview}")
                self.expect(
                    ("database_connected" in preview.lower())
                    or ("database connected" in preview.lower()),
                    f"{case_name} missing database_connected: {preview}",
                )
                self.expect("2.8.0" in preview, f"{case_name} missing version: {preview}")
                matrix_results.append(
                    {
                        "kind": case_name,
                        "builder_status": builder_detail.get("last_execution_status"),
                        "runtime_status": runtime_detail.get("last_execution_status"),
                        "ready": readiness.get("ready"),
                        "verification": verification,
                        "preview": preview[:240],
                    }
                )
            finally:
                if project_id:
                    self.delete_automation_project(auth, project_id)
        return CaseResult(
            name="agentify-real-api-matrix",
            entry="automation",
            status="passed",
            evidence={"matrix": matrix_results},
            cleanup="automation projects deleted via API",
        )

    def case_scheduled_task_one_shot(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        last_error = None
        for attempt in range(1, 3):
            marker = short_uid(f"scheduled-task-once-{attempt}")
            task = self.create_scheduled_task(
                auth,
                title=f"定时任务 {marker}",
                prompt=f"Please reply with exactly {marker} and nothing else.",
                task_kind="one_shot",
                one_shot_at=(utcnow() + timedelta(seconds=20)).isoformat(),
            )
            self.expect(task.get("status") == "draft", "new scheduled task should start as draft")
            channel = self.load_channel_detail(auth, task["channel_id"])
            self.expect(
                channel.get("channel_type") == "scheduled_task",
                f"scheduled task channel type mismatch: {channel.get('channel_type')}",
            )
            published = self.post_scheduled_task_action(auth, task["task_id"], "publish")["task"]
            self.expect(
                published.get("status") == "active",
                "published scheduled task is not active",
            )
            self.expect(
                bool(published.get("next_fire_at")),
                "published scheduled task missing next_fire_at",
            )
            try:
                finished = self.wait_for_scheduled_task_run(auth, task["task_id"], {"completed"})
            except CaseFailed as exc:
                last_error = str(exc)
                if attempt == 1 and "unexpected terminal status: failed" in last_error:
                    continue
                raise
            latest_run = (finished.get("runs") or [])[0]
            fire_message_id = latest_run.get("fire_message_id")
            runtime_session_id = latest_run.get("runtime_session_id")
            self.expect(bool(fire_message_id), "scheduled task run missing fire_message_id")
            self.expect(
                bool(runtime_session_id),
                "scheduled task run missing runtime_session_id",
            )
            runtime_session = self.collection("agent_sessions").find_one(
                {"session_id": runtime_session_id}
            )
            self.expect(runtime_session is not None, "scheduled task runtime session missing in Mongo")
            thread = self.load_channel_thread(auth, task["channel_id"], fire_message_id)
            thread_messages = thread.get("messages") or []
            self.expect(
                any(marker in (item.get("content_text") or "") for item in thread_messages),
                "one-shot scheduled task thread missing assistant reply",
            )
            return CaseResult(
                name="scheduled-task-one-shot",
                entry="scheduled-task",
                status="passed",
                ids={"task_id": task["task_id"], "channel_id": task["channel_id"]},
                evidence={
                    "task_status": finished.get("status"),
                    "latest_run_status": latest_run.get("status"),
                    "fire_message_id": fire_message_id,
                    "runtime_session_id": runtime_session_id,
                    "attempt": attempt,
                },
            )
        raise CaseFailed(last_error or "scheduled task one-shot failed")

    def case_scheduled_task_parse_preview(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        marker = short_uid("scheduled-task-parse")
        preview = self.parse_scheduled_task_preview(
            auth,
            text=f"每天早上9点读取团队文档变化并生成一份 md 报告到工作区 {marker}",
        )
        self.expect(preview.get("ready_to_create") is True, "preview should be ready to create")
        self.expect(preview.get("task_kind") == "cron", "preview task_kind should be cron")
        self.expect(
            preview.get("task_profile") in {"document_task", "hybrid_task"},
            f"unexpected preview task_profile: {preview.get('task_profile')}",
        )
        self.expect(
            (preview.get("execution_contract") or {}).get("output_mode") == "summary_and_artifact",
            "preview should infer summary_and_artifact",
        )
        self.expect(
            bool(preview.get("human_schedule")),
            "preview missing human_schedule",
        )
        return CaseResult(
            name="scheduled-task-parse-preview",
            entry="scheduled-task",
            status="passed",
            evidence={
                "task_profile": preview.get("task_profile"),
                "payload_kind": preview.get("payload_kind"),
                "session_binding": preview.get("session_binding"),
                "delivery_plan": preview.get("delivery_plan"),
                "human_schedule": preview.get("human_schedule"),
                "warnings": preview.get("warnings"),
            },
        )

    def case_scheduled_task_create_from_parse(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        marker = short_uid("scheduled-task-vnext")
        preview = self.parse_scheduled_task_preview(
            auth,
            text=f"每周一上午10点生成一份项目进展 md 到工作区 {marker}",
        )
        self.expect(preview.get("ready_to_create") is True, "parse preview should be ready")
        task = self.create_scheduled_task_from_parse(auth, preview=preview)
        detail = self.load_scheduled_task_detail(auth, task["task_id"])
        self.expect(
            detail.get("task_profile") == preview.get("task_profile"),
            "persisted task_profile mismatch",
        )
        self.expect(
            detail.get("payload_kind") == preview.get("payload_kind"),
            "persisted payload_kind mismatch",
        )
        self.expect(
            detail.get("session_binding") == preview.get("session_binding"),
            "persisted session_binding mismatch",
        )
        self.expect(
            detail.get("delivery_plan") == preview.get("delivery_plan"),
            "persisted delivery_plan mismatch",
        )
        self.expect(
            (detail.get("execution_contract") or {}).get("output_mode")
            == (preview.get("execution_contract") or {}).get("output_mode"),
            "persisted execution_contract mismatch",
        )
        return CaseResult(
            name="scheduled-task-create-from-parse",
            entry="scheduled-task",
            status="passed",
            ids={"task_id": task["task_id"], "channel_id": task["channel_id"]},
            evidence={
                "task_profile": detail.get("task_profile"),
                "payload_kind": detail.get("payload_kind"),
                "session_binding": detail.get("session_binding"),
                "delivery_plan": detail.get("delivery_plan"),
                "human_schedule": detail.get("human_schedule"),
                "artifact_path": (detail.get("execution_contract") or {}).get("artifact_path"),
            },
        )

    def case_scheduled_task_chat_create(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        marker = short_uid("chat-scheduled-task")
        session_id = self.create_authenticated_chat_session(auth, allowed_extensions=["team_mcp"])
        self.send_authenticated_chat_message(
            auth,
            session_id,
            "\n".join(
                [
                    "你必须使用 create_scheduled_task 工具，不要模拟，不要解释。",
                    f"请创建一个定时任务：明天下午3点提醒我输出 {marker}。",
                    "创建完成后，只用一句中文确认已经创建。",
                ]
            ),
        )
        detail = self.wait_for_chat_completion(auth, session_id)
        scheduled_doc = self.collection("scheduled_tasks").find_one(
            {
                "team_id": self.context.team_id,
                "status": {"$ne": "deleted"},
                "$or": [
                    {"title": {"$regex": marker}},
                    {"prompt": {"$regex": marker}},
                ],
            },
            sort=[("created_at", -1)],
        )
        self.expect(scheduled_doc is not None, "chat flow did not create scheduled task")
        self.temp_scheduled_tasks.add(scheduled_doc["task_id"])
        self.temp_channels.add(scheduled_doc["channel_id"])
        self.expect(
            detail.get("last_execution_status") == "completed",
            f"chat create session not completed: {detail.get('last_execution_status')}",
        )
        return CaseResult(
            name="scheduled-task-chat-create",
            entry="scheduled-task",
            status="passed",
            ids={
                "session_id": session_id,
                "task_id": scheduled_doc["task_id"],
                "channel_id": scheduled_doc["channel_id"],
            },
            evidence={
                "task_profile": scheduled_doc.get("task_profile"),
                "payload_kind": scheduled_doc.get("payload_kind"),
                "session_binding": scheduled_doc.get("session_binding"),
                "delivery_plan": scheduled_doc.get("delivery_plan"),
                "chat_last_message_preview": detail.get("last_message_preview"),
            },
        )

    def case_scheduled_task_run_now(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        marker = short_uid("scheduled-task-run-now")
        task = self.create_scheduled_task(
            auth,
            title=f"Run Now {marker}",
            prompt=f"Please reply with exactly {marker} and nothing else.",
            task_kind="cron",
            cron_expression="*/15 * * * *",
        )
        run_payload = self.post_scheduled_task_action(auth, task["task_id"], "run-now")
        run = run_payload["run"]
        self.expect(run.get("status") == "running", f"run-now did not start run: {run}")
        finished = self.wait_for_scheduled_task_run(auth, task["task_id"], {"completed"})
        latest_run = (finished.get("runs") or [])[0]
        self.expect(bool(latest_run.get("fire_message_id")), "run-now missing fire_message_id")
        self.expect(
            bool(latest_run.get("runtime_session_id")),
            "run-now missing runtime_session_id",
        )
        self.expect(
            finished.get("status") == "draft",
            "manual run on draft scheduled task should not publish it",
        )
        self.expect(
            finished.get("next_fire_at") in {None, ""},
            "draft scheduled task should not gain next_fire_at after manual run",
        )
        thread = self.load_channel_thread(auth, task["channel_id"], latest_run["fire_message_id"])
        thread_messages = thread.get("messages") or []
        self.expect(
            any(marker in (item.get("content_text") or "") for item in thread_messages),
            "run-now scheduled task thread missing assistant reply",
        )
        return CaseResult(
            name="scheduled-task-run-now",
            entry="scheduled-task",
            status="passed",
            ids={"task_id": task["task_id"], "channel_id": task["channel_id"]},
            evidence={
                "task_status": finished.get("status"),
                "latest_run_status": latest_run.get("status"),
                "trigger_source": latest_run.get("trigger_source"),
            },
        )

    def case_scheduled_task_pause_resume(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        task = self.create_scheduled_task(
            auth,
            title=short_uid("scheduled-task-cycle"),
            prompt="Please reply with exactly SCHEDULED_TASK_CYCLE_OK and nothing else.",
            task_kind="cron",
            cron_expression="*/30 * * * *",
        )
        published = self.post_scheduled_task_action(auth, task["task_id"], "publish")["task"]
        self.expect(published.get("status") == "active", "cron scheduled task did not activate")
        self.expect(
            bool(published.get("next_fire_at")),
            "cron scheduled task missing next_fire_at after publish",
        )
        paused = self.post_scheduled_task_action(auth, task["task_id"], "pause")["task"]
        self.expect(paused.get("status") == "paused", "pause did not move task to paused")
        resumed = self.post_scheduled_task_action(auth, task["task_id"], "resume")["task"]
        self.expect(resumed.get("status") == "active", "resume did not move task back to active")
        self.expect(
            bool(resumed.get("next_fire_at")),
            "resume did not restore next_fire_at",
        )
        return CaseResult(
            name="scheduled-task-pause-resume",
            entry="scheduled-task",
            status="passed",
            ids={"task_id": task["task_id"], "channel_id": task["channel_id"]},
            evidence={"next_fire_at": resumed.get("next_fire_at")},
        )

    def case_scheduled_task_missed_one_shot_recovery(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        marker = short_uid("scheduled-task-missed-once")
        task = self.create_scheduled_task(
            auth,
            title=f"Missed Once {marker}",
            prompt=f"Please reply with exactly {marker} and nothing else.",
            task_kind="one_shot",
            one_shot_at=(utcnow() + timedelta(minutes=5)).isoformat(),
        )
        published = self.post_scheduled_task_action(auth, task["task_id"], "publish")["task"]
        self.expect(published.get("status") == "active", "missed one-shot task did not activate")
        self.collection("scheduled_tasks").update_one(
            {"task_id": task["task_id"], "team_id": self.context.team_id},
            {
                "$set": {
                    "next_fire_at": utcnow() - timedelta(seconds=30),
                    "updated_at": utcnow(),
                }
            },
        )
        finished = self.wait_for_scheduled_task_run(auth, task["task_id"], {"completed"})
        latest_run = (finished.get("runs") or [])[0]
        self.expect(
            latest_run.get("trigger_source") == "missed_recovery",
            f"expected missed_recovery trigger, got {latest_run}",
        )
        self.expect(
            finished.get("missed_fire_count", 0) >= 1,
            "missed recovery did not increment missed_fire_count",
        )
        return CaseResult(
            name="scheduled-task-missed-one-shot",
            entry="scheduled-task",
            status="passed",
            ids={"task_id": task["task_id"], "channel_id": task["channel_id"]},
            evidence={
                "latest_run_status": latest_run.get("status"),
                "trigger_source": latest_run.get("trigger_source"),
                "missed_fire_count": finished.get("missed_fire_count"),
            },
        )

    def case_scheduled_task_missed_cron_recovery(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        _, auth = self.create_temp_web_session(self.context.owner_user_id)
        marker = short_uid("scheduled-task-missed-cron")
        task = self.create_scheduled_task(
            auth,
            title=f"Missed Cron {marker}",
            prompt=f"Please reply with exactly {marker} and nothing else.",
            task_kind="cron",
            cron_expression="*/30 * * * *",
        )
        published = self.post_scheduled_task_action(auth, task["task_id"], "publish")["task"]
        self.expect(published.get("status") == "active", "missed cron task did not activate")
        self.collection("scheduled_tasks").update_one(
            {"task_id": task["task_id"], "team_id": self.context.team_id},
            {
                "$set": {
                    "next_fire_at": utcnow() - timedelta(seconds=30),
                    "updated_at": utcnow(),
                }
            },
        )
        finished = self.wait_for_scheduled_task_run(auth, task["task_id"], {"completed"})
        latest_run = (finished.get("runs") or [])[0]
        self.expect(
            latest_run.get("trigger_source") == "missed_recovery",
            f"expected missed_recovery trigger, got {latest_run}",
        )
        self.expect(finished.get("status") == "active", "cron task should remain active")
        self.expect(bool(finished.get("next_fire_at")), "cron missed recovery should reschedule")
        return CaseResult(
            name="scheduled-task-missed-cron",
            entry="scheduled-task",
            status="passed",
            ids={"task_id": task["task_id"], "channel_id": task["channel_id"]},
            evidence={
                "latest_run_status": latest_run.get("status"),
                "trigger_source": latest_run.get("trigger_source"),
                "next_fire_at": finished.get("next_fire_at"),
            },
        )

    def case_scheduled_task_visibility(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        member_user_id = self.find_non_owner_team_member_user_id()
        _, owner_auth = self.create_temp_web_session(self.context.owner_user_id)
        _, member_auth = self.create_temp_web_session(member_user_id)
        owner_marker = short_uid("scheduled-task-owner")
        member_marker = short_uid("scheduled-task-member")
        owner_task = self.create_scheduled_task(
            owner_auth,
            title=f"Owner Task {owner_marker}",
            prompt=f"Please reply with exactly {owner_marker} and nothing else.",
            task_kind="one_shot",
            one_shot_at=(utcnow() + timedelta(minutes=10)).isoformat(),
        )
        member_task = self.create_scheduled_task(
            member_auth,
            title=f"Member Task {member_marker}",
            prompt=f"Please reply with exactly {member_marker} and nothing else.",
            task_kind="one_shot",
            one_shot_at=(utcnow() + timedelta(minutes=10)).isoformat(),
        )
        owner_mine = self.list_scheduled_tasks(owner_auth, "mine")
        owner_all = self.list_scheduled_tasks(owner_auth, "all_visible")
        member_mine = self.list_scheduled_tasks(member_auth, "mine")
        member_all = self.list_scheduled_tasks(member_auth, "all_visible")
        owner_mine_ids = {item.get("task_id") for item in owner_mine}
        owner_all_ids = {item.get("task_id") for item in owner_all}
        member_mine_ids = {item.get("task_id") for item in member_mine}
        member_all_ids = {item.get("task_id") for item in member_all}
        self.expect(owner_task["task_id"] in owner_mine_ids, "owner mine view missing owner task")
        self.expect(member_task["task_id"] not in owner_mine_ids, "owner mine view leaked member task")
        self.expect(member_task["task_id"] in owner_all_ids, "admin all_visible missing member task")
        self.expect(member_task["task_id"] in member_mine_ids, "member mine view missing own task")
        self.expect(owner_task["task_id"] not in member_all_ids, "member all_visible should not expose owner task")
        return CaseResult(
            name="scheduled-task-visibility",
            entry="scheduled-task",
            status="passed",
            ids={"owner_task_id": owner_task["task_id"], "member_task_id": member_task["task_id"]},
            evidence={
                "owner_mine_count": len(owner_mine),
                "owner_all_count": len(owner_all),
                "member_mine_count": len(member_mine),
                "member_all_count": len(member_all),
            },
        )

    def case_scheduled_task_session_scoped_expiration(self) -> CaseResult:
        self.expect(self.context is not None, "team context not initialized")
        owner_session_id, auth = self.create_temp_web_session(self.context.owner_user_id)
        task = self.create_scheduled_task(
            auth,
            title=short_uid("scheduled-task-session-scoped"),
            prompt="Please reply with exactly SESSION_SCOPED_OK and nothing else.",
            task_kind="one_shot",
            delivery_tier="session_scoped",
            one_shot_at=(utcnow() + timedelta(minutes=10)).isoformat(),
        )
        self.expect(
            task.get("owner_session_id") == owner_session_id,
            f"session-scoped task did not bind to current session: {task.get('owner_session_id')}",
        )
        published = self.post_scheduled_task_action(auth, task["task_id"], "publish")["task"]
        self.expect(published.get("status") == "active", "session-scoped task did not activate")
        self.collection("sessions").delete_one({"session_id": owner_session_id})
        deadline = time.time() + self.args.poll_timeout
        last_doc = None
        while time.time() < deadline:
            last_doc = self.collection("scheduled_tasks").find_one(
                {"task_id": task["task_id"], "team_id": self.context.team_id}
            )
            if last_doc and last_doc.get("status") == "deleted":
                break
            time.sleep(self.args.poll_interval)
        self.expect(last_doc is not None, "session-scoped task missing from Mongo")
        self.expect(last_doc.get("status") == "deleted", "session-scoped task did not expire")
        self.expect(
            last_doc.get("next_fire_at") in {None, ""},
            "expired session-scoped task should clear next_fire_at",
        )
        return CaseResult(
            name="scheduled-task-session-scoped-expiration",
            entry="scheduled-task",
            status="passed",
            ids={"task_id": task["task_id"], "owner_session_id": owner_session_id},
            evidence={
                "status": last_doc.get("status"),
                "next_fire_at": last_doc.get("next_fire_at"),
            },
        )

    def run_live_suite(self) -> None:
        self.context = self.resolve_team_by_name(self.args.team_name)
        self.run_case("health", "public", self.case_health)
        self.run_case("cookie-auth", "auth", self.case_cookie_auth)
        self.run_case("invalid-session", "auth", self.case_invalid_session)
        self.run_case("session-sliding-renewal", "auth", self.case_session_sliding_renewal)
        self.run_case("api-key-login", "auth", self.case_api_key_login)
        self.run_case("password-login", "auth", self.case_password_login)
        self.run_case("runtime-profile-preview", "agent", self.case_runtime_profile_preview)
        self.run_case(
            "agent-intent-preserved-with-preview-downgrade",
            "agent",
            self.case_agent_intent_preserved_with_preview_downgrade,
        )
        self.run_case(
            "runtime-profile-litellm-modes",
            "agent",
            self.case_runtime_profile_litellm_modes,
        )
        self.run_case("direct-chat", "chat", self.case_direct_chat)
        self.run_case("chat-memory-allowlist", "chat", self.case_chat_memory_allowlist)
        if not self.args.skip_heavy_cases:
            self.run_case(
                "heavy-direct-chat-runtime",
                "chat",
                self.case_heavy_direct_chat_runtime,
            )
        self.run_case("task-flow", "task", self.case_task_flow)
        if not self.args.skip_heavy_cases:
            self.run_case("heavy-task-runtime", "task", self.case_heavy_task_runtime)
        self.run_case("channel-flow", "channel", self.case_channel_flow)
        self.run_case("public-portal", "public", self.case_public_portal)
        self.run_case("document-portal", "document", self.case_document_portal)
        self.run_case(
            "public-unauthorized-doc-access",
            "document",
            self.case_public_unauthorized_doc_access,
        )
        self.run_case(
            "automation-builder-session-source",
            "automation",
            self.case_automation_builder_session_source,
        )

    def run_harness_suite(self) -> None:
        self.context = self.resolve_team_by_name(self.args.team_name)
        self.run_case(
            "harness-direct-chat-baseline",
            "harness",
            self.case_harness_direct_chat_baseline,
        )
        self.run_case(
            "harness-direct-chat-heavy-runtime",
            "harness",
            self.case_harness_direct_chat_heavy_runtime,
        )
        self.run_case(
            "harness-task-runtime",
            "harness",
            self.case_harness_task_runtime,
        )
        self.run_case(
            "harness-task-artifact-truth",
            "harness",
            self.case_harness_task_artifact_truth,
        )
        self.run_case(
            "harness-task-cancel",
            "harness",
            self.case_harness_task_cancel,
        )
        self.run_case(
            "harness-channel-runtime",
            "harness",
            self.case_harness_channel_runtime,
        )
        self.run_case(
            "harness-chat-memory-allowlist",
            "harness",
            self.case_harness_chat_memory_allowlist,
        )
        self.run_case(
            "harness-runtime-diagnostics-audit",
            "harness",
            self.case_harness_runtime_diagnostics_audit,
        )

    def run_scheduled_tasks_suite(self) -> None:
        self.context = self.resolve_team_by_name(self.args.team_name)
        self.run_case(
            "scheduled-task-parse-preview",
            "scheduled-task",
            self.case_scheduled_task_parse_preview,
        )
        self.run_case(
            "scheduled-task-create-from-parse",
            "scheduled-task",
            self.case_scheduled_task_create_from_parse,
        )
        self.run_case(
            "scheduled-task-chat-create",
            "scheduled-task",
            self.case_scheduled_task_chat_create,
        )
        self.run_case(
            "scheduled-task-one-shot",
            "scheduled-task",
            self.case_scheduled_task_one_shot,
        )
        self.run_case(
            "scheduled-task-run-now",
            "scheduled-task",
            self.case_scheduled_task_run_now,
        )
        self.run_case(
            "scheduled-task-pause-resume",
            "scheduled-task",
            self.case_scheduled_task_pause_resume,
        )
        self.run_case(
            "scheduled-task-missed-one-shot",
            "scheduled-task",
            self.case_scheduled_task_missed_one_shot_recovery,
        )
        self.run_case(
            "scheduled-task-missed-cron",
            "scheduled-task",
            self.case_scheduled_task_missed_cron_recovery,
        )
        self.run_case(
            "scheduled-task-visibility",
            "scheduled-task",
            self.case_scheduled_task_visibility,
        )
        self.run_case(
            "scheduled-task-session-scoped-expiration",
            "scheduled-task",
            self.case_scheduled_task_session_scoped_expiration,
        )

    def run_agentify_suite(self) -> None:
        self.context = self.resolve_team_by_name(self.args.team_name)
        self.run_case(
            "agentify-publish-requires-real-http-validation",
            "automation",
            self.case_agentify_publish_requires_real_http_validation,
        )
        self.run_case(
            "agentify-real-api-matrix",
            "automation",
            self.case_agentify_real_api_matrix,
        )

    def write_report(self) -> None:
        report_path = Path(self.args.json_out)
        report_path.parent.mkdir(parents=True, exist_ok=True)
        payload = {
            "generated_at": utcnow().isoformat(),
            "base_url": self.base_url,
            "db_name": self.args.db_name,
            "team_name": self.args.team_name,
            "agent_name": self.args.agent_name,
            "results": [asdict(item) for item in self.report],
        }
        report_path.write_text(
            json.dumps(payload, ensure_ascii=False, indent=2, default=json_default),
            encoding="utf-8",
        )
        print_step(f"report written: {report_path}")

    def print_summary(self) -> None:
        passed = sum(1 for item in self.report if item.status == "passed")
        failed = sum(1 for item in self.report if item.status == "failed")
        blocked = sum(1 for item in self.report if item.status == "blocked by environment")
        print("")
        print("AGIME TEAM SERVER E2E SUMMARY", flush=True)
        print(f"passed={passed} failed={failed} blocked={blocked}", flush=True)
        for item in self.report:
            print(f"- {item.status}: {item.name} ({item.entry})", flush=True)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="AGIME Team Server end-to-end suite")
    parser.add_argument(
        "--mode",
        choices=[
            "rust",
            "live",
            "full",
            "harness",
            "scheduled-tasks",
            "scheduled-tasks-live",
            "agentify",
        ],
        default="full",
    )
    parser.add_argument("--base-url", default=DEFAULT_BASE_URL)
    parser.add_argument("--mongo-uri", default=DEFAULT_MONGO_URI)
    parser.add_argument("--db-name", default=DEFAULT_DB_NAME)
    parser.add_argument("--workspace", default=DEFAULT_WORKSPACE)
    parser.add_argument("--logs-dir", default=DEFAULT_LOGS_DIR)
    parser.add_argument("--team-name", default=DEFAULT_TEAM_NAME)
    parser.add_argument("--agent-name", default=DEFAULT_AGENT_NAME)
    parser.add_argument("--portal-slug")
    parser.add_argument("--json-out", default=DEFAULT_JSON_OUT)
    parser.add_argument("--password-email")
    parser.add_argument("--password")
    parser.add_argument("--poll-timeout", type=int, default=300)
    parser.add_argument("--poll-interval", type=float, default=2.0)
    parser.add_argument("--skip-frontend-build", action="store_true")
    parser.add_argument("--skip-heavy-cases", action="store_true")
    parser.add_argument("--cases")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    suite = TeamServerE2ESuite(args)
    try:
        if args.mode in {"rust", "full", "harness", "scheduled-tasks"}:
            suite.run_case(
                "cargo-check-and-targeted-tests",
                "rust",
                lambda: suite.rust_checks_case_result(
                    include_harness_targets=args.mode == "harness",
                    include_scheduled_task_targets=args.mode == "scheduled-tasks",
                    scheduled_tasks_only=args.mode == "scheduled-tasks",
                ),
            )
            if args.mode == "full" and not args.skip_frontend_build:
                suite.run_case(
                    "web-admin-build",
                    "frontend",
                    lambda: (
                        suite.run_frontend_build(),
                        CaseResult(
                            name="web-admin-build",
                            entry="frontend",
                            status="passed",
                        ),
                    )[1],
                )
        if args.mode in {"live", "full"}:
            suite.run_live_suite()
        if args.mode == "harness":
            suite.run_harness_suite()
        if args.mode in {"scheduled-tasks", "scheduled-tasks-live"}:
            suite.run_scheduled_tasks_suite()
        if args.mode == "agentify":
            suite.run_agentify_suite()
    finally:
        try:
            suite.cleanup()
        finally:
            suite.write_report()
            suite.print_summary()
    return 1 if any(item.status == "failed" for item in suite.report) else 0


if __name__ == "__main__":
    sys.exit(main())
