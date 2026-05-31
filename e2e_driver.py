#!/usr/bin/env python
# -*- coding: utf-8 -*-
"""
E2E 真实任务测试驱动 — 多模态治理 + tasks + subagent + swarm + thinking
依赖: 仅标准库 (urllib). target/release/agimed.exe 已启动并监听 127.0.0.1:3456
凭证: keyring(service=agime) 中的 mimo provider key (桌面版已存)
"""
import json, sys, time, urllib.request, urllib.error, io

SECRET = "e2e-test-secret-key"
BASE = "http://127.0.0.1:3456"
WORKDIR = "E:/yw/agiatme/goose/.e2e_workdir"
import os
_FIX = os.environ.get("AGIME_PNG_FIXTURE",
    r"C:\Users\jsjm\AppData\Local\Temp\agime_test\red_px_b64.txt")
RED_PNG_B64 = open(_FIX).read().strip()

results = []
def rec(name, ok, detail=""):
    mark = "PASS" if ok else "FAIL"
    line = f"[{mark}] {name}" + (f" — {detail}" if detail else "")
    print(line, flush=True)
    results.append((name, ok, detail))

def _req(path, body=None, method="POST", stream=False, timeout=120):
    url = BASE + path
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    req.add_header("X-Secret-Key", SECRET)
    return urllib.request.urlopen(req, timeout=timeout)

def call(path, body=None, method="POST", timeout=30):
    try:
        r = _req(path, body, method, timeout=timeout)
        raw = r.read().decode("utf-8", "replace")
        try: return r.status, json.loads(raw)
        except Exception: return r.status, raw
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")
    except Exception as e:
        return -1, str(e)

def reply_sse(session_id, messages, timeout=120):
    """POST /reply, return list of parsed SSE event dicts."""
    body = {"messages": messages, "session_id": session_id,
            "recipe_name": None, "recipe_version": None}
    events = []
    try:
        r = _req("/reply", body, timeout=timeout)
        buf = ""
        start = time.time()
        for chunk in r:
            buf += chunk.decode("utf-8", "replace")
            while "\n\n" in buf:
                blk, buf = buf.split("\n\n", 1)
                for ln in blk.splitlines():
                    if ln.startswith("data: "):
                        js = ln[6:]
                        try: events.append(json.loads(js))
                        except Exception: events.append({"_raw": js})
            if time.time() - start > timeout:
                break
    except Exception as e:
        events.append({"_error": str(e)})
    return events

def now(): return int(time.time())
_META = {"userVisible": True, "agentVisible": True}
def m_text(t): return {"role":"user","created":now(),"metadata":_META,
                       "content":[{"type":"text","text":t}]}
def m_img(t):  return {"role":"user","created":now(),"metadata":_META,
                       "content":[{"type":"text","text":t},
                                  {"type":"image","data":RED_PNG_B64,"mimeType":"image/png"}]}

def events_summary(events):
    types = {}
    for e in events:
        t = e.get("type") or ("_error" if "_error" in e else "_raw")
        types[t] = types.get(t,0)+1
    return types

def collect_text(events):
    """gather all assistant text + systemNotification text from Message events"""
    out = []
    for e in events:
        if e.get("type") == "Message":
            for c in e.get("message",{}).get("content",[]):
                if c.get("type") == "text": out.append(("text", c.get("text","")))
                elif c.get("type") == "systemNotification":
                    out.append(("sysnotif", json.dumps(c, ensure_ascii=False)))
                elif c.get("type") == "thinking":
                    out.append(("thinking", c.get("thinking","")))
                elif c.get("type") == "toolRequest":
                    out.append(("toolReq", json.dumps(c, ensure_ascii=False)))
                elif c.get("type") == "toolResponse":
                    out.append(("toolResp", json.dumps(c, ensure_ascii=False)[:2000]))
        elif e.get("type") == "Error":
            out.append(("error", e.get("error","")))
        elif e.get("type") == "HarnessControl":
            out.append(("harness", json.dumps(e.get("envelope",{}), ensure_ascii=False)[:1500]))
    return out

def add_builtin_ext(session_id, name, description="", timeout=300):
    cfg = {"type":"builtin","name":name,"description":description,
           "display_name":name,"timeout":timeout,"bundled":True,"available_tools":[]}
    return call("/agent/add_extension", {"session_id":session_id,"config":cfg}, timeout=60)

def add_platform_ext(session_id, name, description="", timeout=300):
    cfg = {"type":"platform","name":name,"description":description,
           "bundled":True,"available_tools":[]}
    return call("/agent/add_extension", {"session_id":session_id,"config":cfg}, timeout=60)

def session_override(session_id, provider, model, thinking_enabled=True,
                     thinking_budget=8000, supports_multimodal=False):
    """Drive thinking like the desktop via HostProviderConfig."""
    pc = {"name": provider, "model": model,
          "thinking_enabled": thinking_enabled,
          "thinking_budget": thinking_budget,
          "supports_multimodal": supports_multimodal}
    return call("/agent/session_override",
                {"session_id": session_id, "provider_config": pc}, timeout=60)

if __name__ == "__main__":
    print("driver-loaded", flush=True)