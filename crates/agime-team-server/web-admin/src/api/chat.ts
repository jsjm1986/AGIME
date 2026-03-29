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
  input_tokens?: number;
  output_tokens?: number;
  compaction_count?: number;
  disabled_extensions: string[];
  enabled_extensions: string[];
  status: string;
  pinned: boolean;
  is_processing: boolean;
  last_execution_status?: 'running' | 'completed' | 'failed' | string | null;
  last_execution_error?: string | null;
  last_execution_finished_at?: string | null;
  workspace_path?: string | null;
  extra_instructions?: string | null;
  allowed_extensions?: string[] | null;
  allowed_skill_ids?: string[] | null;
  retry_config?: unknown | null;
  max_turns?: number | null;
  tool_timeout_seconds?: number | null;
  max_portal_retry_rounds?: number | null;
  require_final_report?: boolean;
  portal_restricted?: boolean;
  portal_id?: string | null;
  portal_slug?: string | null;
  visitor_id?: string | null;
  session_source?: 'chat' | 'mission' | 'portal' | 'system' | string;
  source_mission_id?: string | null;
  hidden_from_chat_list?: boolean;
  created_at: string;
  updated_at: string;
}

export interface SendMessageResponse {
  session_id: string;
  streaming: boolean;
}

export interface ChatSessionEvent {
  _id?: { $oid?: string };
  session_id: string;
  run_id?: string | null;
  event_id: number;
  event_type: string;
  payload: Record<string, unknown>;
  created_at: string;
}

export interface ComposerCapabilitySkill {
  id: string;
  name: string;
  version: string;
  description?: string | null;
  summary_text?: string | null;
  detail_text?: string | null;
  detail_lang?: string | null;
  detail_source?: 'ai_description' | 'raw_description' | 'builtin_cache' | string | null;
  skill_ref: string;
  display_line_zh: string;
  plain_line_zh: string;
}

export interface ComposerCapabilityExtension {
  runtime_name: string;
  display_name: string;
  class: 'builtin' | 'custom' | 'team' | string;
  type?: 'stdio' | 'sse' | 'streamable_http' | string | null;
  description?: string | null;
  summary_text?: string | null;
  detail_text?: string | null;
  detail_lang?: string | null;
  detail_source?: 'ai_description' | 'raw_description' | 'builtin_cache' | string | null;
  ext_ref: string;
  display_line_zh: string;
  plain_line_zh: string;
}

export interface ComposerHiddenCapabilityExtension {
  runtime_name: string;
  display_name: string;
  reason: 'skill_runtime' | 'system_assist' | 'legacy_hidden' | string;
}

export interface ComposerCapabilitiesCatalog {
  skills: ComposerCapabilitySkill[];
  extensions: ComposerCapabilityExtension[];
  hidden_extensions?: ComposerHiddenCapabilityExtension[];
}

export interface CreateSessionOptions {
  extraInstructions?: string;
  allowedExtensions?: string[];
  allowedSkillIds?: string[];
  portalRestricted?: boolean;
}

export type ChatChannelVisibility = 'team_public' | 'team_private';
export type ChatChannelMemberRole = 'owner' | 'manager' | 'member';
export type ChatChannelAuthorType = 'user' | 'agent' | 'system';

export interface ChatChannelSummary {
  channel_id: string;
  team_id: string;
  name: string;
  description?: string | null;
  visibility: ChatChannelVisibility;
  default_agent_id: string;
  default_agent_name: string;
  document_folder_path?: string | null;
  created_by_user_id: string;
  status: 'active' | 'archived';
  last_message_preview?: string | null;
  last_message_at?: string | null;
  message_count: number;
  thread_count: number;
  unread_count: number;
  member_count: number;
  is_processing: boolean;
  active_run_id?: string | null;
  last_run_status?: string | null;
  last_run_error?: string | null;
  last_run_finished_at?: string | null;
  created_at: string;
  updated_at: string;
}

export interface ChatChannelDetail {
  channel_id: string;
  team_id: string;
  name: string;
  description?: string | null;
  visibility: ChatChannelVisibility;
  default_agent_id: string;
  default_agent_name: string;
  document_folder_path?: string | null;
  created_by_user_id: string;
  status: 'active' | 'archived';
  last_message_preview?: string | null;
  last_message_at?: string | null;
  message_count: number;
  thread_count: number;
  unread_count: number;
  member_count: number;
  is_processing: boolean;
  active_run_id?: string | null;
  last_run_status?: string | null;
  last_run_error?: string | null;
  last_run_finished_at?: string | null;
  created_at: string;
  updated_at: string;
  current_user_role: ChatChannelMemberRole;
}

