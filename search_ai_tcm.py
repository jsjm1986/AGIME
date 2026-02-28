import urllib.request, urllib.parse, re, ssl

ctx = ssl.create_default_context()
ctx.check_hostname = False
ctx.verify_mode = ssl.CERT_NONE

headers = {'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36'}

queries = [
    "AI医疗 2025 投融资 市场规模 趋势",
    "AI 中医 大模型 舌诊 脉诊 应用 2025",
    "AI健康平台 融资 竞品 2025 2026",
    "中医药 AI 政策 国家支持 2025",
]

for q in queries:
    print(f"\n{'='*60}")
    print(f"搜索: {q}")
    print('='*60)
    url = "https://html.duckduckgo.com/html/?q=" + urllib.parse.quote(q)
    req = urllib.request.Request(url, headers=headers)
    try:
        resp = urllib.request.urlopen(req, context=ctx, timeout=15)
        html = resp.read().decode('utf-8', errors='ignore')
        titles = re.findall(r'class="result__title"[^>]*>.*?<a[^>]*>(.*?)</a>', html, re.DOTALL)
        snippets = re.findall(r'class="result__snippet"[^>]*>(.*?)</(?:div|td)', html, re.DOTALL)
        for i in range(min(5, len(titles))):
            t = re.sub(r'<[^>]+>', '', titles[i]).strip()
            s = re.sub(r'<[^>]+>', '', snippets[i]).strip() if i < len(snippets) else ''
            print(f"\n{i+1}. {t}")
            print(f"   {s[:250]}")
    except Exception as e:
        print(f"Error: {e}")
