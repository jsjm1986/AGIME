/**
 * Chat API client (Phase 1 - Chat Track)
 * Direct session-based chat that bypasses the Task system.
 */

import { fetchApi } from './client';

const API_BASE = '/api/team/agent/chat';

export interface ChatSession {
  session_id: string;
  agent_id: string;
  agent_name: string;
  title?: string;
  last_message_preview?: string;
  last_message_at?: string;
  message_count: number;
  status: 'active' | 'archived';
  pinned: boolean;
  created_at: string;
}

export interface ChatSessionDetail {
  session_id: string;
  agent_id: string;
  user_id: string;
  title?: string;
  messages_json: string;
  message_count: number;
  total_tokens?: number;
  disabled_extensions: string[];
  enabled_extensions: string[];
  status: string;
  pinned: boolean;
  is_processing: boolean;
  workspace_path?: string | null;
  extra_instructions?: string | null;
  allowed_extensions?: string[] | null;
  allowed_skill_ids?: string[] | null;
  portal_restricted?: boolean;
  portal_id?: string | null;
  portal_slug?: string | null;
  visitor_id?: string | null;
  created_at: string;
  updated_at: string;
}

export interface SendMessageResponse {
  session_id: string;
  streaming: boolean;
}

export interface CreateSessionOptions {
  extraInstructions?: string;
  allowedExtensions?: string[];
  allowedSkillIds?: string[];
  portalRestricted?: boolean;
}

export interface CreatePortalCodingSessionResponse {
  session_id: string;
  agent_id: string;
  status: string;
  portal_restricted: boolean;
  workspace_path: string;
  allowed_extensions: string[];
}

export const chatApi = {
  /** List user's chat sessions */
  async listSessions(
    teamId: string,
    agentId?: string,
    page = 1,
    limit = 20,
    status?: string,
  ): Promise<ChatSession[]> {
    const params = new URLSearchParams({ team_id: teamId, page: String(page), limit: String(limit) });
    if (agentId) params.set('agent_id', agentId);
    if (status) params.set('status', status);
    return fetchApi(`${API_BASE}/sessions?${params}`);
  },

  /** Create a new chat session */
  async createSession(
    agentId: string,
    attachedDocumentIds?: string[],
    options?: CreateSessionOptions,
  ): Promise<{ session_id: string; agent_id: string; status: string }> {
    return fetchApi(`${API_BASE}/sessions`, {
      method: 'POST',
      body: JSON.stringify({
        agent_id: agentId,
        attached_document_ids: attachedDocumentIds || [],
        extra_instructions: options?.extraInstructions,
        allowed_extensions: options?.allowedExtensions,
        allowed_skill_ids: options?.allowedSkillIds,
        portal_restricted: options?.portalRestricted ?? false,
      }),
    });
  },

  /** Create a portal laboratory coding session with strict runtime policy */
  async createPortalCodingSession(
    teamId: string,
    portalId: string,
  ): Promise<CreatePortalCodingSessionResponse> {
    return fetchApi(`${API_BASE}/sessions/portal-coding`, {
      method: 'POST',
      body: JSON.stringify({
        team_id: teamId,
        portal_id: portalId,
      }),
    });
  },

  /** Get session details with messages */
  async getSession(sessionId: string): Promise<ChatSessionDetail> {
    return fetchApi(`${API_BASE}/sessions/${sessionId}`);
  },

  /** Rename a session */
  async renameSession(sessionId: string, title: string): Promise<void> {
    await fetchApi(`${API_BASE}/sessions/${sessionId}`, {
      method: 'PUT',
      body: JSON.stringify({ title }),
    });
  },

  /** Pin/unpin a session */
  async pinSession(sessionId: string, pinned: boolean): Promise<void> {
    await fetchApi(`${API_BASE}/sessions/${sessionId}`, {
      method: 'PUT',
      body: JSON.stringify({ pinned }),
    });
  },

  /** Send a message (triggers execution) */
  async sendMessage(sessionId: string, content: string): Promise<SendMessageResponse> {
    return fetchApi(`${API_BASE}/sessions/${sessionId}/messages`, {
      method: 'POST',
      body: JSON.stringify({ content }),
    });
  },

  /** Subscribe to SSE stream for a session */
  streamChat(sessionId: string, lastEventId?: number | null): EventSource {
    const q = lastEventId && lastEventId > 0 ? `?last_event_id=${lastEventId}` : '';
    return new EventSource(`${API_BASE}/sessions/${sessionId}/stream${q}`, {
      withCredentials: true,
    } as EventSourceInit);
  },

  /** Cancel active chat */
  async cancelChat(sessionId: string): Promise<void> {
    await fetchApi(`${API_BASE}/sessions/${sessionId}/cancel`, { method: 'POST' });
  },

  /** Archive a session */
  async archiveSession(sessionId: string): Promise<void> {
    await fetchApi(`${API_BASE}/sessions/${sessionId}/archive`, { method: 'POST' });
  },

  /** Delete a session permanently */
  async deleteSession(sessionId: string): Promise<void> {
    await fetchApi(`${API_BASE}/sessions/${sessionId}`, { method: 'DELETE' });
  },

  /** Attach documents to a session */
  async attachDocuments(sessionId: string, documentIds: string[]): Promise<void> {
    await fetchApi(`${API_BASE}/sessions/${sessionId}/documents`, {
      method: 'POST',
      body: JSON.stringify({ document_ids: documentIds }),
    });
  },

  /** Detach documents from a session */
  async detachDocuments(sessionId: string, documentIds: string[]): Promise<void> {
    await fetchApi(`${API_BASE}/sessions/${sessionId}/documents`, {
      method: 'DELETE',
      body: JSON.stringify({ document_ids: documentIds }),
    });
  },

  /** List attached documents */
  async listAttachedDocuments(sessionId: string): Promise<string[]> {
    return fetchApi(`${API_BASE}/sessions/${sessionId}/documents`);
  },
};