export interface ChatChannelMember {
  channel_id: string;
  team_id: string;
  user_id: string;
  role: ChatChannelMemberRole;
  joined_at: string;
}

export interface ChatChannelMessage {
  message_id: string;
  channel_id: string;
  team_id: string;
  thread_root_id?: string | null;
  parent_message_id?: string | null;
  author_type: ChatChannelAuthorType;
  author_user_id?: string | null;
  author_agent_id?: string | null;
  author_name: string;
  agent_id?: string | null;
  content_text: string;
  content_blocks: unknown;
  metadata: Record<string, unknown>;
  visible: boolean;
  created_at: string;
  updated_at: string;
  reply_count: number;
}

export interface ChatChannelThread {
  root_message: ChatChannelMessage;
  messages: ChatChannelMessage[];
}

export interface ChatChannelRead {
  channel_id: string;
  user_id: string;
  last_read_message_id?: string | null;
  last_read_at: string;
}

export interface SendChannelMessageResponse {
  channel_id: string;
  message_id: string;
  thread_root_id?: string | null;
  root_message_id: string;
  run_id: string;
  streaming: boolean;
}

export interface ChannelMention {
  mention_type: string;
  target_id: string;
  label?: string | null;
}

export interface UserChatMemory {
  team_id: string;
  user_id: string;
  preferred_address?: string | null;
  role_hint?: string | null;
  current_focus?: string | null;
  collaboration_preference?: string | null;
  notes?: string | null;
  updated_at: string;
  updated_by_user_id: string;
}

export interface UserChatMemorySuggestion {
  suggestion_id: string;
  team_id: string;
  user_id: string;
  session_id: string;
  source_message: string;
  proposed_patch: {
    preferred_address?: string | null;
    role_hint?: string | null;
    current_focus?: string | null;
    collaboration_preference?: string | null;
    notes?: string | null;
  };
  reason: string;
  status: 'pending' | 'accepted' | 'dismissed' | 'expired';
  created_at: string;
  resolved_at?: string | null;
}

export interface CreatePortalCodingSessionResponse {
  session_id: string;
  agent_id: string;
  status: string;
  portal_restricted: boolean;
  workspace_path: string;
  allowed_extensions: string[] | null;
  retry_config?: unknown;
  max_turns?: number | null;
  tool_timeout_seconds?: number | null;
  max_portal_retry_rounds?: number | null;
  require_final_report?: boolean;
}

export interface CreatePortalManagerSessionResponse {
  session_id: string;
  agent_id: string;
  status: string;
  portal_restricted: boolean;
  allowed_extensions: string[] | null;
  retry_config?: unknown;
  max_turns?: number | null;
  tool_timeout_seconds?: number | null;
  max_portal_retry_rounds?: number | null;
  require_final_report?: boolean;
}

