import { useState, useEffect, useCallback, useMemo, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { Button } from '../ui/button';
import { Skeleton } from '../ui/skeleton';
import { apiClient } from '../../api/client';
import { documentApi } from '../../api/documents';
import { BUILTIN_EXTENSIONS, BUILTIN_SKILLS } from '../../api/agent';
import type { SmartLogEntry } from '../../api/types';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../ui/select';
import { RefreshCw, FileText, Zap, Puzzle, BookOpen, Loader2, ChevronDown, ChevronUp, ChevronRight, Sparkles, Eye, EyeOff, ChevronsUpDown, MessageSquareText, SlidersHorizontal } from 'lucide-react';
import MarkdownContent from '../MarkdownContent';
import { formatDateTime } from '../../utils/format';
import { useMobileInteractionMode } from '../../contexts/MobileInteractionModeContext';
import { ContextSummaryBar } from '../mobile/ContextSummaryBar';
import { BottomSheetPanel } from '../mobile/BottomSheetPanel';
import { ManagementRail } from '../mobile/ManagementRail';
import { MobileWorkspaceShell } from '../mobile/MobileWorkspaceShell';

type TabType = 'activity' | 'insights';

const RESOURCE_FILTERS = ['', 'document', 'skill', 'extension', 'recipe'] as const;

const RESOURCE_ICONS: Record<string, React.ReactNode> = {
  document: <FileText className="w-4 h-4" />,
  skill: <Zap className="w-4 h-4" />,
  extension: <Puzzle className="w-4 h-4" />,
  recipe: <BookOpen className="w-4 h-4" />,
};

const FILTER_LABEL_KEYS: Record<string, string> = {
  '': 'smartLog.filterAll',
  document: 'smartLog.filterDocument',
  skill: 'smartLog.filterSkill',
  extension: 'smartLog.filterExtension',
  recipe: 'smartLog.filterRecipe',
};

const SKELETON_COUNT = [1, 2, 3, 4, 5];

const AUTO_REFRESH_MS = 30_000;
const PAGE_SIZE = 20;
const BUILTIN_EXT_COUNT = BUILTIN_EXTENSIONS.length;
const BUILTIN_SKILL_COUNT = BUILTIN_SKILLS.length;

const TYPE_LABEL_KEYS: Record<string, string> = {
  builtin_extension: 'aiInsights.builtinExtensions',
  builtin_skill: 'aiInsights.builtinSkills',
  extension: 'smartLog.filterExtension',
  skill: 'smartLog.filterSkill',
};

interface InsightItem {
  id: string;
  type: string;
  name: string;
  ai_description: string;
  ai_description_lang: string;
  ai_described_at: string;
}

const BADGE_BASE = 'inline-flex items-center gap-1 text-xs px-1.5 py-0.5 rounded-full';
const BADGE_PENDING = `${BADGE_BASE} bg-[hsl(var(--primary)/0.1)] text-[hsl(var(--primary))]`;
const BADGE_COMPLETED = `${BADGE_BASE} bg-[hsl(var(--status-info-bg))] text-[hsl(var(--status-info-text))]`;
const BADGE_COMPLETED_GREEN = `${BADGE_BASE} bg-[hsl(var(--status-success-bg))] text-[hsl(var(--status-success-text))]`;
const BADGE_FAILED = `${BADGE_BASE} bg-[hsl(var(--muted))] text-[hsl(var(--muted-foreground))]`;

type SafetyRiskLevel = 'safe' | 'medium' | 'high' | null;

function extractSafetySection(text: string): string | null {
  const headingPatterns = ['**安全性与危险警告**', '安全性与危险警告', '**Safety & Risk Warnings**', 'Safety & Risk Warnings'];
  let start = -1;
  for (const pattern of headingPatterns) {
    start = text.indexOf(pattern);
    if (start !== -1) {
      start += pattern.length;
      break;
    }
  }
  if (start === -1) return null;

  const rest = text.slice(start);
  const nextHeading = rest.search(/\n\s*\*\*[^*\n]+\*\*/);
  return (nextHeading === -1 ? rest : rest.slice(0, nextHeading)).trim();
}

function detectSafetyRisk(text: string): SafetyRiskLevel {
  const section = extractSafetySection(text);
  if (!section) return null;

  const normalized = section.toLowerCase();
  const safeTerms = ['未发现明显', '没有明显', '无明显', 'no obvious'];
  const hasSafeConclusion = safeTerms.some((term) => normalized.includes(term.toLowerCase()));
  const strongRiskTerms = ['严重安全风险', '严重风险', '高风险', '后门', 'rm -rf', '~/.ssh', 'curl | bash', 'credential theft', 'backdoor'];
  if (hasSafeConclusion && !strongRiskTerms.some((term) => normalized.includes(term.toLowerCase()))) {
    return 'safe';
  }

  const highRiskTerms = [
    '严重安全风险',
    '严重风险',
    '高风险',
    '密钥',
    'api_key',
    'token',
    '外传',
    '上传',
    '窃取',
    '后门',
    'curl | bash',
    'curl -fssl',
    'rm -rf',
    '~/.ssh',
    'dangerous',
    'exfiltration',
    'credential theft',
    'backdoor',
  ];
  if (highRiskTerms.some((term) => normalized.includes(term.toLowerCase()))) {
    return 'high';
  }
  const mediumRiskTerms = ['需要人工确认', '权限变更', '命令执行', '外部访问', '风险', 'human review', 'permission', 'command execution', 'external access'];
  if (mediumRiskTerms.some((term) => normalized.includes(term.toLowerCase()))) {
    return 'medium';
  }
  if (hasSafeConclusion) {
    return 'safe';
  }
  return 'medium';
}

function safetyRiskLabelKey(level: SafetyRiskLevel): string {
  if (level === 'high') return 'aiInsights.safetyHigh';
  if (level === 'medium') return 'aiInsights.safetyMedium';
  if (level === 'safe') return 'aiInsights.safetySafe';
  return '';
}

function safetyRiskHintKey(level: SafetyRiskLevel): string {
  if (level === 'high') return 'aiInsights.safetyHighHint';
  if (level === 'medium') return 'aiInsights.safetyMediumHint';
  if (level === 'safe') return 'aiInsights.safetySafeHint';
  return '';
}

function safetyRiskClasses(level: SafetyRiskLevel): string {
  if (level === 'high') {
    return 'border-status-error/40 bg-[hsl(var(--status-error-bg))] text-[hsl(var(--status-error-text))]';
  }
  if (level === 'medium') {
    return 'border-status-warning/40 bg-[hsl(var(--status-warning-bg))] text-[hsl(var(--status-warning-text))]';
  }
  return 'border-status-success/40 bg-[hsl(var(--status-success-bg))] text-[hsl(var(--status-success-text))]';
}

function formatRelativeTime(dateStr: string, t: (key: string, opts?: Record<string, unknown>) => string): string {
  const diffMs = Date.now() - new Date(dateStr).getTime();
  const diffMin = Math.floor(diffMs / 60_000);
  const diffHour = Math.floor(diffMs / 3_600_000);
  const diffDay = Math.floor(diffMs / 86_400_000);

  if (diffMin < 1) return t('activity.justNow');
  if (diffMin < 60) return t('activity.minutesAgo', { count: diffMin });
  if (diffHour < 24) return t('activity.hoursAgo', { count: diffHour });
  return t('activity.daysAgo', { count: diffDay });
}

function ResourceIcon({ type }: { type: string }) {
  return (
    <span className="text-[hsl(var(--muted-foreground))]">
      {RESOURCE_ICONS[type] || <FileText className="w-4 h-4" />}
    </span>
  );
}

/** Shared hook for toggle-set expand/collapse state. */
function useToggleSet<T>(initialOpen: T[] = []) {
  const [openSet, setOpenSet] = useState<Set<T>>(() => new Set(initialOpen));
  const toggle = useCallback((key: T) => {
    setOpenSet(prev => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key); else next.add(key);
      return next;
    });
  }, []);
  const reset = useCallback((keys: T[]) => setOpenSet(new Set(keys)), []);
  return { openSet, toggle, reset, setOpenSet } as const;
}

