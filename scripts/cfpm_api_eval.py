#!/usr/bin/env python3
"""
CFPM runtime evaluation via local AGIME API.

What it does:
1) Starts local agimed server on a dedicated port (optional)
2) Creates a fresh session
3) Forces provider/model on the session
4) Sends two chat turns via /reply (SSE)
5) Fetches session + CFPM facts/candidates/tool-gates via API
6) Cross-checks API memory against SQLite memory tables
7) Emits a compact JSON report for regression analysis
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sqlite3
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple


DATE_ONLY_RE = re.compile(r"^\d{4}[./-]\d{1,2}[./-]\d{1,2}$")
DESKTOP_WRONG_RE = re.compile(r"^[A-Za-z]:\\Users\\[^\\]+\\Desktop\\?$", re.IGNORECASE)
PROBE_MARKERS = [
    "$env:USERPROFILE",
    "%USERPROFILE%",
    "GetFolderPath('Desktop')",
    "User Shell Folders",
    "Test-Path",
    "Get-ChildItem C:\\Users",
]


def now_ts() -> int:
    return int(time.time())


def maybe_json(value: str) -> Any:
    try:
        return json.loads(value)
    except Exception:
        return value


@dataclass
class ApiClient:
    base_url: str
    secret: str

    def _headers(self, extra: Optional[Dict[str, str]] = None) -> Dict[str, str]:
        headers = {
            "X-Secret-Key": self.secret,
            "Content-Type": "application/json",
        }
        if extra:
            headers.update(extra)
        return headers

    def request_json(self, method: str, path: str, payload: Optional[dict] = None) -> Any:
        url = f"{self.base_url}{path}"
        data = None
        if payload is not None:
            data = json.dumps(payload).encode("utf-8")
        req = urllib.request.Request(url=url, method=method.upper(), data=data)
        for k, v in self._headers().items():
            req.add_header(k, v)
        with urllib.request.urlopen(req, timeout=30) as resp:
            raw = resp.read().decode("utf-8", errors="replace")
            if not raw:
                return None
            return json.loads(raw)

    def request_status(self) -> Tuple[int, str]:
        url = f"{self.base_url}/status"
        req = urllib.request.Request(url=url, method="GET")
        with urllib.request.urlopen(req, timeout=10) as resp:
            return resp.status, resp.read().decode("utf-8", errors="replace")

    def stream_reply(self, chat_payload: dict, timeout_sec: int = 180) -> List[dict]:
        url = f"{self.base_url}/reply"
        data = json.dumps(chat_payload).encode("utf-8")
        req = urllib.request.Request(url=url, method="POST", data=data)
        for k, v in self._headers({"Accept": "text/event-stream"}).items():
            req.add_header(k, v)

        events: List[dict] = []
        started = time.monotonic()
        with urllib.request.urlopen(req, timeout=timeout_sec) as resp:
            for raw in resp:
                if time.monotonic() - started > timeout_sec:
                    break
                line = raw.decode("utf-8", errors="replace").strip()
                if not line or not line.startswith("data:"):
                    continue
                payload = line[5:].strip()
                if not payload:
                    continue
                parsed = maybe_json(payload)
                if isinstance(parsed, dict):
                    events.append(parsed)
                    if parsed.get("type") in ("Finish", "Error"):
                        break
        return events


def wait_for_status(client: ApiClient, max_wait_sec: int = 30) -> None:
    start = time.monotonic()
    while True:
        try:
            code, _ = client.request_status()
            if code == 200:
                return
        except Exception:
            pass
        if time.monotonic() - start > max_wait_sec:
            raise TimeoutError("server /status did not become ready in time")
        time.sleep(0.5)


def start_server(exe: Path, host: str, port: int, secret: str) -> subprocess.Popen:
    env = os.environ.copy()
    env["AGIME_HOST"] = host
    env["AGIME_PORT"] = str(port)
    env["AGIME_SERVER__SECRET_KEY"] = secret
    env.setdefault("RUST_LOG", "info")
    return subprocess.Popen(
        [str(exe), "agent"],
        cwd=str(Path.cwd()),
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def default_message(text: str) -> dict:
    return {
        "role": "user",
        "created": now_ts(),
        "content": [{"type": "text", "text": text}],
        "metadata": {"userVisible": True, "agentVisible": True},
    }


def read_cfg_value(client: ApiClient, key: str) -> Optional[str]:
    try:
        # ConfigKeyQuery requires is_secret + key
        resp = client.request_json(
            "POST",
            "/config/read",
            {
                "key": key,
                "is_secret": False,
            },
        )
        if isinstance(resp, str) and resp.strip():
            return resp.strip()
        if isinstance(resp, dict):
            value = resp.get("value")
            if isinstance(value, str) and value.strip():
                return value.strip()
        return None
    except Exception:
        return None


def select_provider_model(client: ApiClient, provider_arg: Optional[str], model_arg: Optional[str]) -> Tuple[str, str]:
    provider = provider_arg or read_cfg_value(client, "provider") or read_cfg_value(client, "agime-provider")
    model = model_arg or read_cfg_value(client, "model") or read_cfg_value(client, "agime-model")
    if not provider or not model:
        raise RuntimeError(
            "provider/model not resolved; pass --provider and --model explicitly or configure them"
        )
    return provider, model


def extract_tool_commands_from_session(session_obj: dict) -> List[str]:
    commands: List[str] = []
    conv = session_obj.get("conversation") if isinstance(session_obj, dict) else None
    messages = conv.get("messages") if isinstance(conv, dict) else []
    for msg in messages or []:
        for content in msg.get("content", []) or []:
            if content.get("type") != "toolRequest":
                continue
            tool_call = content.get("toolCall") or content.get("tool_call") or {}
            if not isinstance(tool_call, dict):
                continue
            call = tool_call.get("Ok") or tool_call.get("ok") or {}
            if not isinstance(call, dict):
                continue
            args = call.get("arguments")
            if isinstance(args, dict):
                for key in ("command", "cmd", "script"):
                    val = args.get(key)
                    if isinstance(val, str) and val.strip():
                        commands.append(val.strip())
                        break
    return commands


def extract_assistant_texts(events: List[dict]) -> List[str]:
    texts: List[str] = []
    for e in events:
        if e.get("type") != "Message":
            continue
        msg = e.get("message")
        if not isinstance(msg, dict):
            continue
        for content in msg.get("content", []) or []:
            if content.get("type") == "text":
                t = content.get("text")
                if isinstance(t, str) and t.strip():
                    texts.append(t.strip())
    return texts


def score_probe_commands(commands: List[str]) -> Dict[str, Any]:
    probes = []
    for cmd in commands:
        lowered = cmd.lower()
        if any(marker.lower() in lowered for marker in PROBE_MARKERS):
            probes.append(cmd)
    return {
        "probeCount": len(probes),
        "probeCommands": probes,
    }


def find_sessions_db(path_root: Optional[str]) -> Optional[Path]:
    candidates: List[Path] = []
    if path_root:
        candidates.append(Path(path_root) / "data" / "sessions" / "sessions.db")
    env_path_root = os.environ.get("PATH_ROOT")
    if env_path_root:
        candidates.append(Path(env_path_root) / "data" / "sessions" / "sessions.db")
    appdata = os.environ.get("APPDATA")
    if appdata:
        candidates.append(Path(appdata) / "AGIME" / "agime" / "data" / "sessions" / "sessions.db")
    candidates.append(Path("C:/Users/jsjm/AppData/Roaming/AGIME/agime/data/sessions/sessions.db"))
    for path in candidates:
        if path.exists():
            return path
    return None


def db_memory_rows(db_path: Path, session_id: str) -> Dict[str, Any]:
    out: Dict[str, Any] = {
        "facts": [],
        "candidates": [],
    }
    conn = sqlite3.connect(str(db_path))
    conn.row_factory = sqlite3.Row
    try:
        cur = conn.cursor()
        cur.execute(
            """
            SELECT category, content, status, source, confidence, evidence_count, validation_command
            FROM memory_facts
            WHERE session_id = ?
            ORDER BY updated_at DESC, created_at DESC
            """,
            (session_id,),
        )
        out["facts"] = [dict(r) for r in cur.fetchall()]
        cur.execute(
            """
            SELECT category, content, source, decision, reason
            FROM memory_candidates
            WHERE session_id = ?
            ORDER BY created_at DESC
            LIMIT 200
            """,
            (session_id,),
        )
        out["candidates"] = [dict(r) for r in cur.fetchall()]
    finally:
        conn.close()
    return out


def evaluate_quality(
    facts_api: List[dict],
    candidates_api: List[dict],
    tool_gates: List[dict],
    second_turn_commands: List[str],
) -> Dict[str, Any]:
    active_facts = [f for f in facts_api if str(f.get("status", "")).lower() == "active"]
    active_artifacts = [f for f in active_facts if "artifact" in str(f.get("category", "")).lower()]
    active_invalids = [f for f in active_facts if "invalid_path" in str(f.get("category", "")).lower()]

    date_facts = [f for f in active_facts if isinstance(f.get("content"), str) and DATE_ONLY_RE.match(f["content"].strip())]
    wrong_desktop = [
        f
        for f in active_artifacts
        if isinstance(f.get("content"), str) and DESKTOP_WRONG_RE.match(f["content"].strip())
    ]
    onedrive_desktop = [
        f
        for f in active_artifacts
        if isinstance(f.get("content"), str)
        and "onedrive" in f["content"].lower()
        and f["content"].lower().endswith("desktop")
    ]

    accepted_date_candidates = [
        c
        for c in candidates_api
        if str(c.get("decision", "")).lower() == "accepted"
        and isinstance(c.get("content"), str)
        and DATE_ONLY_RE.match(c["content"].strip())
    ]

    probe_stats = score_probe_commands(second_turn_commands)
    second_turn_repeated_probe = probe_stats["probeCount"] > 0 and len(onedrive_desktop) > 0

    wrong_gate_paths = [
        g
        for g in tool_gates
        if isinstance(g.get("path"), str) and DESKTOP_WRONG_RE.match(g["path"].strip())
    ]

    issues = []
    if date_facts:
        issues.append("date_noise_fact_active")
    if accepted_date_candidates:
        issues.append("date_noise_candidate_accepted")
    if wrong_desktop and not any(
        isinstance(f.get("content"), str)
        and f.get("content", "").lower() == wrong_desktop[0].get("content", "").lower()
        for f in active_invalids
    ):
        issues.append("wrong_desktop_without_invalid_marker")
    if second_turn_repeated_probe:
        issues.append("second_turn_repeated_probe_despite_memory")
    if wrong_gate_paths:
        issues.append("tool_gate_reused_wrong_path")

    return {
        "activeFactCount": len(active_facts),
        "activeArtifactCount": len(active_artifacts),
        "activeInvalidPathCount": len(active_invalids),
        "dateNoiseFacts": date_facts,
        "acceptedDateCandidates": accepted_date_candidates,
        "wrongDesktopArtifacts": wrong_desktop,
        "oneDriveDesktopArtifacts": onedrive_desktop,
        "probeStatsSecondTurn": probe_stats,
        "wrongToolGatePaths": wrong_gate_paths,
        "issues": issues,
    }


def parse_args() -> argparse.Namespace:
    ap = argparse.ArgumentParser(description="CFPM API evaluation runner")
    ap.add_argument(
        "--server-exe",
        default="ui/desktop/out/AGIME-win32-x64/resources/bin/agimed.exe",
        help="path to agimed.exe",
    )
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=31337)
    ap.add_argument("--secret", default="cfpm-eval-secret")
    ap.add_argument("--working-dir", default=str(Path.cwd()))
    ap.add_argument("--provider", default=None)
    ap.add_argument("--model", default=None)
    ap.add_argument(
        "--prompt-1",
        default="请用命令行查看我的桌面文件，先确认真实桌面路径再列出文件。",
    )
    ap.add_argument(
        "--prompt-2",
        default="再看一遍我的桌面文件，优先复用你已经确认过的正确路径。",
    )
    ap.add_argument("--path-root", default=None, help="optional PATH_ROOT used by server")
    ap.add_argument("--no-start-server", action="store_true")
    ap.add_argument("--output", default=None)
    return ap.parse_args()


def main() -> int:
    args = parse_args()
    base_url = f"http://{args.host}:{args.port}"
    client = ApiClient(base_url=base_url, secret=args.secret)

    server_proc: Optional[subprocess.Popen] = None
    report: Dict[str, Any] = {
        "startedAt": now_ts(),
        "baseUrl": base_url,
        "steps": [],
    }

    try:
        if not args.no_start_server:
            exe = Path(args.server_exe)
            if not exe.exists():
                raise FileNotFoundError(f"server exe not found: {exe}")
            server_proc = start_server(exe=exe, host=args.host, port=args.port, secret=args.secret)
            wait_for_status(client, max_wait_sec=45)
            report["steps"].append("server_started")
        else:
            wait_for_status(client, max_wait_sec=15)
            report["steps"].append("server_reused")

        provider, model = select_provider_model(client, args.provider, args.model)
        report["provider"] = provider
        report["model"] = model

        session = client.request_json(
            "POST",
            "/agent/start",
            {"working_dir": args.working_dir},
        )
        session_id = session["id"]
        report["sessionId"] = session_id
        report["steps"].append("session_started")

        client.request_json(
            "POST",
            "/agent/update_provider",
            {
                "provider": provider,
                "model": model,
                "session_id": session_id,
            },
        )
        report["steps"].append("provider_updated")

        turn1_req = {
            "session_id": session_id,
            "recipe_name": None,
            "recipe_version": None,
            "messages": [default_message(args.prompt_1)],
        }
        turn1_events = client.stream_reply(turn1_req, timeout_sec=240)
        report["turn1"] = {
            "eventCount": len(turn1_events),
            "eventTypes": [e.get("type") for e in turn1_events],
            "assistantTexts": extract_assistant_texts(turn1_events),
            "error": next((e.get("error") for e in turn1_events if e.get("type") == "Error"), None),
        }

        turn2_req = {
            "session_id": session_id,
            "recipe_name": None,
            "recipe_version": None,
            "messages": [default_message(args.prompt_2)],
        }
        turn2_events = client.stream_reply(turn2_req, timeout_sec=240)
        report["turn2"] = {
            "eventCount": len(turn2_events),
            "eventTypes": [e.get("type") for e in turn2_events],
            "assistantTexts": extract_assistant_texts(turn2_events),
            "error": next((e.get("error") for e in turn2_events if e.get("type") == "Error"), None),
        }

        session_after = client.request_json("GET", f"/sessions/{urllib.parse.quote(session_id)}")
        facts_api = client.request_json("GET", f"/sessions/{urllib.parse.quote(session_id)}/memory/facts") or []
        candidates_api = (
            client.request_json(
                "GET",
                f"/sessions/{urllib.parse.quote(session_id)}/memory/candidates?limit=200",
            )
            or []
        )
        tool_gates = (
            client.request_json(
                "GET",
                f"/sessions/{urllib.parse.quote(session_id)}/memory/tool-gates?limit=200",
            )
            or []
        )

        commands_all = extract_tool_commands_from_session(session_after or {})
        # Approximate second turn commands by tail segment.
        commands_second_turn = commands_all[-30:]

        report["apiSnapshot"] = {
            "factsCount": len(facts_api),
            "candidatesCount": len(candidates_api),
            "toolGateCount": len(tool_gates),
            "toolCommandsCaptured": len(commands_all),
        }

        quality = evaluate_quality(facts_api, candidates_api, tool_gates, commands_second_turn)
        report["quality"] = quality

        db_path = find_sessions_db(args.path_root)
        report["dbPath"] = str(db_path) if db_path else None
        if db_path and db_path.exists():
            db_rows = db_memory_rows(db_path, session_id)
            report["dbSnapshot"] = {
                "factRows": len(db_rows["facts"]),
                "candidateRows": len(db_rows["candidates"]),
            }
            api_fact_keys = {
                (
                    str(f.get("category", "")),
                    str(f.get("content", "")),
                    str(f.get("status", "")),
                    str(f.get("source", "")),
                )
                for f in facts_api
            }
            db_fact_keys = {
                (
                    str(f.get("category", "")),
                    str(f.get("content", "")),
                    str(f.get("status", "")),
                    str(f.get("source", "")),
                )
                for f in db_rows["facts"]
            }
            report["apiDbFactDelta"] = {
                "apiOnly": sorted(list(api_fact_keys - db_fact_keys))[:20],
                "dbOnly": sorted(list(db_fact_keys - api_fact_keys))[:20],
            }
        else:
            report["dbSnapshot"] = {"warning": "sessions.db not found"}

        report["finishedAt"] = now_ts()
        report["success"] = True

    except Exception as exc:
        report["finishedAt"] = now_ts()
        report["success"] = False
        report["error"] = f"{type(exc).__name__}: {exc}"
    finally:
        if server_proc is not None:
            try:
                server_proc.terminate()
                server_proc.wait(timeout=8)
            except Exception:
                try:
                    server_proc.kill()
                except Exception:
                    pass

    output_path = (
        Path(args.output)
        if args.output
        else Path("data") / f"cfpm_eval_report_{int(time.time())}.json"
    )
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")

    print(json.dumps({
        "success": report.get("success"),
        "output": str(output_path),
        "sessionId": report.get("sessionId"),
        "issues": report.get("quality", {}).get("issues", []),
        "error": report.get("error"),
    }, ensure_ascii=False, indent=2))
    return 0 if report.get("success") else 1


if __name__ == "__main__":
    sys.exit(main())
