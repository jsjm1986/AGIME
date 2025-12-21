import React, { useState, useMemo, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from './ui/button';
import { Pill } from './ui/Pill';
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
}

interface ThinkingStats {
  tokenCount: number;
  charCount: number;
  lineCount: number;
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
 * ThinkingBlock Component
 *
 * Displays AI thinking/reasoning content in a collapsible block with:
 * - Expand/collapse animation
 * - Brain icon for visual identification
 * - Statistics (word count, character count, line count)
 * - Copy button for easy content copying
 *
 * Follows the ToolCallExpandable pattern for consistency.
 */
export default function ThinkingBlock({
  content,
  className,
  isStartExpanded = false,
  isStreaming = false,
}: ThinkingBlockProps) {
  const { t } = useTranslation('chat');
  // Auto-expand when streaming for real-time visibility
  const [isExpanded, setIsExpanded] = useState(isStartExpanded || isStreaming);
  const [isCopied, setIsCopied] = useState(false);

  // Auto-expand when streaming starts
  React.useEffect(() => {
    if (isStreaming) {
      setIsExpanded(true);
    }
  }, [isStreaming]);

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
        'w-full text-sm font-sans rounded-lg overflow-hidden',
        'border border-borderSubtle bg-background-muted',
        'transition-all duration-200',
        className
      )}
    >
      {/* Header - Click to expand/collapse */}
      <Button
        onClick={handleToggle}
        className="group w-full flex justify-between items-center pr-3 transition-colors rounded-none"
        variant="ghost"
      >
        <span className="flex items-center gap-2 min-w-0 flex-1">
          {/* Brain icon with purple accent */}
          <Brain className="h-4 w-4 text-purple-500 shrink-0" />
          {/* Title */}
          <span className="truncate font-medium">
            {t('thinkingBlock.title', 'Thinking Process')}
          </span>
          {/* Token count pill */}
          <Pill size="xs" variant="glass" color="purple" className="ml-1">
            {t('thinkingBlock.tokenCount', '~{{count}} tokens', { count: stats.tokenCount })}
          </Pill>
        </span>
        {/* Chevron indicator */}
        <ChevronRight
          className={cn(
            'h-4 w-4 shrink-0 transition-transform duration-200',
            'opacity-70 group-hover:opacity-100',
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
        <div className="flex justify-between items-center px-3 py-2 border-t border-borderSubtle bg-bgSubtle/50">
          {/* Detailed stats */}
          <span className="text-xs text-textMuted">
            {t('thinkingBlock.stats', '{{chars}} chars | {{lines}} lines', {
              chars: stats.charCount.toLocaleString(),
              lines: stats.lineCount,
            })}
          </span>
          {/* Copy button */}
          <Button
            onClick={handleCopy}
            variant="ghost"
            size="xs"
            className="h-6 px-2 text-xs gap-1 hover:bg-background-muted"
          >
            {isCopied ? (
              <>
                <Check className="h-3 w-3 text-green-500" />
                <span className="text-green-600 dark:text-green-400">
                  {t('thinkingBlock.copied', 'Copied')}
                </span>
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
        <div className="px-4 py-3 border-t border-borderSubtle">
          <MarkdownContent
            content={content}
            className="prose-sm max-w-none text-textSubtle"
          />
        </div>
      </div>
    </div>
  );
}

export { ThinkingBlock };
export type { ThinkingBlockProps, ThinkingStats };
