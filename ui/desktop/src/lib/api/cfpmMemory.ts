import { client } from '../../api/client.gen';

export interface MemoryFact {
  id: string;
  sessionId: string;
  category: string;
  content: string;
  status: 'active' | 'stale' | 'forgotten' | 'superseded';
  pinned: boolean;
  source: string;
  confidence?: number;
  evidenceCount?: number;
  lastValidatedAt?: string | null;
  validationCommand?: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface MemoryFactDraft {
  category: string;
  content: string;
  source?: string;
  pinned?: boolean;
}

export interface MemoryFactPatch {
  category?: string;
  content?: string;
  status?: 'active' | 'stale' | 'forgotten' | 'superseded';
  pinned?: boolean;
}

export interface MemorySnapshotRecord {
  id: number;
  sessionId: string;
  reason: string;
  factCount: number;
  createdAt: string;
}

export interface MemoryCandidate {
  id: string;
  sessionId: string;
  category: string;
  content: string;
  source: string;
  decision: 'accepted' | 'rejected' | string;
  reason: string;
  createdAt: string;
}

export interface MemoryToolGateEvent {
  action: string;
  tool: string;
  target: string;
  path: string;
  originalCommand: string;
  rewrittenCommand: string;
  verbosity: string;
  createdTimestamp: number;
}

export interface RenameMemoryPathRequest {
  fromPath: string;
  toPath: string;
}

export interface RenameMemoryPathResponse {
  updatedCount: number;
}

export interface RollbackMemorySnapshotResponse {
  restoredCount: number;
}

export const listMemoryFacts = (sessionId: string) =>
  client.get<MemoryFact[]>({
    url: `/sessions/${sessionId}/memory/facts`,
  });

export const listMemoryCandidates = (
  sessionId: string,
  options?: {
    decision?: 'accepted' | 'rejected';
    limit?: number;
  }
) => {
  const query = new URLSearchParams();
  if (options?.decision) {
    query.set('decision', options.decision);
  }
  if (typeof options?.limit === 'number' && Number.isFinite(options.limit)) {
    query.set('limit', String(Math.max(1, Math.floor(options.limit))));
  }

  const queryString = query.toString();
  const suffix = queryString ? `?${queryString}` : '';

  return client.get<MemoryCandidate[]>({
    url: `/sessions/${sessionId}/memory/candidates${suffix}`,
  });
};

export const listMemoryToolGateEvents = (
  sessionId: string,
  options?: {
    limit?: number;
  }
) => {
  const query = new URLSearchParams();
  if (typeof options?.limit === 'number' && Number.isFinite(options.limit)) {
    query.set('limit', String(Math.max(1, Math.floor(options.limit))));
  }

  const queryString = query.toString();
  const suffix = queryString ? `?${queryString}` : '';

  return client.get<MemoryToolGateEvent[]>({
    url: `/sessions/${sessionId}/memory/tool-gates${suffix}`,
  });
};

export const createMemoryFact = (sessionId: string, body: MemoryFactDraft) =>
  client.post<MemoryFact>({
    url: `/sessions/${sessionId}/memory/facts`,
    body,
    headers: {
      'Content-Type': 'application/json',
    },
  });

export const patchMemoryFact = (
  sessionId: string,
  factId: string,
  body: MemoryFactPatch
) =>
  client.patch<MemoryFact>({
    url: `/sessions/${sessionId}/memory/facts/${factId}`,
    body,
    headers: {
      'Content-Type': 'application/json',
    },
  });

export const renameMemoryPaths = (
  sessionId: string,
  body: RenameMemoryPathRequest
) =>
  client.post<RenameMemoryPathResponse>({
    url: `/sessions/${sessionId}/memory/path-rename`,
    body,
    headers: {
      'Content-Type': 'application/json',
    },
  });

export const listMemorySnapshots = (sessionId: string) =>
  client.get<MemorySnapshotRecord[]>({
    url: `/sessions/${sessionId}/memory/snapshots`,
  });

export const rollbackMemorySnapshot = (
  sessionId: string,
  snapshotId: number
) =>
  client.post<RollbackMemorySnapshotResponse>({
    url: `/sessions/${sessionId}/memory/rollback`,
    body: {
      snapshotId,
    },
    headers: {
      'Content-Type': 'application/json',
    },
  });
