import React, { useState, useMemo, useCallback, useRef, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from './ui/button';
import MarkdownContent from './MarkdownContent';
import { cn } from '../utils';
import { ChevronRight, Brain, Copy, Check } from 'lucide-react';

interface ThinkingBlockProps {
  /** The thinking content to display */
  content: string;
  /** Optional className for custom styling */
  className?: string;
  /** Whether to start expanded (default: false) */
  isStartExpanded?: boolean;
  /** Whether the content is currently being streamed (auto-expands when true) */
  isStreaming?: boolean;
  /** Message ID for persisting duration across re-renders */
  messageId?: string;
}

interface ThinkingStats {
  tokenCount: number;
  charCount: number;
  lineCount: number;
}

/**
 * Module-level cache for thinking durations
 * Persists durations across component unmount/remount cycles
 * Key: messageId or content hash, Value: duration in seconds
 */
const thinkingDurationCache = new Map<string, number>();

/**
 * Module-level cache for thinking content
 * Persists thinking content across component unmount/remount cycles
 * This is essential for tag-based thinking models (DeepSeek, Qwen, etc.)
 * where content might be lost when switching between conversations
 * Key: messageId or content hash, Value: thinking content string
 */
const thinkingContentCache = new Map<string, string>();

/**
 * Generate a simple hash from content for cache key fallback
 */
function generateContentHash(content: string): string {
  let hash = 0;
  for (let i = 0; i < Math.min(content.length, 500); i++) {
    const char = content.charCodeAt(i);
    hash = ((hash << 5) - hash) + char;
    hash = hash & hash; // Convert to 32-bit integer
  }
  return `content_${hash}_${content.length}`;
}

/**
 * Save thinking content to cache
 * Called from GooseMessage when thinking content is extracted
 */
export function cacheThinkingContent(messageId: string, content: string): void {
  if (messageId && content) {
    thinkingContentCache.set(messageId, content);
  }
}

/**
 * Get cached thinking content
 */
export function getCachedThinkingContent(messageId: string): string | undefined {
  return thinkingContentCache.get(messageId);
}

/**
 * Estimate token count for thinking content
 * Uses approximate ratios:
 * - CJK characters: ~0.7 tokens per character (1 char ≈ 0.7 token)
 * - English/other: ~0.25 tokens per character (4 chars ≈ 1 token)
 */
function calculateStats(content: string): ThinkingStats {
  const trimmed = content.trim();
  if (!trimmed) {
    return { tokenCount: 0, charCount: 0, lineCount: 0 };
  }

  // Character count: excluding whitespace
  const charCount = trimmed.replace(/\s/g, '').length;

  // Token estimation: handle both CJK and non-CJK text
  const cjkPattern = /[\u4e00-\u9fff\u3400-\u4dbf\u3040-\u309f\u30a0-\u30ff\uac00-\ud7af]/g;
  const cjkChars = trimmed.match(cjkPattern) || [];
  const cjkCount = cjkChars.length;

  // Non-CJK character count (excluding whitespace)
  const nonCjkCount = charCount - cjkCount;

  // Estimate tokens:
  // - CJK: ~0.7 tokens per character
  // - Non-CJK: ~0.25 tokens per character (4 chars per token)
  const tokenCount = Math.ceil(cjkCount * 0.7 + nonCjkCount * 0.25);

  // Line count: split by newlines
  const lines = trimmed.split(/\r?\n/);
  const lineCount = lines.length;

  return { tokenCount, charCount, lineCount };
}

/**
 * Format duration in a human-readable way
 */
function formatDuration(seconds: number): string {
  if (seconds < 1) {
    return `${Math.round(seconds * 1000)}ms`;
  } else if (seconds < 60) {
    return `${seconds.toFixed(1)}s`;
  } else {
    const mins = Math.floor(seconds / 60);
    const secs = Math.round(seconds % 60);
    return `${mins}m ${secs}s`;
  }
}

/**
 * ThinkingBlock Component
 *
 * Displays AI thinking/reasoning content in a collapsible block with:
 * - Expand/collapse animation
 * - Brain icon for visual identification
 * - Statistics (token count, duration)
 * - Copy button for easy content copying
 * - Duration persistence across component remounts
 *
 * Follows the ToolCallExpandable pattern for consistency.
 */
export default function ThinkingBlock({
  content,
  className,
  isStartExpanded = false,
  isStreaming = false,
  messageId,
}: ThinkingBlockProps) {
  const { t } = useTranslation('chat');

  // Generate cache key: prefer messageId, fallback to content hash
  const cacheKey = useMemo(() => {
    return messageId || generateContentHash(content);
  }, [messageId, content]);

  // Auto-expand when streaming for real-time visibility
  const [isExpanded, setIsExpanded] = useState(isStartExpanded || isStreaming);
  const [isCopied, setIsCopied] = useState(false);

  // Duration tracking with cache restoration
  const startTimeRef = useRef<number | null>(null);
  const [duration, setDuration] = useState<number | null>(() => {
    // Restore duration from cache on mount
    return thinkingDurationCache.get(cacheKey) ?? null;
  });

  // Auto-expand when streaming starts
  useEffect(() => {
    if (isStreaming) {
      setIsExpanded(true);
    }
  }, [isStreaming]);

  // Track thinking duration and persist to cache
  useEffect(() => {
    if (isStreaming && startTimeRef.current === null) {
      // Start timing when streaming begins
      startTimeRef.current = Date.now();
      setDuration(null);
    } else if (!isStreaming && startTimeRef.current !== null) {
      // Calculate duration when streaming ends
      const elapsed = (Date.now() - startTimeRef.current) / 1000;
      setDuration(elapsed);
      // Persist to cache for future restoration
      thinkingDurationCache.set(cacheKey, elapsed);
      startTimeRef.current = null;
    }
  }, [isStreaming, cacheKey]);

  // Update duration in real-time while streaming
  useEffect(() => {
    if (!isStreaming || startTimeRef.current === null) return;

    const interval = setInterval(() => {
      if (startTimeRef.current !== null) {
        const elapsed = (Date.now() - startTimeRef.current) / 1000;
        setDuration(elapsed);
      }
    }, 100);

    return () => clearInterval(interval);
  }, [isStreaming]);

  // Restore duration from cache when cacheKey changes (e.g., switching back to a conversation)
  useEffect(() => {
    if (!isStreaming) {
      const cachedDuration = thinkingDurationCache.get(cacheKey);
      if (cachedDuration !== undefined && duration === null) {
        setDuration(cachedDuration);
      }
    }
  }, [cacheKey, isStreaming, duration]);

  // Calculate content statistics
  const stats = useMemo(() => calculateStats(content), [content]);

  // Toggle expand/collapse
  const handleToggle = useCallback(() => {
    setIsExpanded((prev) => !prev);
  }, []);

  // Copy content to clipboard with fallback
  const handleCopy = useCallback(
    async (e: React.MouseEvent) => {
      // Prevent event bubbling
      e.stopPropagation();

      try {
        // Try modern Clipboard API first
        if (navigator.clipboard && window.isSecureContext) {
          await navigator.clipboard.writeText(content);
        } else {
          // Fallback for non-secure contexts (rare in Electron, but safe)
          const textArea = document.createElement('textarea');
          textArea.value = content;
          textArea.style.position = 'fixed';
          textArea.style.left = '-999999px';
          textArea.style.top = '-999999px';
          document.body.appendChild(textArea);
          textArea.focus();
          textArea.select();
          document.execCommand('copy');
          document.body.removeChild(textArea);
        }
        setIsCopied(true);
        setTimeout(() => setIsCopied(false), 2000);
      } catch (err) {
        console.error('Failed to copy thinking content:', err);
      }
    },
    [content]
  );

  if (!content.trim()) {
    return null;
  }

  return (
    <div
      className={cn(
        'w-full text-sm font-sans overflow-hidden',
        'bg-blue-500/5 dark:bg-blue-500/10 rounded-lg',
        'border border-blue-500/20 dark:border-blue-400/20',
        'transition-all duration-200',
        className
      )}
    >
      {/* Header - Click to expand/collapse */}
      <Button
        onClick={handleToggle}
        className="group w-full flex justify-between items-center px-3 py-2.5 transition-colors rounded-none h-auto"
        variant="ghost"
      >
        <span className="flex items-center gap-2 min-w-0 flex-1">
          {/* Brain icon */}
          <Brain className="h-4 w-4 text-blue-500 dark:text-blue-400 shrink-0" />
          {/* Title */}
          <span className="text-xs text-text-default/80 font-medium">
            {t('thinkingBlock.title', 'Thinking Process')}
          </span>
          {/* Stats badges */}
          <span className="flex items-center gap-1.5">
            {/* Token count */}
            <span className="text-xs text-text-muted/60 bg-blue-500/10 px-1.5 py-0.5 rounded">
              ~{stats.tokenCount.toLocaleString()} tokens
            </span>
            {/* Duration - show with streaming indicator or final time */}
            {duration !== null && (
              <span className={cn(
                "text-xs px-1.5 py-0.5 rounded flex items-center gap-1",
                isStreaming
                  ? "text-blue-500 bg-blue-500/10"
                  : "text-text-muted/60 bg-slate-500/10"
              )}>
                {isStreaming && (
                  <span className="w-1.5 h-1.5 bg-blue-500 rounded-full animate-pulse" />
                )}
                {formatDuration(duration)}
              </span>
            )}
          </span>
        </span>
        {/* Chevron indicator */}
        <ChevronRight
          className={cn(
            'h-4 w-4 shrink-0 transition-transform duration-200 text-blue-500/60',
            'opacity-60 group-hover:opacity-100',
            isExpanded && 'rotate-90'
          )}
        />
      </Button>

      {/* Content - Animated expand/collapse */}
      <div
        className={cn(
          'transition-all duration-300 ease-in-out overflow-hidden',
          isExpanded ? 'max-h-[10000px] opacity-100' : 'max-h-0 opacity-0'
        )}
      >
        {/* Toolbar: Stats + Copy button */}
        <div className="flex justify-between items-center px-3 py-1.5 border-t border-blue-500/10">
          {/* Detailed stats */}
          <span className="text-xs text-text-muted/60">
            {stats.charCount.toLocaleString()} chars · {stats.lineCount} lines
          </span>
          {/* Copy button */}
          <Button
            onClick={handleCopy}
            variant="ghost"
            size="xs"
            className="h-5 px-1.5 text-xs gap-1 text-text-muted hover:text-text-default"
          >
            {isCopied ? (
              <>
                <Check className="h-3 w-3" />
                <span>{t('thinkingBlock.copied', 'Copied')}</span>
              </>
            ) : (
              <>
                <Copy className="h-3 w-3" />
                <span>{t('thinkingBlock.copy', 'Copy')}</span>
              </>
            )}
          </Button>
        </div>

        {/* Markdown content */}
        <div className="px-3 py-3 border-t border-blue-500/10">
          <MarkdownContent
            content={content}
            className="prose-sm max-w-none text-text-muted leading-relaxed"
          />
        </div>
      </div>
    </div>
  );
}

export { ThinkingBlock };
export type { ThinkingBlockProps, ThinkingStats };
