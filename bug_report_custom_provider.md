# Bug Report: 通过 UI 创建自定义服务商时配置文件未被持久化

## Bug 描述

通过 UI 创建自定义服务商（Custom Provider）时，`custom_providers` 目录和对应的 JSON 配置文件没有被创建，但系统没有显示任何错误提示。

## 复现步骤

1. 打开 Goose Desktop 应用
2. 进入设置 -> 模型 -> 添加自定义服务商
3. 填写服务商信息（如 DeepSeek）并保存
4. 尝试打开"切换模型"或"主/工作模式"对话框

## 预期行为

- `custom_providers` 目录应该被创建在 `%APPDATA%\Block\goose\config\custom_providers\`
- 对应的 JSON 配置文件（如 `custom_deepseek.json`）应该被写入
- 如果创建失败，应该向用户显示错误提示

## 实际行为

- `custom_providers` 目录不存在
- JSON 配置文件未被创建
- `config.yaml` 中 `GOOSE_PROVIDER` 被设置为 `custom_deepseek`
- AI 推理可以正常工作
- 但"切换模型"和"主/工作模式"对话框一直显示"加载中..."（无限加载）

## 根本原因分析

`crates/goose/src/config/declarative_providers.rs` 中的 `create_custom_provider` 函数应该：
1. 创建 `custom_providers` 目录
2. 写入 JSON 配置文件
3. 保存 API 密钥到密钥存储

但实际上文件没有被创建，可能原因：
1. 文件写入权限问题
2. 错误被静默忽略，没有向用户显示
3. 异步操作没有正确等待完成

## 临时解决方案

手动创建目录和配置文件：

```bash
mkdir "%APPDATA%\Block\goose\config\custom_providers"
```

创建 `custom_deepseek.json`：
```json
{
  "name": "custom_deepseek",
  "engine": "openai",
  "display_name": "DeepSeek",
  "description": "Custom DeepSeek provider",
  "api_key_env": "CUSTOM_DEEPSEEK_API_KEY",
  "base_url": "https://api.deepseek.com/v1",
  "models": [
    {"name": "deepseek-chat", "context_length": 128000}
  ],
  "supports_streaming": true
}
```

## 环境信息

- 操作系统：Windows
- Goose 版本：1.16.0
- 配置目录：`C:\Users\<user>\AppData\Roaming\Block\goose\config\`

## 建议修复

1. 在 `create_custom_provider` 函数中添加更好的错误处理
2. 如果文件创建失败，向用户显示明确的错误信息
3. 在 `get_provider_models` API 中，当找不到自定义服务商配置时返回更有意义的错误信息，而不是让 UI 无限加载

---
Labels: bug
