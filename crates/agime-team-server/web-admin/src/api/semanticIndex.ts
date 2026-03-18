import { fetchApi } from './client';

const API_BASE = '/api/team';

export type SemanticEntityType =
  | 'portal'
  | 'agent'
  | 'document'
  | 'folder'
  | 'skill'
  | 'extension'
  | 'governance_request';

export interface SemanticIndexEntity {
  id: string;
  type: SemanticEntityType;
  name: string;
  displayName?: string | null;
  aliases: string[];
  status?: string | null;
  scope: string;
  teamId: string;
  portalId?: string | null;
  metadata: Record<string, unknown>;
}

export interface SemanticBuiltinCatalogItem {
  id: string;
  type: 'extension';
  name: string;
  displayName: string;
  aliases: string[];
  description?: string | null;
  scope: string;
  isPlatform: boolean;
  metadata: Record<string, unknown>;
}

export interface SemanticIndexResponse {
  teamId: string;
  version: string;
  generatedAt: string;
  entities: SemanticIndexEntity[];
  builtinCatalog: SemanticBuiltinCatalogItem[];
}

function normalizeEntity(
  entity: Partial<SemanticIndexEntity> & Record<string, unknown>,
): SemanticIndexEntity {
  return {
    id: typeof entity.id === 'string' ? entity.id : '',
    type: (entity.type as SemanticEntityType) ?? 'document',
    name: typeof entity.name === 'string' ? entity.name : '',
    displayName:
      typeof entity.displayName === 'string' ? entity.displayName : null,
    aliases: Array.isArray(entity.aliases)
      ? entity.aliases.filter((value): value is string => typeof value === 'string')
      : [],
    status: typeof entity.status === 'string' ? entity.status : null,
    scope: typeof entity.scope === 'string' ? entity.scope : 'team',
    teamId: typeof entity.teamId === 'string' ? entity.teamId : '',
    portalId: typeof entity.portalId === 'string' ? entity.portalId : null,
    metadata:
      entity.metadata && typeof entity.metadata === 'object' && !Array.isArray(entity.metadata)
        ? (entity.metadata as Record<string, unknown>)
        : {},
  };
}

function normalizeBuiltinCatalogItem(
  builtin: Partial<SemanticBuiltinCatalogItem> & Record<string, unknown>,
): SemanticBuiltinCatalogItem {
  return {
    id: typeof builtin.id === 'string' ? builtin.id : '',
    type: 'extension',
    name: typeof builtin.name === 'string' ? builtin.name : '',
    displayName:
      typeof builtin.displayName === 'string' ? builtin.displayName : '',
    aliases: Array.isArray(builtin.aliases)
      ? builtin.aliases.filter((value): value is string => typeof value === 'string')
      : [],
    description:
      typeof builtin.description === 'string' ? builtin.description : null,
    scope: typeof builtin.scope === 'string' ? builtin.scope : 'builtin',
    isPlatform: Boolean(builtin.isPlatform),
    metadata:
      builtin.metadata &&
      typeof builtin.metadata === 'object' &&
      !Array.isArray(builtin.metadata)
        ? (builtin.metadata as Record<string, unknown>)
        : {},
  };
}

function normalizeSemanticIndexResponse(
  payload: Partial<SemanticIndexResponse> & Record<string, unknown>,
): SemanticIndexResponse {
  return {
    teamId: typeof payload.teamId === 'string' ? payload.teamId : '',
    version: typeof payload.version === 'string' ? payload.version : '',
    generatedAt: typeof payload.generatedAt === 'string' ? payload.generatedAt : '',
    entities: Array.isArray(payload.entities)
      ? payload.entities.map((entity) =>
          normalizeEntity(entity as Partial<SemanticIndexEntity> & Record<string, unknown>),
        )
      : [],
    builtinCatalog: Array.isArray(payload.builtinCatalog)
      ? payload.builtinCatalog.map((builtin) =>
          normalizeBuiltinCatalogItem(
            builtin as Partial<SemanticBuiltinCatalogItem> & Record<string, unknown>,
          ),
        )
      : [],
  };
}

export const semanticIndexApi = {
  get(teamId: string): Promise<SemanticIndexResponse> {
    return fetchApi<SemanticIndexResponse>(
      `${API_BASE}/teams/${encodeURIComponent(teamId)}/semantic-index`,
    ).then((payload) =>
      normalizeSemanticIndexResponse(
        payload as Partial<SemanticIndexResponse> & Record<string, unknown>,
      ),
    );
  },
};
