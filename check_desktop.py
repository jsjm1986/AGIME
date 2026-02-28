import os
import glob

# 获取桌面路径
desktop_paths = [
    os.path.join(os.path.expanduser("~"), "Desktop"),
    os.path.join(os.path.expanduser("~"), "桌面"),
    r"C:\Users\jsjm\Desktop",
    r"C:\Users\jsjm\桌面",
]

print("正在查找桌面文件夹...\n")

for path in desktop_paths:
    print(f"检查路径: {path}")
    if os.path.exists(path):
        print(f"  ✓ 找到桌面文件夹!")
        print(f"\n桌面内容:")
        print("-" * 50)
        
        items = os.listdir(path)
        folders = [item for item in items if os.path.isdir(os.path.join(path, item))]
        files = [item for item in items if os.path.isfile(os.path.join(path, item))]
        
        if folders:
            print(f"\n[文件夹] ({len(folders)}个):")
            for folder in sorted(folders):
                print(f"  📁 {folder}")
        
        if files:
            print(f"\n[文件] ({len(files)}个):")
            for file in sorted(files):
                print(f"  📄 {file}")
        
        if not items:
            print("  (空文件夹)")
        
        print("-" * 50)
        break
    else:
        print(f"  ✗ 路径不存在")

print("\n查找完成!")
