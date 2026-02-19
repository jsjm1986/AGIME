#!/usr/bin/env python3
"""
Evaluate CFPM behavior on an existing session (resume path).

This script avoids "new session has no extension" noise by:
1) starting local agimed (optional)
2) resuming a specified session with load_model_and_extensions=true
3) sending one follow-up prompt via /reply
4) reading facts/candidates/tool-gates + sessions.db rows
5) emitting a compact report for diagnosis
"""

from __future__ import annotations

import argparse
import json
import os
import sqlite3
import subprocess
import sys
import time
import urllib.request
from pathlib import Path
from typing import Any, Dict, Optional


def now_ts() -> int:
    return int(time.time())


def default_message(text: str) -> dict:
    return {
        "role": "user",
        "created": now_ts(),
        "content": [{"type": "text", "text": text}],
        "metadata": {"userVisible": True, "agentVisible": True},
    }


class ApiClient:
    def __init__(self, base_url: str, secret: str):
        self.base_url = base_url
        self.secret = secret

    def req_json(self, method: str, path: str, payload: Optional[dict] = None, timeout: int = 60) -> Any:
        data = None if payload is None else json.dumps(payload).encode("utf-8")
        req = urllib.request.Request(self.base_url + path, data=data, method=method.upper())
        req.add_header("X-Secret-Key", self.secret)
        req.add_header("Content-Type", "application/json")
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read().decode("utf-8", errors="replace")
            return json.loads(raw) if raw else None

    def wait_status(self, wait_sec: int = 30) -> None:
        deadline = time.time() + wait_sec
        while time.time() < deadline:
            try:
                with urllib.request.urlopen(self.base_url + "/status", timeout=2) as r:
                    if r.status == 200:
                        return
            except Exception:
                time.sleep(0.2)
        raise TimeoutError("/status not ready")

    def reply_sse(self, payload: dict, timeout: int = 240) -> Dict[str, Any]:
        req = urllib.request.Request(
            self.base_url + "/reply",
            data=json.dumps(payload).encode("utf-8"),
            method="POST",
        )
        req.add_header("X-Secret-Key", self.secret)
        req.add_header("Content-Type", "application/json")
        req.add_header("Accept", "text/event-stream")

        event_types = []
        assistant_text = []
        system_notes = []
        tool_requests = 0
        error = None
        finish_reason = None

        with urllib.request.urlopen(req, timeout=timeout) as resp:
            for raw in resp:
                line = raw.decode("utf-8", errors="replace").strip()
                if not line.startswith("data:"):
                    continue
                blob = line[5:].strip()
                if not blob:
                    continue
                try:
                    obj = json.loads(blob)
                except Exception:
                    continue
                t = obj.get("type")
                if not isinstance(t, str):
                    continue
                event_types.append(t)
                if t == "Message":
                    msg = obj.get("message", {})
                    for c in msg.get("content", []) or []:
                        if c.get("type") == "text":
                            text = c.get("text")
                            if isinstance(text, str):
                                assistant_text.append(text)
                        if c.get("type") == "toolRequest":
                            tool_requests += 1
                        if c.get("type") == "systemNotification":
                            note = c.get("msg")
                            if isinstance(note, str) and "CFPM" in note:
                                system_notes.append(note)
                if t == "Error":
                    error = obj.get("error")
                    break
                if t == "Finish":
                    finish_reason = obj.get("reason")
                    break

        return {
            "eventTypes": event_types,
            "assistantText": assistant_text,
            "cfpmSystemNotifications": system_notes,
            "toolRequestCount": tool_requests,
            "error": error,
            "finishReason": finish_reason,
        }


