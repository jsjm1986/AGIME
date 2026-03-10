import type { AvatarInstanceProjection, PortalSummary } from '../../../api/portal';

export type UiAvatarType = 'external' | 'internal' | 'unknown';

export function detectAvatarType(
  summary: PortalSummary,
  projection?: AvatarInstanceProjection | null,
): UiAvatarType {
  const projectedType = `${projection?.avatarType || ''}`.trim().toLowerCase();
  if (projectedType === 'external_service') return 'external';
  if (projectedType === 'internal_worker') return 'internal';
  if ((summary.tags || []).includes('avatar:external')) return 'external';
  if ((summary.tags || []).includes('avatar:internal')) return 'internal';
  return 'unknown';
}
