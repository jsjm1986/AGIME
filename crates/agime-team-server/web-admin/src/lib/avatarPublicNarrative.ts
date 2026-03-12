export interface AvatarPublicNarrative {
  heroIntro?: string;
  heroUseCases?: string[];
  heroWorkingStyle?: string;
  heroCtaHint?: string;
}

function asTrimmedString(value: unknown): string {
  return typeof value === 'string' ? value.trim() : '';
}

export function readAvatarPublicNarrative(
  settings: Record<string, unknown> | undefined | null,
): AvatarPublicNarrative {
  const raw = settings?.avatarPublicNarrative;
  if (!raw || typeof raw !== 'object') {
    return {};
  }
  const data = raw as Record<string, unknown>;
  const heroUseCases = Array.isArray(data.heroUseCases)
    ? data.heroUseCases
        .map(item => asTrimmedString(item))
        .filter(Boolean)
    : [];
  return {
    heroIntro: asTrimmedString(data.heroIntro) || undefined,
    heroUseCases,
    heroWorkingStyle: asTrimmedString(data.heroWorkingStyle) || undefined,
    heroCtaHint: asTrimmedString(data.heroCtaHint) || undefined,
  };
}

export function buildAvatarPublicNarrativePayload(
  input: AvatarPublicNarrative,
): AvatarPublicNarrative | undefined {
  const heroIntro = asTrimmedString(input.heroIntro);
  const heroUseCases = (input.heroUseCases || [])
    .map(item => asTrimmedString(item))
    .filter(Boolean);
  const heroWorkingStyle = asTrimmedString(input.heroWorkingStyle);
  const heroCtaHint = asTrimmedString(input.heroCtaHint);

  if (!heroIntro && heroUseCases.length === 0 && !heroWorkingStyle && !heroCtaHint) {
    return undefined;
  }

  return {
    ...(heroIntro ? { heroIntro } : {}),
    ...(heroUseCases.length > 0 ? { heroUseCases } : {}),
    ...(heroWorkingStyle ? { heroWorkingStyle } : {}),
    ...(heroCtaHint ? { heroCtaHint } : {}),
  };
}

export function splitNarrativeUseCases(value: string): string[] {
  return String(value || '')
    .split(/\r?\n/)
    .map(item => item.trim())
    .filter(Boolean);
}

export function joinNarrativeUseCases(items: string[] | undefined): string {
  return Array.isArray(items) ? items.join('\n') : '';
}