def start_server(exe: Path, host: str, port: int, secret: str) -> subprocess.Popen:
    env = os.environ.copy()
    env["AGIME_HOST"] = host
    env["AGIME_PORT"] = str(port)
    env["AGIME_SERVER__SECRET_KEY"] = secret
    return subprocess.Popen(
        [str(exe), "agent"],
        cwd=str(Path.cwd()),
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def find_db(path_root: Optional[str]) -> Optional[Path]:
    cands = []
    if path_root:
        cands.append(Path(path_root) / "data" / "sessions" / "sessions.db")
    appdata = os.environ.get("APPDATA")
    if appdata:
        cands.append(Path(appdata) / "AGIME" / "agime" / "data" / "sessions" / "sessions.db")
    cands.append(Path("C:/Users/jsjm/AppData/Roaming/AGIME/agime/data/sessions/sessions.db"))
    for p in cands:
        if p.exists():
            return p
    return None


def read_db_snapshot(db_path: Path, sid: str) -> Dict[str, Any]:
    conn = sqlite3.connect(str(db_path))
    conn.row_factory = sqlite3.Row
    cur = conn.cursor()
    try:
        cur.execute(
            """
            SELECT category, content, status, source, confidence, evidence_count, validation_command
            FROM memory_facts
            WHERE session_id = ?
            ORDER BY updated_at DESC
            """,
            (sid,),
        )
        facts = [dict(r) for r in cur.fetchall()]
        cur.execute(
            """
            SELECT decision, reason, category, content, source
            FROM memory_candidates
            WHERE session_id = ?
            ORDER BY created_at DESC
            LIMIT 80
            """,
            (sid,),
        )
        candidates = [dict(r) for r in cur.fetchall()]
        return {"facts": facts, "candidates": candidates}
    finally:
        conn.close()


def parse_args() -> argparse.Namespace:
    ap = argparse.ArgumentParser(description="CFPM resume-session evaluator")
    ap.add_argument("--session-id", required=True)
    ap.add_argument("--prompt", default="用命令行查看我的桌面文件，优先复用已验证路径。")
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=31350)
    ap.add_argument("--secret", default="cfpm-eval-secret")
    ap.add_argument("--server-exe", default="ui/desktop/out/AGIME-win32-x64/resources/bin/agimed.exe")
    ap.add_argument("--no-start-server", action="store_true")
    ap.add_argument("--path-root", default=None)
    ap.add_argument("--output", default=None)
    return ap.parse_args()


def main() -> int:
    args = parse_args()
    base = f"http://{args.host}:{args.port}"
    api = ApiClient(base, args.secret)
    report: Dict[str, Any] = {
        "startedAt": now_ts(),
        "baseUrl": base,
        "sessionId": args.session_id,
    }
    proc = None
    try:
        if not args.no_start_server:
            exe = Path(args.server_exe)
            if not exe.exists():
                raise FileNotFoundError(f"server exe missing: {exe}")
            proc = start_server(exe, args.host, args.port, args.secret)
            api.wait_status(45)
            report["server"] = "started"
        else:
            api.wait_status(15)
            report["server"] = "reused"

        resumed = api.req_json(
            "POST",
            "/agent/resume",
            {"session_id": args.session_id, "load_model_and_extensions": True},
            timeout=180,
        )
        report["resumed"] = {
            "name": resumed.get("name") if isinstance(resumed, dict) else None,
            "provider": resumed.get("provider_name") if isinstance(resumed, dict) else None,
        }

        chat = {
            "session_id": args.session_id,
            "messages": [default_message(args.prompt)],
            "recipe_name": None,
            "recipe_version": None,
        }
        sse = api.reply_sse(chat, timeout=300)
        report["reply"] = sse

        facts = api.req_json("GET", f"/sessions/{args.session_id}/memory/facts") or []
        cands = api.req_json("GET", f"/sessions/{args.session_id}/memory/candidates?limit=120") or []
        gates = api.req_json("GET", f"/sessions/{args.session_id}/memory/tool-gates?limit=80") or []
        report["api"] = {
            "factCount": len(facts),
            "candidateCount": len(cands),
            "gateCount": len(gates),
            "facts": facts,
            "candidates": cands,
            "gates": gates,
        }

        db_path = find_db(args.path_root)
        report["dbPath"] = str(db_path) if db_path else None
        if db_path:
            report["db"] = read_db_snapshot(db_path, args.session_id)

        report["success"] = True
    except Exception as e:
        report["success"] = False
        report["error"] = f"{type(e).__name__}: {e}"
    finally:
        report["finishedAt"] = now_ts()
        if proc is not None:
            proc.terminate()
            try:
                proc.wait(timeout=8)
            except Exception:
                proc.kill()

    out = Path(args.output) if args.output else Path("data") / f"cfpm_resume_eval_{int(time.time())}.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")

    print(
        json.dumps(
            {
                "success": report.get("success"),
                "sessionId": args.session_id,
                "output": str(out),
                "error": report.get("error"),
                "toolRequestCount": report.get("reply", {}).get("toolRequestCount"),
                "gateCount": report.get("api", {}).get("gateCount"),
            },
            ensure_ascii=False,
            indent=2,
        )
    )
    return 0 if report.get("success") else 1


if __name__ == "__main__":
    sys.exit(main())

