import os
import glob

def read_file_any_encoding(filepath):
    encodings = ['utf-8', 'gb18030', 'gbk', 'gb2312', 'utf-16', 'big5']
    for enc in encodings:
        try:
            with open(filepath, 'r', encoding=enc) as f:
                return f.read()
        except:
            continue
    return None

# Find all markdown files
base_path = 'her prompts'
file_list = []
for root, dirs, files in os.walk(base_path):
    for f in files:
        if f.endswith('.md'):
            full_path = os.path.join(root, f)
            file_list.append(full_path)

# Print file list
print(f"Found {len(file_list)} markdown files:\n")
for i, fp in enumerate(file_list[:50]):
    print(f"{i+1}. {fp}")
print()

# Read first 10 files fully and summarize
for i, fp in enumerate(file_list[:15]):
    content = read_file_any_encoding(fp)
    if content:
        print(f"\n{'='*60}")
        print(f"FILE: {fp}")
        print(f"SIZE: {len(content)} characters")
        print(f"{'='*60}\n")
        print(content[:2000])
        print("\n... [truncated]\n")
