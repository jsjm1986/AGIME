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
  const extracted = extractInviteCode(trimmed);
  return extracted ? `/join/${extracted}` : trimmed.startsWith('/') ? trimmed : `/join/${trimmed}`;
}

export function buildInviteUrl(codeOrPath: string): string {
  const path = normalizeInvitePath(codeOrPath);
  if (typeof window === 'undefined') {
    return path;
  }
  return `${window.location.origin}${path}`;
}

export function extractInviteCode(value: string): string | null {
  const raw = value.trim();
  if (!raw) {
    return null;
  }

  if (/^[a-zA-Z0-9-]{8,}$/.test(raw) && !raw.includes('/')) {
    return raw;
  }

  try {
    if (raw.startsWith('http://') || raw.startsWith('https://')) {
      const url = new URL(raw);
      const segments = url.pathname.split('/').filter(Boolean);
      const joinIndex = segments.lastIndexOf('join');
      if (joinIndex >= 0 && segments[joinIndex + 1]) {
        return segments[joinIndex + 1];
      }
    }
  } catch {
    // Fall through to path parsing below.
  }

  const path = raw.replace(/^\/admin/, '');
  const segments = path.split('/').filter(Boolean);
  const joinIndex = segments.lastIndexOf('join');
  if (joinIndex >= 0 && segments[joinIndex + 1]) {
    return segments[joinIndex + 1];
  }

  return null;
}
