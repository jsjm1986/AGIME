// Unified Authentication Adapter
// Handles authentication for different data source types

import type { DataSource, AuthType } from '../sources/types';
import { platform } from '../../../platform';

// ============================================================
// Auth Verification Types
// ============================================================

/** Auth verification result */
export interface AuthVerifyResult {
  success: boolean;
  userId?: string;
  email?: string;
  displayName?: string;
  error?: string;
}

/** Auth refresh result */
export interface AuthRefreshResult {
  success: boolean;
  newCredential?: string;
  expiresAt?: string;
  error?: string;
}

// ============================================================
// Credential Storage
// ============================================================

/** Secure credential storage keys */
const CREDENTIAL_STORAGE_PREFIX = 'AGIME_CRED_';

/** Get credential from secure storage */
function getStoredCredential(credentialRef: string): string | null {
  try {
    return localStorage.getItem(`${CREDENTIAL_STORAGE_PREFIX}${credentialRef}`);
  } catch {
    return null;
  }
}

/** Store credential securely */
export function storeCredential(credentialRef: string, value: string): void {
  try {
    localStorage.setItem(`${CREDENTIAL_STORAGE_PREFIX}${credentialRef}`, value);
  } catch {
    console.error('Failed to store credential');
  }
}

/** Remove credential from storage */
export function removeCredential(credentialRef: string): void {
  try {
    localStorage.removeItem(`${CREDENTIAL_STORAGE_PREFIX}${credentialRef}`);
  } catch {
    // Ignore errors
  }
}

// ============================================================
// Unified Auth Adapter Class
// ============================================================

export class UnifiedAuthAdapter {
  /**
   * Get authentication headers for a data source
   */
  async getAuthHeaders(source: DataSource): Promise<Record<string, string>> {
    const headers: Record<string, string> = {};

    if (source.type === 'local') {
      // Local source uses platform secret key
      const secretKey = await platform.getSecretKey();
      if (secretKey) {
        headers['X-Secret-Key'] = secretKey;
      }
      return headers;
    }

    // Remote sources use stored credentials
    const credential = getStoredCredential(source.connection.credentialRef);
    if (!credential) {
      throw new Error(`No credential found for source: ${source.name}`);
    }

    switch (source.connection.authType) {
      case 'secret-key':
        headers['X-Secret-Key'] = credential;
        break;
      case 'api-key':
        headers['X-API-Key'] = credential;
        break;
    }

    return headers;
  }

  /**
   * Get the appropriate auth header name for a source type
   */
  getAuthHeaderName(authType: AuthType): string {
    switch (authType) {
      case 'secret-key':
        return 'X-Secret-Key';
      case 'api-key':
        return 'X-API-Key';
    }
  }

  /**
   * Verify authentication for a data source
   */
  async verifyAuth(source: DataSource): Promise<AuthVerifyResult> {
    try {
      const headers = await this.getAuthHeaders(source);
      const url = `${source.connection.url}/api/team/teams?page=1&limit=1`;

      const response = await fetch(url, {
        method: 'GET',
        headers: {
          'Content-Type': 'application/json',
          ...headers,
        },
        signal: AbortSignal.timeout(10000),
      });

      if (response.ok) {
        return {
          success: true,
          userId: source.userInfo?.userId,
          email: source.userInfo?.email,
          displayName: source.userInfo?.displayName,
        };
      }

      if (response.status === 401) {
        return {
          success: false,
          error: 'Authentication failed: Invalid credentials',
        };
      }

      return {
        success: false,
        error: `Authentication failed: HTTP ${response.status}`,
      };
    } catch (error) {
      return {
        success: false,
        error: error instanceof Error ? error.message : 'Authentication failed',
      };
    }
  }

  /**
   * Refresh authentication (for sources that support token refresh)
   */
  async refreshAuth(source: DataSource): Promise<AuthRefreshResult> {
    // Currently, API keys and secret keys don't expire
    // This is a placeholder for future OAuth/JWT support
    const verifyResult = await this.verifyAuth(source);

    if (verifyResult.success) {
      return { success: true };
    }

    return {
      success: false,
      error: verifyResult.error,
    };
  }

  /**
   * Test connection with provided credentials (before saving)
   */
  async testConnection(
    url: string,
    authType: AuthType,
    credential: string
  ): Promise<AuthVerifyResult & { teamsCount?: number; version?: string }> {
    try {
      const headerName = this.getAuthHeaderName(authType);
      const normalizedUrl = url.replace(/\/+$/, '');

      // First, try health check endpoint
      const healthResponse = await fetch(`${normalizedUrl}/health`, {
        method: 'GET',
        signal: AbortSignal.timeout(10000),
      });

      let version: string | undefined;
      if (healthResponse.ok) {
        const healthData = await healthResponse.json();
        version = healthData.version;
      }

      // Then verify auth with teams endpoint
      const teamsResponse = await fetch(`${normalizedUrl}/api/team/teams?page=1&limit=1`, {
        method: 'GET',
        headers: {
          'Content-Type': 'application/json',
          [headerName]: credential,
        },
        signal: AbortSignal.timeout(10000),
      });

      if (!teamsResponse.ok) {
        if (teamsResponse.status === 401) {
          return {
            success: false,
            error: 'Invalid credentials',
          };
        }
        return {
          success: false,
          error: `Connection failed: HTTP ${teamsResponse.status}`,
        };
      }

      const teamsData = await teamsResponse.json();

      return {
        success: true,
        teamsCount: teamsData.total || 0,
        version,
      };
    } catch (error) {
      return {
        success: false,
        error: error instanceof Error ? error.message : 'Connection failed',
      };
    }
  }
}

// Singleton instance
export const authAdapter = new UnifiedAuthAdapter();
