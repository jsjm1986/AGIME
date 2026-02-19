// Document API client - matches MongoDB backend

import { fetchApi } from './client';

const API_BASE = '/api/team';

// Types matching backend DocumentSummary
export type DocumentOrigin = 'human' | 'agent';
export type DocumentStatusType = 'active' | 'draft' | 'accepted' | 'archived' | 'superseded';
export type DocumentCategory = 'general' | 'report' | 'translation' | 'summary' | 'review' | 'code' | 'other';

export interface SourceDocumentSnapshot {
  id: string;
  name: string;
  mime_type: string;
  origin: DocumentOrigin;
  category: DocumentCategory;
}

export interface DocumentSummary {
  id: string;
  name: string;
  display_name: string | null;
  description: string | null;
  mime_type: string;
  file_size: number;
  folder_path: string;
  tags: string[];
  uploaded_by: string;
  origin: DocumentOrigin;
  status: DocumentStatusType;
  category: DocumentCategory;
  source_document_ids: string[];
  source_snapshots?: SourceDocumentSnapshot[];
  source_session_id: string | null;
  source_mission_id: string | null;
  created_by_agent_id: string | null;
  supersedes_id: string | null;
  lineage_description: string | null;
  is_public?: boolean;
  created_at: string;
  updated_at?: string;
}

// Folder info from backend
export interface FolderInfo {
  id: string;
  name: string;
  parentPath: string;
  fullPath: string;
  description: string | null;
  createdBy: string;
  createdAt: string;
}

// Folder tree node from backend
export interface FolderTreeNode {
  id: string;
  name: string;
  fullPath: string;
  is_system?: boolean;
  children: FolderTreeNode[];
}

export interface ListDocumentsResponse {
  items: DocumentSummary[];
  total: number;
  page: number;
  limit: number;
  total_pages: number;
}

// Lock info from backend
export interface LockInfo {
  document_id: string;
  locked_by: string;
  locked_by_name: string;
  locked_at: string;
  expires_at: string;
}

// Version summary from backend
export interface VersionSummary {
  id: string;
  version_number: number;
  message: string;
  file_size: number;
  created_by: string;
  created_by_name: string;
  tag: string | null;
  created_at: string;
}

export interface ListVersionsResponse {
  items: VersionSummary[];
  total: number;
  page: number;
  limit: number;
  totalPages: number;
}

export interface TextContentResponse {
  text: string;
  mime_type: string;
}

function request<T>(path: string, options?: RequestInit): Promise<T> {
  return fetchApi<T>(`${API_BASE}${path}`, options);
}

// Folder API - backed by real server endpoints
export const folderApi = {
  async getFolderTree(teamId: string): Promise<FolderTreeNode[]> {
    return request<FolderTreeNode[]>(`/teams/${teamId}/folders/tree`);
  },

  async listFolders(teamId: string, parentPath?: string): Promise<FolderInfo[]> {
    const params = new URLSearchParams();
    if (parentPath) params.set('parent_path', parentPath);
    const query = params.toString() ? `?${params}` : '';
    return request<FolderInfo[]>(`/teams/${teamId}/folders${query}`);
  },

  async createFolder(teamId: string, req: { name: string; parentPath?: string; description?: string }): Promise<FolderInfo> {
    return request<FolderInfo>(`/teams/${teamId}/folders`, {
      method: 'POST',
      body: JSON.stringify({
        name: req.name,
        parent_path: req.parentPath || '/',
        description: req.description,
      }),
    });
  },

  async updateFolder(teamId: string, folderId: string, data: { name?: string; description?: string }): Promise<FolderInfo> {
    return request<FolderInfo>(`/teams/${teamId}/folders/${folderId}`, {
      method: 'PUT',
      body: JSON.stringify(data),
    });
  },

  async deleteFolder(teamId: string, folderId: string): Promise<void> {
    await request(`/teams/${teamId}/folders/${folderId}`, { method: 'DELETE' });
  },
};

