function stripAdminBasename(path: string): string {
  if (path === '/admin') {
    return '/';
  }
  if (path.startsWith('/admin/')) {
    return path.slice('/admin'.length);
  }
  return path;
}

export function resolveSafeRedirectPath(
  value: string | null | undefined,
  fallback = '/dashboard',
): string {
  if (!value) {
    return fallback;
  }

  const raw = value.trim();
  if (!raw || raw.startsWith('//')) {
    return fallback;
  }

  try {
    if (raw.startsWith('http://') || raw.startsWith('https://')) {
      const origin =
        typeof window !== 'undefined' ? window.location.origin : 'http://localhost';
      const url = new URL(raw);
      if (url.origin !== origin) {
        return fallback;
      }
      const normalized = stripAdminBasename(`${url.pathname}${url.search}${url.hash}`);
      return normalized.startsWith('/') && !normalized.startsWith('//') && normalized !== '/'
        ? normalized
        : fallback;
    }

    if (!raw.startsWith('/')) {
      return fallback;
    }

    const normalized = stripAdminBasename(raw);
    return normalized !== '/' ? normalized : fallback;
  } catch {
    return fallback;
  }
}

export function buildRedirectQuery(path: string): string {
  return `?redirect=${encodeURIComponent(path)}`;
}

export function normalizeInvitePath(codeOrPath: string): string {
  const trimmed = codeOrPath.trim();
  if (!trimmed) {
    return '/join';
  }
  return trimmed.startsWith('/') ? trimmed : `/join/${trimmed}`;
}

export function buildInviteUrl(codeOrPath: string): string {
  const path = normalizeInvitePath(codeOrPath);
  if (typeof window === 'undefined') {
    return path;
  }
  return `${window.location.origin}${path}`;
}
