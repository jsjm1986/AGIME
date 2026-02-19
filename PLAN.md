# 文档频道功能增强计划

## 概述
基于现有文档协作系统（Phase 1-3 已完成），补充缺失的核心功能，提升文件管理体验和协作能力。

---

## 第一批：核心文件管理（后端 + 前端）

### 1.1 后端 - 新增 API

**文件：`crates/agime-team/src/services/document_service_mongo.rs`**
- 新增 `rename(doc_id, new_name)` - 重命名文档
- 新增 `move_document(doc_id, new_folder_path)` - 移动文档到其他文件夹
- 新增 `copy_document(doc_id, user_id, target_folder_path)` - 复制文档
- 新增 `list_deleted(team_id, page, limit)` - 列出已删除文档（回收站）
- 新增 `restore(doc_id)` - 恢复已删除文档
- 新增 `permanent_delete(doc_id)` - 永久删除

**文件：`crates/agime-team/src/services/folder_service_mongo.rs`**
- 新增 `rename(folder_id, new_name)` - 重命名文件夹（需更新 full_path 及子文件夹/文档的路径）
- 新增 `move_folder(folder_id, new_parent_path)` - 移动文件夹

**文件：`crates/agime-team/src/routes/mongo/documents.rs`**
- `PUT /teams/{team_id}/documents/{doc_id}/rename` - 重命名
- `PUT /teams/{team_id}/documents/{doc_id}/move` - 移动
- `POST /teams/{team_id}/documents/{doc_id}/copy` - 复制
- `GET /teams/{team_id}/documents/trash` - 回收站列表
- `POST /teams/{team_id}/documents/{doc_id}/restore` - 恢复
- `DELETE /teams/{team_id}/documents/{doc_id}/permanent` - 永久删除

**文件：`crates/agime-team/src/routes/mongo/folders.rs`**
- `PUT /teams/{team_id}/folders/{folder_id}/rename` - 重命名文件夹
- `PUT /teams/{team_id}/folders/{folder_id}/move` - 移动文件夹

### 1.2 前端 - API 层

**文件：`web-admin/src/api/documents.ts`**
- 新增 `renameDocument(teamId, docId, newName)`
- 新增 `moveDocument(teamId, docId, targetFolderPath)`
- 新增 `copyDocument(teamId, docId, targetFolderPath)`
- 新增 `listTrash(teamId, page, limit)`
- 新增 `restoreDocument(teamId, docId)`
- 新增 `permanentDeleteDocument(teamId, docId)`
- 新增 `renameFolder(teamId, folderId, newName)`
- 新增 `moveFolder(teamId, folderId, newParentPath)`

### 1.3 前端 - Toast 通知系统

**新增文件：`web-admin/src/components/ui/toast.tsx`**
- 基于 @radix-ui/react-toast 实现
- 支持 success / error / info 三种类型

**新增文件：`web-admin/src/components/ui/toaster.tsx`**
- Toast 容器组件，挂载到 App 根节点

**新增文件：`web-admin/src/hooks/use-toast.ts`**
- `useToast()` hook，提供 `toast({ title, description, variant })` 方法

### 1.4 前端 - 右键菜单 & DropdownMenu

**新增文件：`web-admin/src/components/ui/dropdown-menu.tsx`**
- 基于 @radix-ui/react-dropdown-menu 实现

**新增文件：`web-admin/src/components/ui/context-menu.tsx`**
- 基于 @radix-ui/react-context-menu 实现

### 1.5 前端 - DocumentsTab 增强

**修改文件：`web-admin/src/components/team/DocumentsTab.tsx`**

新增功能：
- 拖拽上传区域（onDragOver/onDrop）
- 面包屑导航（替代简单的文件夹路径显示）
- 文件排序（名称/日期/大小/类型，升序/降序）
- 文件列表中显示锁定图标
- 右键菜单（重命名/移动/复制/下载/删除）
- 文件夹右键菜单（重命名/删除）
- 回收站入口和视图
- 文件夹删除按钮
- 所有操作使用 Toast 反馈替代 console.error

---

## 第二批：编辑体验增强

### 2.1 自动保存 & 离开确认

**修改文件：`web-admin/src/components/documents/DocumentEditor.tsx`**
- 添加 `beforeunload` 事件监听（未保存时阻止关闭）
- 添加 Ctrl+S / Cmd+S 快捷键保存
- 添加自动保存（防抖 30 秒，自动保存时 message 为 "Auto-save"）
- 添加锁续期定时器（每 20 分钟自动续期）
- 添加锁剩余时间倒计时显示

### 2.2 锁状态增强

**修改文件：`web-admin/src/components/team/DocumentsTab.tsx`**
- 文件列表项显示锁定图标和锁定者名称
- 点击被锁文件时显示友好提示（"xxx 正在编辑，请稍后再试"）
- 管理员可强制解锁

---

## 第三批：高级功能

### 3.1 批量操作

**修改文件：`web-admin/src/components/team/DocumentsTab.tsx`**
- 添加多选模式（checkbox）
- 批量删除、批量移动、批量下载

### 3.2 上传进度

**修改文件：`web-admin/src/components/team/DocumentsTab.tsx`**
- 使用 XMLHttpRequest 或 fetch + ReadableStream 获取上传进度
- 显示进度条 UI

### 3.3 文件标签管理

**修改文件：`web-admin/src/components/team/DocumentsTab.tsx`**
- 文件详情中显示标签
- 添加/删除标签的 UI

---

## i18n 翻译

**修改文件：`web-admin/src/i18n/locales/zh.ts` 和 `en.ts`**
- 新增所有新功能的翻译键（重命名、移动、复制、回收站、排序、批量操作等）

---

## NPM 依赖

需要安装：
- `@radix-ui/react-toast` - Toast 通知
- `@radix-ui/react-dropdown-menu` - 下拉菜单
- `@radix-ui/react-context-menu` - 右键菜单

---

## 实施顺序

1. 安装 NPM 依赖
2. 创建 Toast 组件 + hook
3. 创建 DropdownMenu / ContextMenu 组件
4. 后端：document_service 新增 rename/move/copy/trash/restore 方法
5. 后端：folder_service 新增 rename/move 方法
6. 后端：routes 新增路由
7. 前端：API 层新增方法
8. 前端：DocumentsTab 大改造（拖拽上传、面包屑、排序、右键菜单、回收站、锁状态、Toast）
9. 前端：DocumentEditor 增强（自动保存、离开确认、快捷键、锁续期）
10. 前端：批量操作、上传进度、标签管理
11. i18n 翻译补充
12. cargo check + tsc --noEmit 验证
