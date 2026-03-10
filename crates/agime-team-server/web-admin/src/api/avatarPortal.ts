import {
  portalApi,
  type AvatarGovernanceEventPayload,
  type AvatarGovernanceQueueItemPayload,
  type AvatarGovernanceStatePayload,
  type AvatarInstanceProjection,
  type AvatarWorkbenchSnapshotPayload,
  type CreatePortalRequest,
  type PortalDetail,
  type PortalDocumentAccessMode,
  type PortalSummary,
  type UpdatePortalRequest,
} from './portal';

export type {
  AvatarGovernanceEventPayload,
  AvatarGovernanceQueueItemPayload,
  AvatarGovernanceStatePayload,
  AvatarInstanceProjection,
  AvatarWorkbenchSnapshotPayload,
  CreatePortalRequest,
  PortalDetail,
  PortalDocumentAccessMode,
  PortalSummary,
  UpdatePortalRequest,
};

function governanceStateFromSettings(
  settings: Record<string, unknown> | undefined,
): Record<string, unknown> | undefined {
  const governance = settings?.digitalAvatarGovernance;
  if (!governance || typeof governance !== 'object') {
    return undefined;
  }
  const normalized = { ...(governance as Record<string, unknown>) };
  delete normalized.config;
  return Object.keys(normalized).length > 0 ? normalized : undefined;
}

function governanceConfigFromSettings(
  settings: Record<string, unknown> | undefined,
): Record<string, unknown> | undefined {
  const topLevel = settings?.digitalAvatarGovernanceConfig;
  if (topLevel && typeof topLevel === 'object') {
    return topLevel as Record<string, unknown>;
  }
  const governance = settings?.digitalAvatarGovernance;
  if (governance && typeof governance === 'object') {
    const nested = (governance as Record<string, unknown>).config;
    if (nested && typeof nested === 'object') {
      return nested as Record<string, unknown>;
    }
  }
  return undefined;
}

function hasGovernanceOverrides(settings: Record<string, unknown> | undefined): boolean {
  return Boolean(
    governanceStateFromSettings(settings) || governanceConfigFromSettings(settings),
  );
}

export const avatarPortalApi = {
  async list(teamId: string, page = 1, limit = 20) {
    return portalApi.list(teamId, page, limit, 'avatar');
  },

  async listInstances(teamId: string): Promise<AvatarInstanceProjection[]> {
    return portalApi.listAvatarInstances(teamId);
  },

  async get(teamId: string, portalId: string): Promise<PortalDetail> {
    return portalApi.get(teamId, portalId);
  },

  async create(teamId: string, req: CreatePortalRequest): Promise<PortalDetail> {
    const detail = await portalApi.create(teamId, req);
    if (!hasGovernanceOverrides(req.settings)) {
      return detail;
    }
    await portalApi.updateAvatarGovernance(teamId, detail.id, {
      state: governanceStateFromSettings(req.settings),
      config: governanceConfigFromSettings(req.settings),
    });
    return portalApi.get(teamId, detail.id);
  },

  async update(teamId: string, portalId: string, req: UpdatePortalRequest): Promise<PortalDetail> {
    const detail = await portalApi.update(teamId, portalId, req);
    if (!hasGovernanceOverrides(req.settings)) {
      return detail;
    }
    await portalApi.updateAvatarGovernance(teamId, portalId, {
      state: governanceStateFromSettings(req.settings),
      config: governanceConfigFromSettings(req.settings),
    });
    return portalApi.get(teamId, portalId);
  },

  async delete(teamId: string, portalId: string): Promise<void> {
    return portalApi.delete(teamId, portalId);
  },

  async publish(teamId: string, portalId: string): Promise<PortalDetail> {
    return portalApi.publish(teamId, portalId);
  },

  async unpublish(teamId: string, portalId: string): Promise<PortalDetail> {
    return portalApi.unpublish(teamId, portalId);
  },

  async getGovernance(teamId: string, portalId: string): Promise<AvatarGovernanceStatePayload> {
    return portalApi.getAvatarGovernance(teamId, portalId);
  },

  async updateGovernance(
    teamId: string,
    portalId: string,
    req: { state?: Record<string, unknown>; config?: Record<string, unknown> },
  ): Promise<AvatarGovernanceStatePayload> {
    return portalApi.updateAvatarGovernance(teamId, portalId, req);
  },

  async listGovernanceEvents(
    teamId: string,
    portalId: string,
    limit = 120,
  ): Promise<AvatarGovernanceEventPayload[]> {
    return portalApi.listAvatarGovernanceEvents(teamId, portalId, limit);
  },

  async listTeamGovernanceEvents(
    teamId: string,
    limit = 300,
    portalId?: string,
  ): Promise<AvatarGovernanceEventPayload[]> {
    return portalApi.listTeamAvatarGovernanceEvents(teamId, limit, portalId);
  },

  async listGovernanceQueue(
    teamId: string,
    portalId: string,
  ): Promise<AvatarGovernanceQueueItemPayload[]> {
    return portalApi.listAvatarGovernanceQueue(teamId, portalId);
  },

  async getWorkbenchSnapshot(
    teamId: string,
    portalId: string,
  ): Promise<AvatarWorkbenchSnapshotPayload> {
    return portalApi.getAvatarWorkbenchSnapshot(teamId, portalId);
  },
};
