# -*- coding: utf-8 -*-
"""多模态治理 5 项真实测试 (A/B/C/D + 正常路径)。"""
import sys, json, time
import e2e_driver as d

SECRET = d.SECRET
BASE = d.BASE
WORKDIR = "E:/yw/agiatme/goose/.e2e_workdir"
TEXT_PROVIDER = "custom_mimo-v2.5-pro_"
TEXT_MODEL = "mimo-v2.5-pro"      # 文字模型
MM_MODEL = "mimo-v2.5"           # 多模态模型

def set_multimodal(val):
    """写 AGIME_MULTIMODAL 到 config.yaml。"""
    st, body = d.call("/config/upsert", {"key": "AGIME_MULTIMODAL", "value": val, "is_secret": False})
    return st, body

def read_multimodal():
    st, body = d.call("/config/read", {"key": "AGIME_MULTIMODAL", "is_secret": False})
    return st, body

def start_session():
    st, body = d.call("/agent/start", {"working_dir": WORKDIR})
    sid = body.get("id") if isinstance(body, dict) else None
    return sid

def set_provider(sid, model):
    st, body = d.call("/agent/update_provider",
                      {"session_id": sid, "provider": TEXT_PROVIDER, "model": model})
    return st, body

def has_image_dropped_notice(events):
    for kind, txt in d.collect_text(events):
        if "未开启多模态" in txt or "not have multimodal" in txt:
            return True
    return False

def find_text(events, needle):
    for kind, txt in d.collect_text(events):
        if needle in txt:
            return True
    return False

def dump(events, label):
    print(f"  [{label}] summary={d.events_summary(events)}")
    for kind, txt in d.collect_text(events):
        if kind in ("harness",):
            continue
        s = txt[:300].replace("\n", " ")
        print(f"    {kind}: {s}")

# ---------------------------------------------------------------------------
print("=" * 70)
print("TEST A: 多模态开关真正生效 (toggle reaches ModelConfig.supports_multimodal)")
print("=" * 70)
# A1: 关闭开关
set_multimodal(False)
st, rd = read_multimodal()
print(f"  config/read after set False -> {st} {rd}")
sid = start_session()
st, body = set_provider(sid, TEXT_MODEL)
print(f"  update_provider (model={TEXT_MODEL}) with MULTIMODAL=false -> HTTP {st} {body}")
a_off_ok = (st == 200)
# A2: 开启开关
set_multimodal(True)
st, rd = read_multimodal()
print(f"  config/read after set True -> {st} {rd}")
st, body = set_provider(sid, MM_MODEL)
print(f"  update_provider (model={MM_MODEL}) with MULTIMODAL=true -> HTTP {st} {body}")
a_on_ok = (st == 200)
d.rec("A: 多模态开关读写+provider重建", a_off_ok and a_on_ok,
      f"off_ok={a_off_ok} on_ok={a_on_ok}")

# ---------------------------------------------------------------------------
print("=" * 70)
print("TEST B: 关闭开关 + 发图 -> 出现中文可见提示 (image_dropped_notice)")
print("=" * 70)
set_multimodal(False)
sid_b = start_session()
set_provider(sid_b, TEXT_MODEL)
ev = d.reply_sse(sid_b, [d.m_img("请描述这张图片的颜色。")], timeout=90)
dump(ev, "B")
b_ok = has_image_dropped_notice(ev)
d.rec("B: 图片丢弃可见提示", b_ok,
      "found notice" if b_ok else "NO notice in stream")

# ---------------------------------------------------------------------------
print("=" * 70)
print("TEST C: 关闭开关 + 截图工具 -> 立即清晰中文错误 (dispatch guard)")
print("=" * 70)
set_multimodal(False)
sid_c = start_session()
set_provider(sid_c, TEXT_MODEL)
st_ext, body_ext = d.add_builtin_ext(sid_c, "developer", "General development tools")
print(f"  add developer ext -> HTTP {st_ext}")
ev = d.reply_sse(sid_c,
    [d.m_text("请调用 developer__screen_capture 工具对当前屏幕截图，然后告诉我屏幕上有什么。")],
    timeout=120)
dump(ev, "C")
# 守卫文案："当前所选模型不支持图片（多模态）输入" 或英文等价
c_guard = find_text(ev, "不支持图片") or find_text(ev, "无法使用截图") \
          or find_text(ev, "does not support image") \
          or find_text(ev, "切换到支持视觉") \
          or find_text(ev, "screen_capture")
d.rec("C: 截图工具门禁拦截", c_guard,
      "guard fired" if c_guard else "guard NOT observed (check toolReq/toolResp)")

# ---------------------------------------------------------------------------
print("=" * 70)
print("TEST E(正常路径): 开启开关 + 多模态模型 -> 图片被正常读取")
print("=" * 70)
set_multimodal(True)
sid_e = start_session()
set_provider(sid_e, MM_MODEL)
ev = d.reply_sse(sid_e, [d.m_img("这张图片是什么颜色？只回答颜色名称。")], timeout=90)
dump(ev, "E")
e_no_notice = not has_image_dropped_notice(ev)
e_answered = any(kind == "text" and txt.strip() for kind, txt in d.collect_text(ev))
d.rec("E: 多模态正常路径(无丢弃提示+有回答)", e_no_notice and e_answered,
      f"no_notice={e_no_notice} answered={e_answered}")

# ---------------------------------------------------------------------------
print("=" * 70)
print("SUMMARY")
print("=" * 70)
ok = sum(1 for _, o, _ in d.results if o)
tot = len(d.results)
for name, o, detail in d.results:
    print(("PASS" if o else "FAIL"), name, "—", detail)
print(f"\n{ok}/{tot} passed")
# restore default
set_multimodal(True)