/** Extract the first paragraph (up to first blank line) as a Markdown-safe summary. */
function extractSummary(text: string): { summary: string; hasMore: boolean } {
  const idx = text.indexOf('\n\n');
  if (idx !== -1 && idx <= 300) {
    return { summary: text.slice(0, idx), hasMore: idx < text.length - 2 };
  }
  const lines = text.split('\n');
  let len = 0;
  for (let i = 0; i < lines.length; i++) {
    len += lines[i].length + 1;
    if (len > 200 && i > 0) return { summary: lines.slice(0, i).join('\n'), hasMore: true };
  }
  return { summary: text, hasMore: false };
}

function TextBlock({ content, markdown }: { content: string; markdown?: boolean }) {
  return markdown
    ? <MarkdownContent content={content} className="text-sm" />
    : <div className="text-sm whitespace-pre-wrap">{content}</div>;
}

function ExpandableText({ text, maxLength = 200, markdown = false }: { text: string; maxLength?: number; markdown?: boolean }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);

  const { summary, hasMore } = useMemo(() => {
    if (markdown) return extractSummary(text);
    if (text.length <= maxLength) return { summary: text, hasMore: false };
    return { summary: text.slice(0, maxLength) + '...', hasMore: true };
  }, [text, maxLength, markdown]);

  if (!hasMore) return <TextBlock content={text} markdown={markdown} />;

  return (
    <div>
      <TextBlock content={expanded ? text : summary} markdown={markdown} />
      <button
        onClick={() => setExpanded(!expanded)}
        className="text-xs text-[hsl(var(--primary))] hover:underline mt-1 flex items-center gap-1"
      >
        {expanded ? <ChevronUp className="w-3 h-3" /> : <ChevronDown className="w-3 h-3" />}
        {expanded ? t('smartLog.collapseText') : t('smartLog.expandMore')}
      </button>
    </div>
  );
}

interface DocumentAnalysisPayload {
  status?: string;
  summary?: string;
  content_structure?: string[];
  key_points?: string[];
  file_observations?: string[];
  limitations?: string[];
}

function stringArray(value: unknown): string[] {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === 'string' && item.trim().length > 0)
    : [];
}

function parseDocumentAnalysisPayload(text: string): DocumentAnalysisPayload | null {
  try {
    const parsed = JSON.parse(text) as DocumentAnalysisPayload;
    if (!parsed || typeof parsed !== 'object') return null;

    if (typeof parsed.summary === 'string' && parsed.summary.trim().startsWith('{')) {
      try {
        const nested = JSON.parse(parsed.summary) as DocumentAnalysisPayload;
        if (nested && typeof nested === 'object' && typeof nested.summary === 'string') {
          return nested;
        }
      } catch {
        // Keep the outer payload when legacy model text only looks like JSON.
      }
    }

    if (
      typeof parsed.summary !== 'string' &&
      stringArray(parsed.content_structure).length === 0 &&
      stringArray(parsed.key_points).length === 0
    ) {
      return null;
    }
    return parsed;
  } catch {
    return null;
  }
}

