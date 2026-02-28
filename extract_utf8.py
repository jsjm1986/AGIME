# -*- coding: utf-8 -*-
from docx import Document
import json
import sys

# Set stdout encoding
sys.stdout.reconfigure(encoding='utf-8')

doc = Document('E:/yw/agiatme/goose/data/workspaces/698616a1980c003c66f6421e/doc-analysis/2a4f1523-e885-49e3-b5e9-20507002ad93/documents/doc_fe921b159ff6ab5f.docx')

# Extract all paragraphs with their styles
content = []
for para in doc.paragraphs:
    text = para.text.strip()
    if not text:
        continue
    
    style = para.style.name if para.style else 'Normal'
    
    content.append({
        'text': text,
        'style': style
    })

# Save to JSON file
output_data = {
    'paragraphs': content,
    'total_paragraphs': len(content)
}

with open('E:/yw/agiatme/goose/doc_content_utf8.json', 'w', encoding='utf-8') as f:
    json.dump(output_data, f, ensure_ascii=False, indent=2)

# Print all entries to a file instead of stdout
with open('E:/yw/agiatme/goose/output.txt', 'w', encoding='utf-8') as f:
    for i, item in enumerate(content):
        style = item['style']
        text = item['text']
        f.write(f"{i}: [{style}] {text}\n")

print(f"Total paragraphs: {len(content)}")
print("Output saved to output.txt")
