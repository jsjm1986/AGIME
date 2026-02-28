export function formatDate(date: string | Date): string {
  const d = typeof date === 'string' ? new Date(date) : date;
  return d.toLocaleDateString();
}

export function formatDateTime(date: string | Date): string {
  const d = typeof date === 'string' ? new Date(date) : date;
  return d.toLocaleString();
}

export function formatRelativeTime(
  date: string | Date,
  t?: (k: string, opts?: Record<string, unknown>) => string
): string {
  const d = typeof date === 'string' ? new Date(date) : date;
  const diff = Date.now() - d.getTime();
  const min = Math.floor(diff / 60000);
  if (min < 1) return t?.('chat.justNow') || 'now';
  if (min < 60) return t?.('chat.minutesAgo', { n: min }) || `${min}m`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return t?.('chat.hoursAgo', { n: hr }) || `${hr}h`;
  const days = Math.floor(hr / 24);
  return `${days}d`;
}

export function formatTime(date: string | Date): string {
  const d = typeof date === 'string' ? new Date(date) : date;
  return d.toLocaleTimeString();
}

// Re-export from documents API for convenience
export { formatFileSize } from '../api/documents';