// Document API
export const documentApi = {
  async listDocuments(
    teamId: string,
    page = 1,
    limit = 50,
    folderPath?: string
  ): Promise<ListDocumentsResponse> {
    const params = new URLSearchParams();
    params.set('page', String(page));
    params.set('limit', String(limit));
    if (folderPath) {
      params.set('folder_path', folderPath);
    }
    return request<ListDocumentsResponse>(`/teams/${teamId}/documents?${params}`);
  },

  async searchDocuments(
    teamId: string,
    query: string,
    page = 1,
    limit = 50,
    mimeType?: string,
    folderPath?: string,
  ): Promise<ListDocumentsResponse> {
    const params = new URLSearchParams();
    params.set('q', query);
    params.set('page', String(page));
    params.set('limit', String(limit));
    if (mimeType) params.set('mime_type', mimeType);
    if (folderPath) params.set('folder_path', folderPath);
    return request<ListDocumentsResponse>(`/teams/${teamId}/documents/search?${params}`);
  },

  async uploadDocument(
    teamId: string,
    file: File,
    folderPath?: string,
  ): Promise<DocumentSummary> {
    const formData = new FormData();
    formData.append('file', file);
    if (folderPath) {
      formData.append('folder_path', folderPath);
    }

    const res = await fetch(`${API_BASE}/teams/${teamId}/documents`, {
      method: 'POST',
      credentials: 'include',
      body: formData,
    });

    if (!res.ok) {
      const error = await res.json().catch(() => ({ error: 'Upload failed' }));
      throw new Error(error.error || 'Upload failed');
    }

    return res.json();
  },

  async deleteDocument(teamId: string, docId: string): Promise<void> {
    await request(`/teams/${teamId}/documents/${docId}`, { method: 'DELETE' });
  },

  async updateDocument(
    teamId: string,
    docId: string,
    data: { display_name?: string; description?: string; tags?: string[] },
  ): Promise<DocumentSummary> {
    return request<DocumentSummary>(`/teams/${teamId}/documents/${docId}`, {
      method: 'PUT',
      body: JSON.stringify(data),
    });
  },

  getDownloadUrl(teamId: string, docId: string): string {
    return `${API_BASE}/teams/${teamId}/documents/${docId}/download`;
  },

  // Phase 1: Content preview
  getContentUrl(teamId: string, docId: string): string {
    return `${API_BASE}/teams/${teamId}/documents/${docId}/content`;
  },

  async getTextContent(teamId: string, docId: string): Promise<TextContentResponse> {
    return request<TextContentResponse>(`/teams/${teamId}/documents/${docId}/content?format=text`);
  },

  // Phase 2: Edit & Lock
  async updateContent(teamId: string, docId: string, content: string, message?: string): Promise<DocumentSummary> {
    return request<DocumentSummary>(`/teams/${teamId}/documents/${docId}/content`, {
      method: 'PUT',
      body: JSON.stringify({ content, message }),
    });
  },

  async acquireLock(teamId: string, docId: string): Promise<LockInfo> {
    return request<LockInfo>(`/teams/${teamId}/documents/${docId}/lock`, { method: 'POST' });
  },

  async releaseLock(teamId: string, docId: string): Promise<void> {
    await request(`/teams/${teamId}/documents/${docId}/lock`, { method: 'DELETE' });
  },

  async getLockStatus(teamId: string, docId: string): Promise<LockInfo | null> {
    return request<LockInfo | null>(`/teams/${teamId}/documents/${docId}/lock`);
  },

  // Phase 3: Versions
  async listVersions(teamId: string, docId: string, page = 1, limit = 50): Promise<ListVersionsResponse> {
    const params = new URLSearchParams();
    params.set('page', String(page));
    params.set('limit', String(limit));
    return request<ListVersionsResponse>(`/teams/${teamId}/documents/${docId}/versions?${params}`);
  },

  async getVersionContent(teamId: string, docId: string, versionId: string): Promise<TextContentResponse> {
    return request<TextContentResponse>(`/teams/${teamId}/documents/${docId}/versions/${versionId}/content`);
  },

  async rollbackVersion(teamId: string, docId: string, versionId: string): Promise<VersionSummary> {
    return request<VersionSummary>(`/teams/${teamId}/documents/${docId}/versions/${versionId}/rollback`, {
      method: 'POST',
    });
  },

  async tagVersion(teamId: string, docId: string, versionId: string, tag: string): Promise<void> {
    await request(`/teams/${teamId}/documents/${docId}/versions/${versionId}/tag`, {
      method: 'PUT',
      body: JSON.stringify({ tag }),
    });
  },

  // Phase 2: Agent integration
  async listAiWorkbench(
    teamId: string,
    sessionId?: string,
    missionId?: string,
    page = 1,
    limit = 50,
  ): Promise<ListDocumentsResponse> {
    const params = new URLSearchParams();
    params.set('page', String(page));
    params.set('limit', String(limit));
    if (sessionId) params.set('session_id', sessionId);
    if (missionId) params.set('mission_id', missionId);
    return request<ListDocumentsResponse>(`/teams/${teamId}/documents/ai-workbench?${params}`);
  },

  async listByOrigin(
    teamId: string,
    origin: DocumentOrigin,
    page = 1,
    limit = 50,
  ): Promise<ListDocumentsResponse> {
    const params = new URLSearchParams();
    params.set('origin', origin);
    params.set('page', String(page));
    params.set('limit', String(limit));
    return request<ListDocumentsResponse>(`/teams/${teamId}/documents/by-origin?${params}`);
  },

  async updateStatus(teamId: string, docId: string, status: DocumentStatusType): Promise<void> {
    await request(`/teams/${teamId}/documents/${docId}/status`, {
      method: 'PUT',
      body: JSON.stringify({ status }),
    });
  },

  async getLineage(teamId: string, docId: string): Promise<DocumentSummary[]> {
    return request<DocumentSummary[]>(`/teams/${teamId}/documents/${docId}/lineage`);
  },

  async listDerived(teamId: string, docId: string, page = 1, limit = 50): Promise<ListDocumentsResponse> {
    const params = new URLSearchParams();
    params.set('page', String(page));
    params.set('limit', String(limit));
    return request<ListDocumentsResponse>(`/teams/${teamId}/documents/${docId}/derived?${params}`);
  },

  async listArchived(teamId: string, page = 1, limit = 50): Promise<ListDocumentsResponse> {
    const params = new URLSearchParams();
    params.set('page', String(page));
    params.set('limit', String(limit));
    return request<ListDocumentsResponse>(`/teams/${teamId}/documents/archived?${params}`);
  },
};

// Helper functions
export function formatFileSize(bytes: number): string {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
}