export const chatApi = {
  /** List user's chat sessions */
  async listSessions(
    teamId: string,
    agentId?: string,
    page = 1,
    limit = 20,
    status?: string,
    includeHidden = false,
  ): Promise<ChatSession[]> {
    const params = new URLSearchParams({ team_id: teamId, page: String(page), limit: String(limit) });
    if (agentId) params.set('agent_id', agentId);
    if (status) params.set('status', status);
    if (includeHidden) params.set('include_hidden', 'true');
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

/** Create a portal ecosystem coding session */
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

  /** Create a team-level portal manager session (no existing portal required) */
  async createPortalManagerSession(
    teamId: string,
    managerAgentId?: string,
    portalId?: string,
  ): Promise<CreatePortalManagerSessionResponse> {
    return fetchApi(`${API_BASE}/sessions/portal-manager`, {
      method: 'POST',
      body: JSON.stringify({
        team_id: teamId,
        manager_agent_id: managerAgentId || undefined,
        portal_id: portalId || undefined,
      }),
    });
  },

  /** Get session details with messages */
  async getSession(sessionId: string): Promise<ChatSessionDetail> {
    return fetchApi(`${API_BASE}/sessions/${sessionId}`);
  },

  /** Get resolved skills / extensions visible for a new chat with a specific agent */
  async getAgentComposerCapabilities(agentId: string): Promise<ComposerCapabilitiesCatalog> {
    return fetchApi(`${API_BASE}/agents/${agentId}/composer-capabilities`);
  },

  /** Get resolved skills / extensions visible for an existing chat session */
  async getSessionComposerCapabilities(sessionId: string): Promise<ComposerCapabilitiesCatalog> {
    return fetchApi(`${API_BASE}/sessions/${sessionId}/composer-capabilities`);
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

  /** List persisted runtime events for a session */
  async listSessionEvents(
    sessionId: string,
    options?: {
      runId?: string;
      afterEventId?: number;
      beforeEventId?: number;
      order?: 'asc' | 'desc';
      limit?: number;
    }
  ): Promise<ChatSessionEvent[]> {
    const params = new URLSearchParams();
    if (options?.runId && options.runId.trim()) params.set('run_id', options.runId.trim());
    if (typeof options?.afterEventId === 'number' && Number.isFinite(options.afterEventId)) {
      params.set('after_event_id', String(Math.max(0, Math.floor(options.afterEventId))));
    }
    if (typeof options?.beforeEventId === 'number' && Number.isFinite(options.beforeEventId)) {
      params.set('before_event_id', String(Math.max(0, Math.floor(options.beforeEventId))));
    }
    if (options?.order === 'desc' || options?.order === 'asc') {
      params.set('order', options.order);
    }
    params.set('limit', String(options?.limit ? Math.min(Math.max(options.limit, 1), 2000) : 500));
    return fetchApi(`${API_BASE}/sessions/${sessionId}/events?${params.toString()}`);
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

  async getMyMemory(teamId: string): Promise<UserChatMemory | null> {
    const res = await fetchApi<{ memory?: UserChatMemory | null }>(
      `${API_BASE}/memory/me?team_id=${encodeURIComponent(teamId)}`,
    );
    return res.memory || null;
  },

  async updateMyMemory(
    teamId: string,
    payload: {
      preferred_address?: string | null;
      role_hint?: string | null;
      current_focus?: string | null;
      collaboration_preference?: string | null;
      notes?: string | null;
      session_id?: string | null;
    },
  ): Promise<UserChatMemory | null> {
    const res = await fetchApi<{ memory?: UserChatMemory | null }>(
      `${API_BASE}/memory/me?team_id=${encodeURIComponent(teamId)}`,
      {
        method: 'PATCH',
        body: JSON.stringify(payload),
      },
    );
    return res.memory || null;
  },

  async listMemorySuggestions(
    teamId: string,
    sessionId?: string | null,
  ): Promise<UserChatMemorySuggestion[]> {
    const params = new URLSearchParams({ team_id: teamId });
    if (sessionId) params.set('session_id', sessionId);
    const res = await fetchApi<{ suggestions?: UserChatMemorySuggestion[] }>(
      `${API_BASE}/memory/suggestions?${params.toString()}`,
    );
    return res.suggestions || [];
  },

  async acceptMemorySuggestion(
    suggestionId: string,
  ): Promise<{ suggestion: UserChatMemorySuggestion; memory?: UserChatMemory | null }> {
    return fetchApi(`${API_BASE}/memory/suggestions/${suggestionId}/accept`, {
      method: 'POST',
    });
  },

  async dismissMemorySuggestion(
    suggestionId: string,
  ): Promise<{ suggestion: UserChatMemorySuggestion }> {
    return fetchApi(`${API_BASE}/memory/suggestions/${suggestionId}/dismiss`, {
      method: 'POST',
    });
  },

  async listChannels(teamId: string): Promise<ChatChannelSummary[]> {
    const res = await fetchApi<{ channels?: ChatChannelSummary[] }>(
      `${API_BASE}/channels?team_id=${encodeURIComponent(teamId)}`,
    );
    return res.channels || [];
  },

  async createChannel(
    teamId: string,
    payload: {
      name: string;
      description?: string | null;
      visibility?: ChatChannelVisibility;
      default_agent_id: string;
      member_user_ids?: string[];
    },
  ): Promise<ChatChannelDetail> {
    const res = await fetchApi<{ channel: ChatChannelDetail }>(
      `${API_BASE}/channels?team_id=${encodeURIComponent(teamId)}`,
      {
        method: 'POST',
        body: JSON.stringify(payload),
      },
    );
    return res.channel;
  },

  async getChannel(channelId: string): Promise<ChatChannelDetail> {
    const res = await fetchApi<{ channel: ChatChannelDetail }>(
      `${API_BASE}/channels/${channelId}`,
    );
    return res.channel;
  },

  async updateChannel(
    channelId: string,
    payload: {
      name?: string;
      description?: string | null;
      visibility?: ChatChannelVisibility;
      default_agent_id?: string;
    },
  ): Promise<ChatChannelDetail> {
    const res = await fetchApi<{ channel: ChatChannelDetail }>(
      `${API_BASE}/channels/${channelId}`,
      {
        method: 'PATCH',
        body: JSON.stringify(payload),
      },
    );
    return res.channel;
  },

  async archiveChannel(channelId: string): Promise<void> {
    await fetchApi(`${API_BASE}/channels/${channelId}/archive`, {
      method: 'POST',
    });
  },

  async deleteChannel(channelId: string): Promise<void> {
    await fetchApi(`${API_BASE}/channels/${channelId}`, { method: 'DELETE' });
  },

  async listChannelMembers(channelId: string): Promise<ChatChannelMember[]> {
    const res = await fetchApi<{ members?: ChatChannelMember[] }>(
      `${API_BASE}/channels/${channelId}/members`,
    );
    return res.members || [];
  },

  async addChannelMember(
    channelId: string,
    payload: { user_id: string; role?: ChatChannelMemberRole },
  ): Promise<ChatChannelMember[]> {
    const res = await fetchApi<{ members?: ChatChannelMember[] }>(
      `${API_BASE}/channels/${channelId}/members`,
      {
        method: 'POST',
        body: JSON.stringify(payload),
      },
    );
    return res.members || [];
  },

  async updateChannelMember(
    channelId: string,
    userId: string,
    payload: { role: ChatChannelMemberRole },
  ): Promise<ChatChannelMember[]> {
    const res = await fetchApi<{ members?: ChatChannelMember[] }>(
      `${API_BASE}/channels/${channelId}/members/${userId}`,
      {
        method: 'PATCH',
        body: JSON.stringify(payload),
      },
    );
    return res.members || [];
  },

  async removeChannelMember(channelId: string, userId: string): Promise<ChatChannelMember[]> {
    const res = await fetchApi<{ members?: ChatChannelMember[] }>(
      `${API_BASE}/channels/${channelId}/members/${userId}`,
      { method: 'DELETE' },
    );
    return res.members || [];
  },

  async listChannelMessages(channelId: string): Promise<ChatChannelMessage[]> {
    const res = await fetchApi<{ messages?: ChatChannelMessage[] }>(
      `${API_BASE}/channels/${channelId}/messages`,
    );
    return res.messages || [];
  },

  async getChannelThread(channelId: string, threadRootId: string): Promise<ChatChannelThread> {
    return fetchApi(`${API_BASE}/channels/${channelId}/threads/${threadRootId}`);
  },

  async sendChannelMessage(
    channelId: string,
    payload: {
      content: string;
      agent_id?: string | null;
      thread_root_id?: string | null;
      parent_message_id?: string | null;
      mentions?: ChannelMention[];
    },
  ): Promise<SendChannelMessageResponse> {
    return fetchApi(`${API_BASE}/channels/${channelId}/messages`, {
      method: 'POST',
      body: JSON.stringify(payload),
    });
  },

  async sendChannelThreadMessage(
    channelId: string,
    threadRootId: string,
    payload: {
      content: string;
      agent_id?: string | null;
      parent_message_id?: string | null;
      mentions?: ChannelMention[];
    },
  ): Promise<SendChannelMessageResponse> {
    return fetchApi(`${API_BASE}/channels/${channelId}/threads/${threadRootId}/messages`, {
      method: 'POST',
      body: JSON.stringify(payload),
    });
  },

  streamChannel(channelId: string, lastEventId?: number | null): EventSource {
    const q = lastEventId && lastEventId > 0 ? `?last_event_id=${lastEventId}` : '';
    return new EventSource(`${API_BASE}/channels/${channelId}/stream${q}`, {
      withCredentials: true,
    } as EventSourceInit);
  },

  async listChannelEvents(
    channelId: string,
    options?: {
      runId?: string;
      order?: 'asc' | 'desc';
      limit?: number;
    },
  ): Promise<Record<string, unknown>[]> {
    const params = new URLSearchParams();
    if (options?.runId && options.runId.trim()) params.set('run_id', options.runId.trim());
    if (options?.order === 'asc' || options?.order === 'desc') params.set('order', options.order);
    params.set('limit', String(options?.limit ? Math.min(Math.max(options.limit, 1), 2000) : 500));
    return fetchApi(`${API_BASE}/channels/${channelId}/events?${params.toString()}`);
  },

  async markChannelRead(
    channelId: string,
    payload?: { last_read_message_id?: string | null },
  ): Promise<ChatChannelRead> {
    return fetchApi(`${API_BASE}/channels/${channelId}/read`, {
      method: 'POST',
      body: JSON.stringify(payload || {}),
    });
  },
};
