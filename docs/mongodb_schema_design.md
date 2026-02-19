// MongoDB 数据模型设计方案
//
// 集合设计：
// - teams: 团队信息
// - documents: 文档元数据 + GridFS 文件引用
// - folders: 文件夹（可选，也可嵌入 teams）

// ==========================================
// teams 集合
// ==========================================
{
  "_id": ObjectId,
  "name": "团队名称",
  "description": "描述",
  "owner_id": "用户ID",
  "members": [
    {
      "user_id": "用户ID",
      "role": "owner|admin|member",
      "joined_at": ISODate
    }
  ],
  "settings": {
    "allow_public_docs": false
  },
  "created_at": ISODate,
  "updated_at": ISODate
}

// ==========================================
// documents 集合
// ==========================================
{
  "_id": ObjectId,
  "team_id": ObjectId,
  "folder_path": "/path/to/folder",  // 使用路径而非外键
  "name": "文件名.pdf",
  "display_name": "显示名称",
  "description": "文件描述",
  "mime_type": "application/pdf",
  "file_size": 1024000,
  "grid_fs_id": ObjectId,  // GridFS 文件引用
  "tags": ["tag1", "tag2"],
  "metadata": {
    // 灵活的元数据，不同文件类型可有不同字段
    "author": "作者",
    "pages": 10
  },
  "uploaded_by": "用户ID",
  "created_at": ISODate,
  "updated_at": ISODate
}

// ==========================================
// skills 集合
// ==========================================
{
  "_id": ObjectId,
  "team_id": ObjectId,
  "name": "技能名称",
  "description": "描述",
  "content": "技能内容...",
  "version": "1.0.0",
  "tags": ["tag1"],
  "created_by": "用户ID",
  "created_at": ISODate,
  "updated_at": ISODate
}

// ==========================================
// recipes 集合
// ==========================================
{
  "_id": ObjectId,
  "team_id": ObjectId,
  "name": "配方名称",
  "description": "描述",
  "content_yaml": "yaml内容...",
  "category": "分类",
  "tags": ["tag1"],
  "created_by": "用户ID",
  "created_at": ISODate,
  "updated_at": ISODate
}

// ==========================================
// 索引设计
// ==========================================
// teams:
//   - { "owner_id": 1 }
//   - { "members.user_id": 1 }
//
// documents:
//   - { "team_id": 1, "folder_path": 1 }
//   - { "team_id": 1, "name": "text" }  // 全文搜索
//   - { "tags": 1 }
//
// skills/recipes:
//   - { "team_id": 1 }
//   - { "name": "text", "description": "text" }
