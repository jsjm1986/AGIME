export function formatMessageTimestamp(timestamp?: number, locale: string = 'en-US'): string {
  const date = timestamp ? new Date(timestamp * 1000) : new Date();
  const now = new Date();

  // Format time - use 12-hour format for English, 24-hour for Chinese
  const is24Hour = locale.startsWith('zh');
  const timeStr = date.toLocaleTimeString(locale, {
    hour: 'numeric',
    minute: '2-digit',
    hour12: !is24Hour,
  });

  // Check if the message is from today
  if (
    date.getDate() === now.getDate() &&
    date.getMonth() === now.getMonth() &&
    date.getFullYear() === now.getFullYear()
  ) {
    return timeStr;
  }

  // If not today, format as localized date + time
  const dateStr = date.toLocaleDateString(locale, {
    month: '2-digit',
    day: '2-digit',
    year: 'numeric',
  });

  return `${dateStr} ${timeStr}`;
}

/**
 * Format a timestamp as a relative time string (e.g., "2 hours ago", "yesterday")
 * @param timestamp - Unix timestamp in seconds
 * @param locale - Locale string (e.g., 'en-US', 'zh-CN')
 */
export function formatRelativeTime(timestamp: number, locale: string = 'en-US'): string {
  const now = Date.now();
  const date = new Date(timestamp * 1000);
  const diffMs = now - date.getTime();
  const diffSeconds = Math.floor(diffMs / 1000);
  const diffMinutes = Math.floor(diffSeconds / 60);
  const diffHours = Math.floor(diffMinutes / 60);
  const diffDays = Math.floor(diffHours / 24);

  const isZh = locale.startsWith('zh');

  // Less than 1 minute
  if (diffSeconds < 60) {
    return isZh ? '刚刚' : 'just now';
  }

  // Less than 1 hour
  if (diffMinutes < 60) {
    return isZh
      ? `${diffMinutes} 分钟前`
      : `${diffMinutes} minute${diffMinutes !== 1 ? 's' : ''} ago`;
  }

  // Less than 24 hours
  if (diffHours < 24) {
    return isZh
      ? `${diffHours} 小时前`
      : `${diffHours} hour${diffHours !== 1 ? 's' : ''} ago`;
  }

  // Yesterday
  if (diffDays === 1) {
    return isZh ? '昨天' : 'yesterday';
  }

  // Less than 7 days
  if (diffDays < 7) {
    return isZh
      ? `${diffDays} 天前`
      : `${diffDays} day${diffDays !== 1 ? 's' : ''} ago`;
  }

  // More than 7 days - show date
  return date.toLocaleDateString(locale, {
    month: 'short',
    day: 'numeric',
    year: date.getFullYear() !== new Date().getFullYear() ? 'numeric' : undefined,
  });
}
