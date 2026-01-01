"""
Test script to check if GPT-5.2 API returns thinking/reasoning content.
This script directly calls the API to diagnose where the issue is.
"""
import json
import requests

# Configuration from custom_gpt-5.2.json
BASE_URL = "https://xiaohumini.site"  # Note: removed trailing space
MODEL = "gpt-5.2"

def test_api_response():
    """Test the API response format"""

    endpoint = f"{BASE_URL.strip()}/v1/chat/completions"

    headers = {
        "Content-Type": "application/json",
    }

    # Simple test request that should trigger thinking
    payload = {
        "model": MODEL,
        "messages": [
            {
                "role": "user",
                "content": "What is 15 * 23? Please show your thinking process."
            }
        ],
        "stream": False
    }

    print(f"=" * 60)
    print(f"Testing GPT-5.2 API (No Auth)")
    print(f"=" * 60)
    print(f"Endpoint: {endpoint}")
    print(f"Model: {MODEL}")
    print(f"=" * 60)

    print("\n[1] Testing non-streaming response...")
    try:
        response = requests.post(endpoint, headers=headers, json=payload, timeout=120)

        print(f"Status Code: {response.status_code}")

        if response.status_code == 200:
            data = response.json()

            # Pretty print the raw response
            print("\n[RAW RESPONSE]:")
            print(json.dumps(data, indent=2, ensure_ascii=False))

            # Check for thinking/reasoning content
            print("\n[ANALYSIS]:")

            if "choices" in data and len(data["choices"]) > 0:
                choice = data["choices"][0]
                message = choice.get("message", {})

                # Check various possible thinking fields
                fields_to_check = [
                    "reasoning_content",  # DeepSeek/Qwen style
                    "thinking",           # Alternative name
                    "reasoning",          # Another alternative
                    "thought",            # Yet another
                ]

                found_thinking = False
                for field in fields_to_check:
                    if field in message:
                        value = message[field]
                        if value:
                            print(f"  [FOUND] {field}: {value[:200] if len(value) > 200 else value}...")
                            found_thinking = True
                        else:
                            print(f"  [FOUND BUT EMPTY] {field} exists but is empty/null")

                if not found_thinking:
                    print("  [NOT FOUND] No thinking/reasoning field in message!")
                    print(f"  Message keys: {list(message.keys())}")

                # Check content for inline thinking tags
                content = message.get("content", "")
                if "<think>" in content or "</think>" in content:
                    print("  [FOUND] Inline <think> tags in content")
                elif "<thinking>" in content or "</thinking>" in content:
                    print("  [FOUND] Inline <thinking> tags in content")
                else:
                    print("  [NOT FOUND] No inline thinking tags in content")

                print(f"\n  Content preview: {content[:300]}...")
        else:
            print(f"[ERROR] API returned error: {response.text}")

    except Exception as e:
        print(f"[ERROR] Request failed: {e}")

    # Test streaming response
    print("\n" + "=" * 60)
    print("[2] Testing streaming response...")

    payload["stream"] = True

    try:
        response = requests.post(endpoint, headers=headers, json=payload, timeout=120, stream=True)

        print(f"Status Code: {response.status_code}")

        if response.status_code == 200:
            print("\n[STREAMING CHUNKS]:")
            chunk_count = 0
            found_reasoning_in_stream = False
            all_fields_seen = set()

            for line in response.iter_lines():
                if line:
                    line_str = line.decode('utf-8')
                    if line_str.startswith("data: "):
                        data_str = line_str[6:]
                        if data_str == "[DONE]":
                            print("\n  [DONE]")
                            break

                        try:
                            chunk = json.loads(data_str)
                            chunk_count += 1

                            # Show first few chunks in detail
                            if chunk_count <= 3:
                                print(f"\n  Chunk {chunk_count}:")
                                print(f"  {json.dumps(chunk, indent=4, ensure_ascii=False)}")

                            # Check for reasoning_content in delta
                            if "choices" in chunk and len(chunk["choices"]) > 0:
                                delta = chunk["choices"][0].get("delta", {})
                                all_fields_seen.update(delta.keys())

                                for field in ["reasoning_content", "thinking", "reasoning"]:
                                    if field in delta and delta[field]:
                                        found_reasoning_in_stream = True
                                        if chunk_count <= 5:
                                            print(f"  [FOUND] {field} in delta!")

                        except json.JSONDecodeError:
                            pass

            print(f"\n  Total chunks: {chunk_count}")
            print(f"  All delta fields seen: {all_fields_seen}")
            print(f"  Found reasoning_content in stream: {found_reasoning_in_stream}")
        else:
            print(f"[ERROR] Streaming API error: {response.text}")

    except Exception as e:
        print(f"[ERROR] Streaming request failed: {e}")

    print("\n" + "=" * 60)
    print("[CONCLUSION]:")
    print("=" * 60)
    print("If no 'reasoning_content' field was found, the API itself")
    print("does not return thinking content in a standard format.")
    print("")
    print("Possible solutions:")
    print("1. Check if the API requires special parameters to enable thinking")
    print("2. Check if thinking is returned in a different field name")
    print("3. Contact the API provider about thinking/reasoning support")
    print("=" * 60)

if __name__ == "__main__":
    test_api_response()
