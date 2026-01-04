/**
 * Session metadata API functions
 * Re-exports from the generated SDK for consistent API access
 */

import {
  updateSessionMetadata as sdkUpdateSessionMetadata,
  getAllTags as sdkGetAllTags,
} from '../../api/sdk.gen';

export interface SessionMetadataUpdatePayload {
  isFavorite?: boolean;
  tags?: string[];
}

export interface SessionMetadataResponse {
  isFavorite: boolean;
  tags: string[];
}

/**
 * Update session metadata (favorites and/or tags)
 */
export async function updateSessionMetadata(
  sessionId: string,
  data: SessionMetadataUpdatePayload
): Promise<SessionMetadataResponse> {
  const response = await sdkUpdateSessionMetadata({
    path: { session_id: sessionId },
    body: data,
  });

  if (response.error) {
    throw new Error(`Failed to update metadata: ${response.response.status}`);
  }

  return response.data as SessionMetadataResponse;
}

/**
 * Get all available tags across all sessions
 */
export async function getAllTags(): Promise<string[]> {
  const response = await sdkGetAllTags();

  if (response.error) {
    throw new Error(`Failed to get tags: ${response.response.status}`);
  }

  return (response.data as { tags: string[] }).tags;
}
