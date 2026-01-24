const API_BASE = '/api';

export interface User {
  id: string;
  email: string;
  display_name: string;
}

export interface ApiKey {
  id: string;
  key_prefix: string;
  name: string | null;
  last_used_at: string | null;
  expires_at: string | null;
  created_at: string;
}

export interface RegisterResponse {
  user: User;
  api_key: string;
  message: string;
}

export interface SessionResponse {
  user: User;
}

export interface ApiKeysResponse {
  keys: ApiKey[];
}

export interface CreateApiKeyResponse {
  key: {
    id: string;
    api_key: string;
    name: string | null;
    expires_at: string | null;
  };
  message: string;
}

class ApiClient {
  private async request<T>(path: string, options?: RequestInit): Promise<T> {
    const res = await fetch(`${API_BASE}${path}`, {
      ...options,
      credentials: 'include',
      headers: {
        'Content-Type': 'application/json',
        ...options?.headers,
      },
    });

    if (!res.ok) {
      const error = await res.json().catch(() => ({ error: 'Request failed' }));
      throw new Error(error.error || 'Request failed');
    }

    return res.json();
  }

  async register(email: string, displayName: string): Promise<RegisterResponse> {
    return this.request('/auth/register', {
      method: 'POST',
      body: JSON.stringify({ email, display_name: displayName }),
    });
  }

  async login(apiKey: string): Promise<SessionResponse> {
    return this.request('/auth/login', {
      method: 'POST',
      body: JSON.stringify({ api_key: apiKey }),
    });
  }

  async logout(): Promise<void> {
    await this.request('/auth/logout', { method: 'POST' });
  }

  async getSession(): Promise<SessionResponse> {
    return this.request('/auth/session');
  }

  async getApiKeys(): Promise<ApiKeysResponse> {
    return this.request('/auth/keys');
  }

  async createApiKey(name?: string, expiresInDays?: number): Promise<CreateApiKeyResponse> {
    return this.request('/auth/keys', {
      method: 'POST',
      body: JSON.stringify({ name, expires_in_days: expiresInDays }),
    });
  }

  async revokeApiKey(keyId: string): Promise<void> {
    await this.request(`/auth/keys/${keyId}`, { method: 'DELETE' });
  }
}

export const apiClient = new ApiClient();
