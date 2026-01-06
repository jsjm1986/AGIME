import { safeJsonParse } from './utils/conversionUtils';
import { Message } from './api';

export interface SharedSessionDetails {
  share_token: string;
  created_at: number;
  base_url: string;
  description: string;
  working_dir: string;
  messages: Message[];
  message_count: number;
  total_tokens: number | null;
}

export interface CreateShareRequest {
  sessionId: string;
  expiresInDays: number | null;
  password?: string;
}

export interface CreateShareResponse {
  shareToken: string;
  shareUrl: string | null;
  expiresAt: string | null;
  hasPassword: boolean;
}

export interface SharedSessionResponse {
  name: string;
  workingDir: string;
  messages: Message[];
  messageCount: number;
  totalTokens: number | null;
  createdAt: string;
  expiresAt: string | null;
}

export interface PasswordRequiredResponse {
  passwordRequired: boolean;
  name: string;
  messageCount: number;
}

/**
 * Fetches details for a specific shared session (legacy - external server)
 * @param baseUrl The base URL for session sharing API
 * @param shareToken The share token of the session to fetch
 * @returns Promise with shared session details
 */
export async function fetchSharedSessionDetails(
  baseUrl: string,
  shareToken: string
): Promise<SharedSessionDetails> {
  try {
    const response = await fetch(`${baseUrl}/sessions/share/${shareToken}`, {
      method: 'GET',
      headers: {
        'Content-Type': 'application/json',
      },
      credentials: 'include',
    });

    if (!response.ok) {
      throw new Error(`Failed to fetch shared session: ${response.status} ${response.statusText}`);
    }

    const data = await safeJsonParse<SharedSessionDetails>(
      response,
      'Failed to parse shared session'
    );

    if (baseUrl != data.base_url) {
      throw new Error(`Base URL mismatch for shared session: ${baseUrl} != ${data.base_url}`);
    }

    return {
      share_token: data.share_token,
      created_at: data.created_at,
      base_url: data.base_url,
      description: data.description,
      working_dir: data.working_dir,
      messages: data.messages,
      message_count: data.message_count,
      total_tokens: data.total_tokens,
    };
  } catch (error) {
    console.error('Error fetching shared session:', error);
    throw error;
  }
}

/**
 * Creates a new shared session (legacy - external server)
 * @param baseUrl The base URL for session sharing API
 * @param workingDir The working directory for the shared session
 * @param messages The messages to include in the shared session
 * @param description Description for the shared session
 * @param totalTokens Total token count for the session, or null if not available
 * @returns Promise with the share token
 */
export async function createSharedSession(
  baseUrl: string,
  workingDir: string,
  messages: Message[],
  description: string,
  totalTokens: number | null
): Promise<string> {
  try {
    const response = await fetch(`${baseUrl}/sessions/share`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        working_dir: workingDir,
        messages,
        description: description,
        base_url: baseUrl,
        total_tokens: totalTokens ?? null,
      }),
    });

    if (!response.ok) {
      if (response.status === 302) {
        throw new Error(
          `Failed to create shared session. Please check that you are connected to VPN - ${response.status} ${response.statusText}`
        );
      }
      throw new Error(`Failed to create shared session: ${response.status} ${response.statusText}`);
    }

    const data = await safeJsonParse<{ share_token: string }>(
      response,
      'Failed to parse shared session response'
    );
    return data.share_token;
  } catch (error) {
    console.error('Error creating shared session:', error);
    throw error;
  }
}

/**
 * Creates a new shared session via local API (tunnel mode)
 * @param baseUrl The local API base URL (e.g., http://localhost:3000)
 * @param sessionId The session ID to share
 * @param expiresInDays Days until expiration, or null for never
 * @param password Optional password for protection
 * @param secretKey Optional secret key for authentication
 * @returns Promise with share response
 */
export async function createLocalShare(
  baseUrl: string,
  sessionId: string,
  expiresInDays: number | null,
  password?: string,
  secretKey?: string
): Promise<CreateShareResponse> {
  try {
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
    };
    if (secretKey) {
      headers['X-Secret-Key'] = secretKey;
    }

    const response = await fetch(`${baseUrl}/sessions/share`, {
      method: 'POST',
      headers,
      body: JSON.stringify({
        sessionId,
        expiresInDays,
        password: password || undefined,
      }),
    });

    if (!response.ok) {
      const errorData = await response.json().catch(() => ({}));
      throw new Error(errorData.error || `Failed to create share: ${response.status}`);
    }

    return await response.json();
  } catch (error) {
    console.error('Error creating local share:', error);
    throw error;
  }
}

/**
 * Fetches a shared session via local API
 * @param baseUrl The local API base URL
 * @param shareToken The share token
 * @returns Promise with shared session response or password required info
 */
export async function fetchLocalSharedSession(
  baseUrl: string,
  shareToken: string
): Promise<SharedSessionResponse | PasswordRequiredResponse> {
  const response = await fetch(`${baseUrl}/sessions/share/${shareToken}`, {
    method: 'GET',
    headers: {
      'Content-Type': 'application/json',
    },
  });

  if (response.status === 401) {
    // Password required
    return await response.json();
  }

  if (!response.ok) {
    const errorData = await response.json().catch(() => ({}));
    throw new Error(errorData.error || `Failed to fetch share: ${response.status}`);
  }

  return await response.json();
}

/**
 * Verifies password and fetches shared session
 * @param baseUrl The local API base URL
 * @param shareToken The share token
 * @param password The password to verify
 * @returns Promise with shared session response
 */
export async function verifyAndFetchSharedSession(
  baseUrl: string,
  shareToken: string,
  password: string
): Promise<SharedSessionResponse> {
  const response = await fetch(`${baseUrl}/sessions/share/${shareToken}/verify`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ password }),
  });

  if (!response.ok) {
    const errorData = await response.json().catch(() => ({}));
    throw new Error(errorData.error || `Failed to verify: ${response.status}`);
  }

  return await response.json();
}