function DocumentAnalysisSection({ title, items }: { title: string; items: string[] }) {
  if (items.length === 0) return null;
  return (
    <section className="space-y-1.5">
      <h4 className="text-xs font-semibold uppercase tracking-[0.08em] text-[hsl(var(--muted-foreground))]">
        {title}
      </h4>
      <ul className="space-y-1.5">
        {items.map((item, index) => (
          <li key={`${title}-${index}`} className="flex gap-2 text-sm leading-relaxed">
            <span className="mt-2 h-1.5 w-1.5 shrink-0 rounded-full bg-[hsl(var(--primary)/0.65)]" />
            <span>{item}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}

function StructuredDocumentAnalysis({ payload }: { payload: DocumentAnalysisPayload }) {
  const { t } = useTranslation();
  const summary = typeof payload.summary === 'string' ? payload.summary.trim() : '';
  const contentStructure = stringArray(payload.content_structure);
  const keyPoints = stringArray(payload.key_points);
  const fileObservations = stringArray(payload.file_observations);
  const limitations = stringArray(payload.limitations);

  return (
    <div className="space-y-4 pt-3">
      {summary && (
        <section className="rounded-md border border-[hsl(var(--border))] bg-[hsl(var(--muted)/0.22)] px-3 py-2.5">
          <h4 className="mb-1 text-xs font-semibold uppercase tracking-[0.08em] text-[hsl(var(--muted-foreground))]">
            {t('smartLog.analysisSummary')}
          </h4>
          <p className="text-sm leading-relaxed">{summary}</p>
        </section>
      )}
      <DocumentAnalysisSection title={t('smartLog.analysisStructure')} items={contentStructure} />
      <DocumentAnalysisSection title={t('smartLog.analysisKeyPoints')} items={keyPoints} />
      <DocumentAnalysisSection title={t('smartLog.analysisObservations')} items={fileObservations} />
      <DocumentAnalysisSection title={t('smartLog.analysisLimitations')} items={limitations} />
    </div>
  );
}

/** Parse markdown into sections by **bold headings** or ## headings, each independently collapsible. */
function SectionAccordion({ text }: { text: string }) {
  const { t } = useTranslation();
  const structuredAnalysis = useMemo(() => parseDocumentAnalysisPayload(text), [text]);
  const sections = useMemo(() => {
    const parts: { title: string; body: string }[] = [];
    const lines = text.split('\n');
    let currentTitle = '';
    let currentBody: string[] = [];

    for (const line of lines) {
      const m = line.trim().match(/^(?:\*\*(.+?)\*\*[：:]?\s*$|#{1,3}\s+(.+)$)/);
      if (m) {
        if (currentTitle || currentBody.length > 0) {
          parts.push({ title: currentTitle, body: currentBody.join('\n').trim() });
        }
        currentTitle = (m[1] ?? m[2]) || '';
        currentBody = [];
      } else {
        currentBody.push(line);
      }
    }
    if (currentTitle || currentBody.length > 0) {
      parts.push({ title: currentTitle, body: currentBody.join('\n').trim() });
    }
    return parts;
  }, [text]);

  const { openSet, toggle, reset } = useToggleSet<number>([0]);

  // Reset open state when text content changes (e.g. AI regeneration)
  useEffect(() => { reset([0]); }, [text, reset]);

  if (structuredAnalysis) {
    return <StructuredDocumentAnalysis payload={structuredAnalysis} />;
  }

  if (sections.length < 2) {
    return <ExpandableText text={text} markdown />;
  }

  return (
    <div className="space-y-1">
      <div className="flex justify-end">
        <button
          onClick={() => reset(openSet.size === sections.length ? [0] : sections.map((_, i) => i))}
          className="text-xs text-[hsl(var(--muted-foreground))] hover:text-[hsl(var(--foreground))] flex items-center gap-1"
        >
          <ChevronsUpDown className="w-3 h-3" />
          {openSet.size === sections.length ? t('smartLog.collapseAll') : t('smartLog.expandAll')}
        </button>
      </div>
      {sections.map((sec, i) => {
        if (!sec.title) {
          return sec.body ? <MarkdownContent key={i} content={sec.body} className="text-sm" /> : null;
        }
        const isOpen = openSet.has(i);
        return (
          <div key={i}>
            <button
              onClick={() => toggle(i)}
              className="flex items-center gap-1.5 w-full text-left py-1 text-sm font-medium hover:text-[hsl(var(--primary))] transition-colors"
            >
              <ChevronRight className={`w-3.5 h-3.5 shrink-0 transition-transform ${isOpen ? 'rotate-90' : ''}`} />
              {sec.title}
            </button>
            {isOpen && sec.body && (
              <div className="pl-5">
                <MarkdownContent content={sec.body} className="text-sm" />
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

function AnalysisStatusBadge({ status, variant, onClick }: {
  status: string | null;
  variant?: 'activity' | 'insight';
  onClick?: () => void;
}) {
  const { t } = useTranslation();
  if (!status) return null;

  if (status === 'pending') {
    return (
      <span className={BADGE_PENDING}>
        <Loader2 className="w-3 h-3 animate-spin" />
        {t('smartLog.analyzing')}
      </span>
    );
  }

  if (status === 'completed') {
    if (variant === 'activity' && onClick) {
      return (
        <button onClick={onClick} className={`${BADGE_COMPLETED} hover:opacity-80 transition-opacity`}>
          <Sparkles className="w-3 h-3" />
          {t('smartLog.analyzed')}
        </button>
      );
    }
    return (
      <span className={BADGE_COMPLETED_GREEN}>
        {t('smartLog.analyzed')}
      </span>
    );
  }

  if (status === 'failed' || status === 'cancelled') {
    return (
      <span className={BADGE_FAILED}>
        {t('smartLog.analysisFailed')}
      </span>
    );
  }

  return null;
}

function isPendingTimedOut(createdAt: string, timeoutMs = 120_000): boolean {
  return Date.now() - new Date(createdAt).getTime() > timeoutMs;
}

function ActivityCard({ log, onSwitchToInsights }: { log: SmartLogEntry; onSwitchToInsights: () => void }) {
  const { t } = useTranslation();
  const stillPending = log.aiSummaryStatus === 'pending' && !isPendingTimedOut(log.createdAt);

  return (
    <div className="flex items-start gap-3 p-3 rounded-lg border border-[hsl(var(--border))] hover:bg-[hsl(var(--muted))/0.3] transition-colors">
      <div className="w-8 h-8 rounded-full bg-[hsl(var(--muted))] flex items-center justify-center shrink-0 mt-0.5">
        <ResourceIcon type={log.resourceType} />
      </div>
      <div className="flex-1 min-w-0">
        {stillPending ? (
          <div className="flex items-center gap-2 text-sm text-[hsl(var(--muted-foreground))]">
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
            <span>{t('smartLog.generating')}</span>
          </div>
        ) : (
          <p className="text-sm leading-relaxed">
            {log.userName && (
              <span className="font-medium">{log.userName} </span>
            )}
            <span className={log.aiSummaryStatus === 'failed' || (!log.aiSummary && log.aiSummaryStatus === 'pending') ? 'text-[hsl(var(--muted-foreground))] italic' : ''}>
              {log.aiSummary || t('smartLog.failed')}
            </span>
          </p>
        )}
        <div className="flex items-center gap-2 mt-1">
          <p className="text-xs text-[hsl(var(--muted-foreground))]">
            {formatRelativeTime(log.createdAt, t)}
          </p>
          <AnalysisStatusBadge status={log.aiAnalysisStatus} variant="activity" onClick={onSwitchToInsights} />
        </div>
      </div>
    </div>
  );
}

function InsightCard({ log, onRetry, retrying }: { log: SmartLogEntry; onRetry?: (resourceId: string, prompt?: string) => void; retrying?: boolean }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [showPrompt, setShowPrompt] = useState(false);
  const [prompt, setPrompt] = useState('');
  const isFailed = log.aiAnalysisStatus === 'failed' || log.aiAnalysisStatus === 'cancelled';
  const isPending = log.aiAnalysisStatus === 'pending' || retrying;
  const canRetry = !isPending && onRetry && log.resourceId;

  const handleSubmitRetry = () => {
    if (!onRetry || !log.resourceId) return;
    onRetry(log.resourceId, prompt.trim() || undefined);
    setShowPrompt(false);
    setPrompt('');
  };

  return (
    <div className={`rounded-lg border bg-[hsl(var(--card))] overflow-hidden ${
      isFailed && !retrying ? 'opacity-60' : ''
    }`}>
      <button
        onClick={() => !isPending && setOpen(!open)}
        className="flex items-center gap-2 w-full text-left px-4 py-2.5 hover:bg-[hsl(var(--muted)/0.3)] transition-colors"
      >
        {isPending ? (
          <Loader2 className="w-3.5 h-3.5 animate-spin shrink-0 text-[hsl(var(--muted-foreground))]" />
        ) : (
          <ChevronRight className={`w-3.5 h-3.5 shrink-0 text-[hsl(var(--muted-foreground))] transition-transform ${open ? 'rotate-90' : ''}`} />
        )}
        <ResourceIcon type={log.resourceType} />
        <span className="font-medium text-sm truncate">{log.resourceName}</span>
        <AnalysisStatusBadge status={retrying ? 'pending' : log.aiAnalysisStatus} variant="insight" />
        {canRetry && (
          <button
            onClick={(e) => { e.stopPropagation(); setShowPrompt(!showPrompt); }}
            className="ml-1 p-1 rounded hover:bg-[hsl(var(--muted))] transition-colors"
            title={t('smartLog.retryAnalysis')}
          >
            <RefreshCw className="w-3.5 h-3.5 text-[hsl(var(--muted-foreground))]" />
          </button>
        )}
        <span className="ml-auto text-xs text-[hsl(var(--muted-foreground))] shrink-0">
          {formatRelativeTime(log.createdAt, t)}
        </span>
      </button>
      {showPrompt && !isPending && (
        <div className="px-4 py-3 border-t border-[hsl(var(--border)/0.5)] space-y-2">
          <textarea
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            placeholder={t('smartLog.retryPromptPlaceholder')}
            className="w-full text-sm rounded-md border border-[hsl(var(--border))] bg-[hsl(var(--background))] px-3 py-2 resize-none focus:outline-none focus:ring-1 focus:ring-[hsl(var(--primary))]"
            rows={2}
          />
          <div className="flex items-center gap-2 justify-end">
            <button onClick={() => { setShowPrompt(false); setPrompt(''); }} className="text-xs text-[hsl(var(--muted-foreground))] hover:text-[hsl(var(--foreground))]">
              {t('common.cancel')}
            </button>
            <Button size="sm" onClick={handleSubmitRetry}>
              {t('smartLog.retryAnalysis')}
            </Button>
          </div>
        </div>
      )}
      {open && (
        <div className="px-4 pb-3 border-t border-[hsl(var(--border)/0.5)]">
          {log.aiAnalysis ? (
            <SectionAccordion text={log.aiAnalysis} />
          ) : (
            <p className="text-sm text-[hsl(var(--muted-foreground))] italic pt-3">
              {log.aiSummary || t('smartLog.failed')}
            </p>
          )}
        </div>
      )}
    </div>
  );
}

function LogSkeleton() {
  return (
    <div className="space-y-3">
      {SKELETON_COUNT.map((i) => (
        <div key={i} className="flex items-start gap-3 p-3 rounded-lg border">
          <Skeleton className="w-8 h-8 rounded-full shrink-0" />
          <div className="flex-1 space-y-2">
            <Skeleton className="h-4 w-3/4" />
            <Skeleton className="h-3 w-1/4" />
          </div>
        </div>
      ))}
    </div>
  );
}

interface InsightSectionProps {
  title: string;
  items: InsightItem[];
  refreshingId: string | null;
  onRefresh: (item: InsightItem) => void;
  batchGenerating?: boolean;
  onBatchRefresh?: () => void;
}

function InsightSection({
  title,
  items,
  refreshingId,
  onRefresh,
  batchGenerating,
  onBatchRefresh,
}: InsightSectionProps) {
  const { t } = useTranslation();
  const { openSet, toggle, reset } = useToggleSet<string>();

  return (
    <div>
      <div className="flex items-center justify-between mb-3">
        <div className="flex items-center gap-3">
          <h3 className="text-lg font-semibold">{title}</h3>
          <span className="text-xs text-[hsl(var(--muted-foreground))]">{items.length}</span>
          {items.length > 1 && (
            <button
              onClick={() => reset(openSet.size === items.length ? [] : items.map(i => i.id))}
              className="text-xs text-[hsl(var(--muted-foreground))] hover:text-[hsl(var(--foreground))] flex items-center gap-1"
            >
              <ChevronsUpDown className="w-3 h-3" />
              {openSet.size === items.length ? t('smartLog.collapseAll') : t('smartLog.expandAll')}
            </button>
          )}
        </div>
        {onBatchRefresh && (
          <Button variant="outline" size="sm" onClick={onBatchRefresh} disabled={batchGenerating}>
            <RefreshCw className={`h-3.5 w-3.5 mr-1.5 ${batchGenerating ? 'animate-spin' : ''}`} />
            {t('aiInsights.refreshAll')}
          </Button>
        )}
      </div>
      <div className="space-y-1">
        {items.map((item) => {
          const isOpen = openSet.has(item.id);
          const safetyRisk = detectSafetyRisk(item.ai_description);
          return (
            <div
              key={item.id}
              className={`rounded-lg border bg-[hsl(var(--card))] overflow-hidden ${
                safetyRisk === 'high' ? 'border-status-error/35' : 'border-[hsl(var(--border))]'
              }`}
            >
              <button
                onClick={() => toggle(item.id)}
                className="flex items-center gap-2 w-full text-left px-4 py-2.5 hover:bg-[hsl(var(--muted)/0.3)] transition-colors"
              >
                <ChevronRight className={`w-3.5 h-3.5 shrink-0 text-[hsl(var(--muted-foreground))] transition-transform ${isOpen ? 'rotate-90' : ''}`} />
                <Sparkles className="h-3.5 w-3.5 shrink-0 text-status-warning-text" />
                <span className="font-medium text-sm truncate">{item.name}</span>
                <span className="text-xs px-1.5 py-0.5 rounded bg-[hsl(var(--muted))] text-[hsl(var(--muted-foreground))] shrink-0">
                  {t(TYPE_LABEL_KEYS[item.type] ?? 'smartLog.filterSkill')}
                </span>
                {safetyRisk ? (
                  <span className={`text-xs px-2 py-0.5 rounded-full border font-semibold shrink-0 ${safetyRiskClasses(safetyRisk)}`}>
                    {t(safetyRiskLabelKey(safetyRisk))}
                  </span>
                ) : null}
                <span className="ml-auto text-xs text-[hsl(var(--muted-foreground))] shrink-0">
                  {formatRelativeTime(item.ai_described_at, t)}
                </span>
              </button>
              {isOpen && (
                <div className="px-4 pt-3 pb-3 border-t border-[hsl(var(--border)/0.5)]">
                  {safetyRisk ? (
                    <div className={`mb-3 rounded-lg border px-3 py-2 text-sm font-medium ${safetyRiskClasses(safetyRisk)}`}>
                      {t(safetyRiskLabelKey(safetyRisk))}
                      <span className="ml-2 font-normal">
                        {t(safetyRiskHintKey(safetyRisk))}
                      </span>
                    </div>
                  ) : null}
                  <MarkdownContent content={item.ai_description} className="text-sm" />
                  <div className="flex items-center justify-between mt-2">
                    <div className="text-xs text-[hsl(var(--muted-foreground))]">
                      {t('aiInsights.generatedAt')}: {formatDateTime(item.ai_described_at)}
                    </div>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={(e) => { e.stopPropagation(); onRefresh(item); }}
                      disabled={refreshingId === item.id}
                      title={t('aiInsights.refresh')}
                    >
                      <RefreshCw className={`h-3.5 w-3.5 ${refreshingId === item.id ? 'animate-spin' : ''}`} />
                    </Button>
                  </div>
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

export function SmartLogTab({ teamId }: { teamId: string }) {
  const { t, i18n } = useTranslation();
  const navigate = useNavigate();
  const { isConversationMode, isMobileWorkspace } = useMobileInteractionMode();
  const [activeTab, setActiveTab] = useState<TabType>('activity');
  const [mobileFilterSheetOpen, setMobileFilterSheetOpen] = useState(false);
  const [logs, setLogs] = useState<SmartLogEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [filter, setFilter] = useState('');
  const [userFilter, setUserFilter] = useState('');
  const [showDeleted, setShowDeleted] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [page, setPage] = useState(1);
  const [totalPages, setTotalPages] = useState(0);

  // AI Insights state (builtin/extension/skill descriptions)
  const [aiInsights, setAiInsights] = useState<InsightItem[]>([]);
  const [insightsLoading, setInsightsLoading] = useState(false);
  const [retryingDocId, setRetryingDocId] = useState<string | null>(null);
  const [generatingKeys, setGeneratingKeys] = useState<Set<string>>(new Set());
  const [refreshingInsightId, setRefreshingInsightId] = useState<string | null>(null);
  const refreshingInsightIdRef = useRef<string | null>(null);
  const batchTriggered = useRef(false);
  const isConversationTaskMode = isConversationMode && isMobileWorkspace;

  const isGenerating = (key: string) => generatingKeys.has(key);
  const anyGenerating = generatingKeys.size > 0;

  const lang = i18n.language.substring(0, 2);

  const fetchLogs = useCallback(async (pageNum: number, append = false) => {
    try {
      if (!append) setLoading(true);
      const res = await apiClient.getSmartLogs(teamId, {
        resourceType: filter || undefined,
        userId: userFilter || undefined,
        page: pageNum,
        limit: PAGE_SIZE,
      });
      setLogs(prev => append ? [...prev, ...res.items] : res.items);
      setTotalPages(res.totalPages);
      setPage(pageNum);
    } catch {
      // silently fail
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, [teamId, filter, userFilter]);

  // Fetch AI insights (builtin extension descriptions etc.)
  const fetchAiInsights = useCallback(async () => {
    try {
      setInsightsLoading(true);
      const response = await apiClient.getAiInsights(teamId, lang);
      setAiInsights(response.insights);
      return response.insights;
    } catch {
      return [];
    } finally {
      setInsightsLoading(false);
    }
  }, [teamId, lang]);

  // Shared batch runner
  const runBatch = useCallback(async (key: string, apiFn: () => Promise<unknown>, silent: boolean) => {
    setGeneratingKeys(prev => new Set(prev).add(key));
    try {
      await apiFn();
      await fetchAiInsights();
    } catch (err) {
      if (!silent) console.warn(`Batch ${key} failed:`, err);
    } finally {
      setGeneratingKeys(prev => { const next = new Set(prev); next.delete(key); return next; });
    }
  }, [fetchAiInsights]);

  const runBatchBuiltinExts = useCallback(
    (silent: boolean) => runBatch('builtinExts', () => apiClient.describeBuiltinExtensionsBatch(teamId, lang), silent),
    [teamId, lang, runBatch],
  );
  const runBatchBuiltinSkills = useCallback(
    (silent: boolean) => runBatch('builtinSkills', () => apiClient.describeBuiltinSkillsBatch(teamId, lang), silent),
    [teamId, lang, runBatch],
  );
  const runBatchSkills = useCallback(
    (silent: boolean) => runBatch('skills', () => apiClient.describeSkillsBatch(teamId, lang), silent),
    [teamId, lang, runBatch],
  );
  const runBatchExts = useCallback(
    (silent: boolean) => runBatch('exts', () => apiClient.describeExtensionsBatch(teamId, lang), silent),
    [teamId, lang, runBatch],
  );

  useEffect(() => {
    fetchLogs(1);
  }, [fetchLogs]);

  // Load AI insights when insights tab is active
  useEffect(() => {
    if (activeTab !== 'insights') return;
    batchTriggered.current = false;
    fetchAiInsights().then(items => {
      if (batchTriggered.current) return;
      batchTriggered.current = true;
      const builtinExtCount = items.filter(i => i.type === 'builtin_extension').length;
      const builtinSkillCount = items.filter(i => i.type === 'builtin_skill').length;
      if (builtinExtCount < BUILTIN_EXT_COUNT) runBatchBuiltinExts(true);
      if (builtinSkillCount < BUILTIN_SKILL_COUNT) runBatchBuiltinSkills(true);
      runBatchSkills(true);
      runBatchExts(true);
    });
  }, [activeTab, fetchAiInsights, runBatchBuiltinExts, runBatchBuiltinSkills, runBatchSkills, runBatchExts]);

  // Auto-refresh: silently update the first page of results
  useEffect(() => {
    const interval = setInterval(() => {
      apiClient.getSmartLogs(teamId, {
        resourceType: filter || undefined,
        userId: userFilter || undefined,
        page: 1,
        limit: PAGE_SIZE,
      }).then(res => {
        setLogs(prev => prev.length <= PAGE_SIZE
          ? res.items
          : [...res.items, ...prev.slice(PAGE_SIZE)]
        );
        setTotalPages(res.totalPages);
      }).catch(() => {});
    }, AUTO_REFRESH_MS);
    return () => clearInterval(interval);
  }, [teamId, filter, userFilter]);

  const handleRefresh = useCallback(() => {
    setRefreshing(true);
    fetchLogs(1);
    if (activeTab === 'insights') fetchAiInsights();
  }, [fetchLogs, activeTab, fetchAiInsights]);

  const handleLoadMore = useCallback(() => {
    fetchLogs(page + 1, true);
  }, [fetchLogs, page]);

  const handleRefreshInsight = useCallback(async (item: InsightItem) => {
    if (refreshingInsightIdRef.current) return;
    refreshingInsightIdRef.current = item.id;
    setRefreshingInsightId(item.id);
    try {
      if (item.type === 'extension') {
        await apiClient.describeExtension(teamId, item.id, lang);
      } else if (item.type === 'builtin_extension') {
        const builtinExt = BUILTIN_EXTENSIONS.find(e => e.id === item.id);
        await apiClient.describeBuiltinExtension(teamId, {
          id: item.id,
          name: item.name,
          description: builtinExt?.description ?? '',
          is_platform: builtinExt?.isPlatform ?? false,
          lang,
        });
      } else if (item.type === 'builtin_skill') {
        const builtinSkill = BUILTIN_SKILLS.find(s => s.id === item.id);
        await apiClient.describeBuiltinSkill(teamId, {
          id: item.id,
          name: item.name,
          description: builtinSkill?.description ?? '',
          lang,
        });
      } else {
        await apiClient.describeSkill(teamId, item.id, lang);
      }
      await fetchAiInsights();
    } catch {
      // silently fail
    } finally {
      refreshingInsightIdRef.current = null;
      setRefreshingInsightId(null);
    }
  }, [teamId, lang, fetchAiInsights]);

  const uniqueUsers = useMemo(() => {
    const map = new Map<string, string>();
    for (const log of logs) {
      if (log.userId && log.userName) map.set(log.userId, log.userName);
    }
    return Array.from(map.entries()).sort((a, b) => a[1].localeCompare(b[1]));
  }, [logs]);

  const activityLogs = useMemo(
    () => logs.filter(log => showDeleted || log.action !== 'delete'),
    [logs, showDeleted],
  );

  const insightLogs = useMemo(
    () => logs.filter(log => log.aiAnalysis || ['pending', 'failed', 'cancelled'].includes(log.aiAnalysisStatus ?? '')),
    [logs],
  );

  const handleRetryAnalysis = useCallback(async (resourceId: string, prompt?: string) => {
    setRetryingDocId(resourceId);
    try {
      await documentApi.retryAnalysis(teamId, resourceId, prompt);
    } catch (e) {
      console.warn('Retry analysis failed:', e);
    } finally {
      setRetryingDocId(null);
    }
  }, [teamId]);

  // Categorize AI insights in a single pass
  const { builtinInsights, builtinSkillInsights, extensionInsights, skillInsights } = useMemo(() => {
    const builtin: InsightItem[] = [];
    const builtinSkills: InsightItem[] = [];
    const extensions: InsightItem[] = [];
    const skills: InsightItem[] = [];
    for (const item of aiInsights) {
      if (item.type === 'builtin_extension') builtin.push(item);
      else if (item.type === 'builtin_skill') builtinSkills.push(item);
      else if (item.type === 'extension') extensions.push(item);
      else if (item.type === 'skill') skills.push(item);
    }
    return { builtinInsights: builtin, builtinSkillInsights: builtinSkills, extensionInsights: extensions, skillInsights: skills };
  }, [aiInsights]);

  const showInsightsEmpty = useMemo(() => {
    if (anyGenerating) return false;
    const hasLogs = (!filter || filter === 'document') && insightLogs.length > 0;
    if (hasLogs) return false;
    if (!filter) return aiInsights.length === 0;
    if (filter === 'extension') return builtinInsights.length === 0 && extensionInsights.length === 0;
    if (filter === 'skill') return builtinSkillInsights.length === 0 && skillInsights.length === 0;
    return true;
  }, [filter, aiInsights, builtinInsights, extensionInsights, builtinSkillInsights, skillInsights, insightLogs, anyGenerating]);

  const tabClass = (tab: TabType): string => {
    const base = 'px-3 py-1.5 text-sm rounded-full transition-colors';
    if (activeTab === tab) return `${base} bg-[hsl(var(--ui-surface-panel-strong))/0.96] font-medium text-[hsl(var(--foreground))] shadow-[0_6px_16px_hsl(var(--ui-shadow)/0.08)]`;
    return `${base} text-[hsl(var(--muted-foreground))] hover:text-[hsl(var(--foreground))]`;
  };

  const tabSwitcher = (
    <div className="inline-flex items-center gap-1 rounded-full border border-[hsl(var(--ui-line-soft))/0.66] bg-[hsl(var(--ui-surface-panel-muted))/0.8] p-1">
      <button onClick={() => setActiveTab('activity')} className={tabClass('activity')}>
        {t('smartLog.tabActivity')}
      </button>
      <button onClick={() => setActiveTab('insights')} className={`${tabClass('insights')} flex items-center gap-1.5`}>
        <Sparkles className="w-3.5 h-3.5" />
        {t('smartLog.tabInsights')}
      </button>
    </div>
  );

  const filterControls = (
    <div className="flex items-center gap-2 flex-wrap">
      <span className="mr-1 text-xs text-[hsl(var(--muted-foreground))]">{t('smartLog.filterByResource')}</span>
      {RESOURCE_FILTERS.map((f) => (
        <Button
          key={f}
          variant={filter === f ? 'default' : 'outline'}
          size="sm"
          onClick={() => setFilter(f)}
        >
          {t(FILTER_LABEL_KEYS[f])}
        </Button>
      ))}
      {uniqueUsers.length > 1 && (
        <Select value={userFilter || '__all__'} onValueChange={v => setUserFilter(v === '__all__' ? '' : v)}>
          <SelectTrigger className="h-8 w-36">
            <SelectValue placeholder={t('smartLog.filterByUser')} />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="__all__">{t('smartLog.allMembers')}</SelectItem>
            {uniqueUsers.map(([uid, name]) => (
              <SelectItem key={uid} value={uid}>{name}</SelectItem>
            ))}
          </SelectContent>
        </Select>
      )}
      {activeTab === 'activity' && (
        <button
          onClick={() => setShowDeleted(!showDeleted)}
          className={`ml-auto flex items-center gap-1.5 rounded-full border px-2.5 py-1.5 text-xs transition-colors ${
            showDeleted
              ? 'border-[hsl(var(--primary))] bg-[hsl(var(--primary)/0.08)] text-[hsl(var(--primary))]'
              : 'border-[hsl(var(--border))] text-[hsl(var(--muted-foreground))] hover:text-[hsl(var(--foreground))]'
          }`}
        >
          {showDeleted ? <Eye className="w-3 h-3" /> : <EyeOff className="w-3 h-3" />}
          {t('smartLog.showDeleted')}
        </button>
      )}
    </div>
  );

  const smartLogContent = (
    <div className="space-y-4">
      {!isConversationTaskMode && (
        <>
          <div className="flex items-center justify-between">
            {tabSwitcher}
            <Button variant="outline" size="sm" onClick={handleRefresh} disabled={refreshing}>
              <RefreshCw className={`w-4 h-4 mr-1.5 ${refreshing ? 'animate-spin' : ''}`} />
              {t('smartLog.refresh')}
            </Button>
          </div>
          {filterControls}
        </>
      )}

      {/* Content */}
      {activeTab === 'activity' ? (
        loading ? <LogSkeleton /> : activityLogs.length === 0 ? (
          <div className="text-center py-12 text-[hsl(var(--muted-foreground))]">
            {t('smartLog.noLogs')}
          </div>
        ) : (
          <div className="space-y-2">
            {activityLogs.map((log) => (
              <ActivityCard
                key={log.id}
                log={log}
                onSwitchToInsights={() => setActiveTab('insights')}
              />
            ))}
            {page < totalPages && (
              <div className="text-center pt-2">
                <Button variant="outline" size="sm" onClick={handleLoadMore}>
                  {t('smartLog.loadMore')}
                </Button>
              </div>
            )}
          </div>
        )
      ) : (
        /* Insights tab: AI descriptions + smart log AI analyses */
        (loading && insightsLoading) ? <LogSkeleton /> : (
          <div className="space-y-6">
            {anyGenerating && (
              <div className="ui-subtle-panel flex items-center gap-2 px-4 py-3 text-sm ui-secondary-text">
                <Loader2 className="h-4 w-4 animate-spin" />
                {t('aiInsights.batchGenerating')}
              </div>
            )}

            {/* Builtin extension AI descriptions (show under 'extension' or 'all') */}
            {(!filter || filter === 'extension') && builtinInsights.length > 0 && (
              <InsightSection
                title={t('aiInsights.builtinExtensions')}
                items={builtinInsights}
                refreshingId={refreshingInsightId}
                onRefresh={handleRefreshInsight}
                batchGenerating={isGenerating('builtinExts')}
                onBatchRefresh={() => runBatchBuiltinExts(false)}
              />
            )}

            {/* Shared extension AI descriptions (show under 'extension' or 'all') */}
            {(!filter || filter === 'extension') && extensionInsights.length > 0 && (
              <InsightSection
                title={t('aiInsights.extensions')}
                items={extensionInsights}
                refreshingId={refreshingInsightId}
                onRefresh={handleRefreshInsight}
                batchGenerating={isGenerating('exts')}
                onBatchRefresh={() => runBatchExts(false)}
              />
            )}

            {/* Builtin skill AI descriptions (show under 'skill' or 'all') */}
            {(!filter || filter === 'skill') && builtinSkillInsights.length > 0 && (
              <InsightSection
                title={t('aiInsights.builtinSkills')}
                items={builtinSkillInsights}
                refreshingId={refreshingInsightId}
                onRefresh={handleRefreshInsight}
                batchGenerating={isGenerating('builtinSkills')}
                onBatchRefresh={() => runBatchBuiltinSkills(false)}
              />
            )}

            {/* Skill AI descriptions (show under 'skill' or 'all') */}
            {(!filter || filter === 'skill') && skillInsights.length > 0 && (
              <InsightSection
                title={t('aiInsights.skills')}
                items={skillInsights}
                refreshingId={refreshingInsightId}
                onRefresh={handleRefreshInsight}
                batchGenerating={isGenerating('skills')}
                onBatchRefresh={() => runBatchSkills(false)}
              />
            )}

            {/* Smart log AI analyses (document analyses etc.) - show under 'document' or 'all') */}
            {(!filter || filter === 'document') && insightLogs.length > 0 && (
              <div>
                <h3 className="text-lg font-semibold mb-3">{t('smartLog.tabDocAnalysis')}</h3>
                <div className="space-y-1">
                  {insightLogs.map((log) => (
                    <InsightCard key={log.id} log={log} onRetry={handleRetryAnalysis} retrying={retryingDocId === log.resourceId} />
                  ))}
                </div>
              </div>
            )}

            {/* Empty state */}
            {showInsightsEmpty && (
              <div className="text-center py-12">
                <Sparkles className="h-12 w-12 mx-auto mb-4 text-[hsl(var(--muted-foreground))]" />
                <p className="text-[hsl(var(--muted-foreground))]">
                  {t('smartLog.noInsights')}
                </p>
              </div>
            )}

            {page < totalPages && (
              <div className="text-center pt-2">
                <Button variant="outline" size="sm" onClick={handleLoadMore}>
                  {t('smartLog.loadMore')}
                </Button>
              </div>
            )}
          </div>
        )
      )}
    </div>
  );

  if (isConversationTaskMode) {
    return (
      <>
        <MobileWorkspaceShell
          summary={(
            <ContextSummaryBar
              eyebrow={t('teamNav.smartLog', '智能日志')}
              title={t('smartLog.title', '日志与洞察')}
              description={t(
                'smartLog.mobileConversationDescription',
                '移动端优先查看活动与洞察结果，需要更多筛选时再展开抽屉，处理完再回到对话继续协同。',
              )}
              metrics={[
                { label: t('smartLog.summaryTab', '当前标签'), value: activeTab === 'activity' ? t('smartLog.tabActivity') : t('smartLog.tabInsights') },
                { label: t('smartLog.summaryActivity', '活动数'), value: activityLogs.length },
                { label: t('smartLog.summaryInsights', '洞察数'), value: insightLogs.length + aiInsights.length },
                { label: t('smartLog.summaryPending', '处理中'), value: generatingKeys.size },
              ]}
            />
          )}
          quickActions={(
            <div className="grid grid-cols-2 gap-2">
              <Button variant="outline" className="h-11 justify-start" onClick={() => setActiveTab('activity')}>
                <RefreshCw className="mr-2 h-4 w-4" />
                {t('smartLog.tabActivity')}
              </Button>
              <Button variant="outline" className="h-11 justify-start" onClick={() => setActiveTab('insights')}>
                <Sparkles className="mr-2 h-4 w-4" />
                {t('smartLog.tabInsights')}
              </Button>
              <Button variant="outline" className="h-11 justify-start" onClick={() => setMobileFilterSheetOpen(true)}>
                <SlidersHorizontal className="mr-2 h-4 w-4" />
                {t('smartLog.quickFilters', '筛选与视角')}
              </Button>
              <Button
                variant="outline"
                className="h-11 justify-start"
                onClick={() => navigate(`/teams/${teamId}?section=collaboration`)}
              >
                <MessageSquareText className="mr-2 h-4 w-4" />
                {t('smartLog.quickChat', '进入智能协作')}
              </Button>
            </div>
          )}
        >
          <ManagementRail
            title={activeTab === 'activity' ? t('smartLog.tabActivity') : t('smartLog.tabInsights')}
            description={t(
              'smartLog.mobileConversationRail',
              '日志与洞察内容继续沿用现有任务流，只把高频切换和筛选提升到统一移动交互层。',
            )}
            action={(
              <Button variant="outline" size="sm" onClick={handleRefresh} disabled={refreshing}>
                <RefreshCw className={`mr-1.5 h-4 w-4 ${refreshing ? 'animate-spin' : ''}`} />
                {t('smartLog.refresh')}
              </Button>
            )}
          >
            {smartLogContent}
          </ManagementRail>
        </MobileWorkspaceShell>
        <BottomSheetPanel
          open={mobileFilterSheetOpen}
          onOpenChange={setMobileFilterSheetOpen}
          title={t('smartLog.quickFilters', '筛选与视角')}
          description={t('smartLog.filterHint', '选择资源类型、成员范围，以及是否显示删除事件。')}
        >
          <div className="space-y-4">
            {tabSwitcher}
            {filterControls}
            <div className="flex gap-2">
              <Button variant="outline" className="flex-1" onClick={() => setMobileFilterSheetOpen(false)}>
                {t('common.confirm', '确认')}
              </Button>
              <Button
                variant="ghost"
                className="flex-1"
                onClick={() => {
                  setFilter('');
                  setUserFilter('');
                  setShowDeleted(false);
                }}
              >
                {t('common.reset', '重置')}
              </Button>
            </div>
          </div>
        </BottomSheetPanel>
      </>
    );
  }

  return smartLogContent;
}
