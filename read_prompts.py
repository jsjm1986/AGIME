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
for root, dirs, files in os.walk(base_path):
    for f in files:
        if f.endswith('.md'):
            full_path = os.path.join(root, f)
            print(f"=== {full_path} ===")
            content = read_file_any_encoding(full_path)
            if content:
                print(content[:1500])
            else:
                print("Could not read file")
            print("\n" + "="*50 + "\n")
