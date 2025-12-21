# AGIME/Goose Windows 构建指南

## 环境要求

- **Visual Studio 2022** (安装在 `E:\vs`)
  - 需要 "Desktop development with C++" 工作负载
  - 需要 Windows SDK 10.0.26100.0
- **Rust 工具链** (使用项目自带的 `.devtools\rust\`)

## 快速构建

在项目根目录双击运行：

```
build-incremental.bat
```

或者在命令提示符中：

```cmd
cd E:\yw\agiatme\goose
build-incremental.bat
```

## 构建输出

成功后，二进制文件位于：
- `target\debug\goose.exe` - CLI 工具
- `target\debug\goosed.exe` - 后端服务

脚本会自动将它们复制到 `ui\desktop\src\bin\`

## 运行应用

```cmd
cd ui\desktop
npm run start-gui
```

## 常见问题

### 1. 进程占用导致权限错误 (os error 5 - 拒绝访问)

如果遇到类似以下错误：
```
error: failed to run custom build command for `rayon-core v1.12.1`
Caused by:
  could not execute process ... (never executed)
Caused by:
  拒绝访问。 (os error 5)
```

**解决方案：**

1. **杀死所有相关进程：**
   ```cmd
   taskkill /F /IM goosed.exe
   taskkill /F /IM goose.exe
   taskkill /F /IM cargo.exe
   taskkill /F /IM rustc.exe
   ```

2. **清理构建缓存：**
   ```powershell
   Remove-Item -Path "target\debug\build" -Recurse -Force
   ```

3. **重新运行构建脚本：**
   ```cmd
   build-incremental.bat
   ```

### 2. aws-lc-sys 构建失败

如果遇到 "missing DEP_AWS_LC_ include" 或类似错误：

**解决方案：**

1. **清理 aws-lc-sys 缓存：**
   ```cmd
   rd /s /q target\debug\build\aws-lc-sys-*
   ```

2. **重新构建：**
   ```cmd
   build-incremental.bat
   ```

### 3. CMake Generator 冲突

如果遇到 "Does not match the generator used previously" 错误：

```cmd
rd /s /q target\debug\build\aws-lc-sys-*
build-incremental.bat
```

### 4. 完整重新编译

如果以上方法都无效，尝试完全清理后重新编译：

```cmd
cargo clean
build-incremental.bat
```

> ⚠️ **注意：** 完整重新编译可能需要 30-60 分钟。

### 5. 通用故障排除步骤

如果构建失败，按以下顺序尝试：

1. **杀死所有进程：**
   ```cmd
   taskkill /F /IM goosed.exe 2>nul
   taskkill /F /IM goose.exe 2>nul
   taskkill /F /IM cargo.exe 2>nul
   taskkill /F /IM rustc.exe 2>nul
   ```

2. **清理整个 build 目录：**
   ```powershell
   Remove-Item -Path "target\debug\build" -Recurse -Force -ErrorAction SilentlyContinue
   ```

3. **重新运行构建：**
   ```cmd
   build-incremental.bat
   ```

如果仍然失败，执行完整清理：

```cmd
cargo clean
build-incremental.bat
```

## 构建脚本说明

`build-incremental.bat` 会自动：
- 设置 Visual Studio 编译环境
- 配置 Rust 工具链路径
- 设置 AWS-LC-SYS 所需的环境变量
- 使用 Ninja 作为 CMake 生成器
- 编译 `goose-cli` 和 `goose-server`
- 复制编译好的二进制到 `ui\desktop\src\bin\`

## 环境变量

构建脚本会自动设置以下关键环境变量：

| 变量 | 值 | 说明 |
|------|-----|------|
| `AWS_LC_SYS_NO_ASM` | `1` | 禁用汇编优化 |
| `CMAKE_GENERATOR` | `Ninja` | 使用 Ninja 构建 |
| `CC` / `CXX` | `cl.exe` | 使用 MSVC 编译器 |

---
*最后更新: 2025-12-21*
