"""
Quick test for GPT-5.2 API connectivity
"""
import json
import requests

BASE_URL = "https://xiaohumini.site"
MODEL = "gpt-5.2"

endpoint = f"{BASE_URL}/v1/chat/completions"

headers = {"Content-Type": "application/json"}
payload = {
    "model": MODEL,
    "messages": [{"role": "user", "content": "hi"}],
    "stream": False
}

print(f"Testing: {endpoint}")
print(f"Model: {MODEL}")

try:
    response = requests.post(endpoint, headers=headers, json=payload, timeout=30)
    print(f"Status: {response.status_code}")
    print(f"Response: {response.text[:500]}")
except requests.exceptions.Timeout:
    print("Request timed out after 30s")
except Exception as e:
    print(f"Error: {e}")
