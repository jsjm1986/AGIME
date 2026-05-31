# -*- coding: utf-8 -*-
"""任务系统 / subagent / swarm / thinking 真实测试。
两种模型：mimo-v2.5-pro(文字), mimo-v2.5(多模态)。"""
import sys, json, time
import e2e_driver as d

WORKDIR = "E:/yw/agiatme/goose/.e2e_workdir"
PROVIDER = "custom_mimo-v2.5-pro_"
TEXT_MODEL = "mimo-v2.5-pro"
MM_MODEL = "mimo-v2.5"

def session(model, exts=("developer",), platform_exts=()):
    st, body = d.call("/agent/start", {"working_dir": WORKDIR})
    sid = body["id"]
    d.call("/agent/update_provider",
           {"session_id": sid, "provider": PROVIDER, "model": model})
    for e in exts:
        d.add_builtin_ext(sid, e, f"{e} tools")
    for e in platform_exts:
        d.add_platform_ext(sid, e, f"{e} platform extension")
    return sid

def texts(events):
    return [t for k, t in d.collect_text(events) if k == "text"]

def tool_reqs(events):
    out = []
    for k, t in d.collect_text(events):
        if k == "toolReq":
            try:
                j = json.loads(t)
                out.append(j.get("toolCall", {}).get("value", {}).get("name", "?"))
            except Exception:
                pass
    return out

def harness_kinds(events):
    """Collect harness payload channel/event-type pairs + any swarm/subagent words."""
    sigs = []
    for e in events:
        if e.get("type") == "HarnessControl":
            env = e.get("envelope", {})
            pl = env.get("payload", {})
            ch = pl.get("channel")
            ev = pl.get("event", {})
            sigs.append(f"{ch}:{ev.get('type')}")
    return sigs

def dump(events, label, show_thinking=False):
    print(f"  [{label}] summary={d.events_summary(events)}")
    print(f"    toolReqs={tool_reqs(events)}")
    for k, t in d.collect_text(events):
        if k == "harness":
            continue
        if k == "thinking" and not show_thinking:
            print(f"    thinking: <{len(t)} chars>")
            continue
        print(f"    {k}: {t[:260].replace(chr(10),' ')}")

def run_one(sid, prompt, timeout=180):
    return d.reply_sse(sid, [d.m_text(prompt)], timeout=timeout)

# ============================================================
print("=" * 70); print("TEST tasks: 任务系统 (TaskCreate/Update/List)"); print("=" * 70)
sid = session(TEXT_MODEL, exts=("developer",), platform_exts=("tasks",))
ev = run_one(sid,
    "请使用任务系统(TaskCreate)创建一个包含三个步骤的任务清单："
    "1) 读取目录 2) 统计文件数 3) 汇报结果；创建后用 TaskList 列出它们。"
    "只要演示任务系统即可，不需要真正执行步骤。", timeout=180)
dump(ev, "tasks")
treqs = tool_reqs(ev)
tasks_ok = any("Task" in r or "task" in r for r in treqs)
d.rec("tasks: 任务系统工具被调用", tasks_ok, f"toolReqs={treqs}")

# ============================================================
print("=" * 70); print("TEST subagent: 子代理委派"); print("=" * 70)
sid = session(TEXT_MODEL, exts=("developer",))
ev = d.reply_sse(sid,
    [d.m_text("请把下面这件事委派给一个 subagent 子代理来独立完成，并汇总它的结果："
              "用一句话解释什么是快速排序算法。明确使用 subagent 委派。")],
    timeout=240)
dump(ev, "subagent")
treqs = tool_reqs(ev)
hk = harness_kinds(ev)
sub_ok = ("subagent" in treqs) or any("subagent" in str(x).lower() for x in hk) \
         or any("subagent" in t.lower() for k,t in d.collect_text(ev) if k=="harness")
d.rec("subagent: 子代理委派触发", sub_ok, f"toolReqs={treqs}")

# ============================================================
print("=" * 70); print("TEST swarm: 并行委派"); print("=" * 70)
sid = session(TEXT_MODEL, exts=("developer",))
ev = d.reply_sse(sid,
    [d.m_text("请使用 swarm 并行委派：同时启动多个并行 worker，"
              "分别用一句话回答：(a) 什么是HTTP (b) 什么是TCP (c) 什么是DNS，"
              "最后把三个并行结果汇总给我。明确使用 swarm 并行。")],
    timeout=300)
dump(ev, "swarm")
treqs = tool_reqs(ev)
allh = " ".join(t.lower() for k,t in d.collect_text(ev) if k=="harness")
swarm_ok = ("swarm" in treqs) or ("swarm" in allh)
d.rec("swarm: 并行委派触发", swarm_ok, f"toolReqs={treqs} swarm_in_harness={'swarm' in allh}")

# ============================================================
print("=" * 70); print("TEST thinking: 思考模式输出"); print("=" * 70)
sid = session(TEXT_MODEL, exts=("developer",))
ev = d.reply_sse(sid,
    [d.m_text("请仔细思考后回答：一个农夫要把狼、羊、白菜运过河，"
              "船每次只能载农夫和一样东西，狼吃羊、羊吃白菜，怎么安全运过河？"
              "请展示你的推理过程。")],
    timeout=240)
think = [t for k,t in d.collect_text(ev) if k == "thinking" and t.strip()]
dump(ev, "thinking")
think_ok = len(think) > 0
d.rec("thinking: 思考内容输出", think_ok,
      f"thinking_blocks={len(think)} total_chars={sum(len(t) for t in think)}")

# ============================================================
print("=" * 70); print("SUMMARY"); print("=" * 70)
ok = sum(1 for _, o, _ in d.results if o)
for name, o, detail in d.results:
    print(("PASS" if o else "FAIL"), name, "—", detail)
print(f"\n{ok}/{len(d.results)} passed")
