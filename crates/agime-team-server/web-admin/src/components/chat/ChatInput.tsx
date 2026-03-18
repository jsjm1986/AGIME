import { useState, useRef, useCallback, useEffect } from 'react';
import { ChevronUp, Send, Sparkles, Square } from 'lucide-react';
import { Button } from '../ui/button';
import { useTranslation } from 'react-i18next';

export interface ChatInputComposeRequest {
  id: string;
  text: string;
  autoSend?: boolean;
}

export interface ChatInputQuickAction {
  key: string;
  label: string;
  description?: string;
  onSelect: () => void;
}

export interface ChatInputQuickActionGroup {
  key: string;
  label: string;
  actions: ChatInputQuickAction[];
}

interface ChatInputProps {
  onSend: (content: string) => void;
  onStop: () => void;
  isProcessing: boolean;
  disabled?: boolean;
  canSendEmpty?: boolean;
  composeRequest?: ChatInputComposeRequest | null;
  quickActionGroups?: ChatInputQuickActionGroup[];
  onFocusChange?: (focused: boolean) => void;
  onContentChange?: (content: string) => void;
  onComposeApplied?: (id: string) => void;
}

export function ChatInput({
  onSend,
  onStop,
  isProcessing,
  disabled,
  canSendEmpty = false,
  composeRequest,
  quickActionGroups = [],
  onFocusChange,
  onContentChange,
  onComposeApplied,
}: ChatInputProps) {
  const [input, setInput] = useState('');
  const [quickActionsOpen, setQuickActionsOpen] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const quickActionsRef = useRef<HTMLDivElement>(null);
  const blurTimerRef = useRef<number | null>(null);
  const lastComposeIdRef = useRef<string | null>(null);
  const pendingAutoSendRef = useRef<{ id: string; text: string } | null>(null);
  const { t } = useTranslation();
  const visibleQuickActionGroups = quickActionGroups.filter((group) => group.actions.length > 0);

  const handleSend = useCallback(() => {
    const trimmed = input.trim();
    if ((!trimmed && !canSendEmpty) || isProcessing || disabled) return;
    onSend(trimmed);
    setInput('');
    onContentChange?.('');
    if (textareaRef.current) {
      textareaRef.current.style.height = 'auto';
    }
  }, [canSendEmpty, disabled, input, isProcessing, onContentChange, onSend]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const handleInput = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    setInput(e.target.value);
    onContentChange?.(e.target.value);
    // Auto-resize
    const el = e.target;
    el.style.height = 'auto';
    el.style.height = Math.min(el.scrollHeight, 200) + 'px';
  };

  useEffect(() => {
    if (!composeRequest || !composeRequest.id || lastComposeIdRef.current === composeRequest.id) {
      return;
    }
    lastComposeIdRef.current = composeRequest.id;
    onComposeApplied?.(composeRequest.id);
    const rawText = composeRequest.text || '';
    const text = rawText.trim();
    if (!text) {
      setInput('');
      pendingAutoSendRef.current = null;
      onContentChange?.('');
      if (textareaRef.current) {
        textareaRef.current.style.height = 'auto';
        textareaRef.current.focus();
      }
      return;
    }
    if (composeRequest.autoSend && !isProcessing && !disabled) {
      onSend(text);
      setInput('');
      onContentChange?.('');
      pendingAutoSendRef.current = null;
      if (textareaRef.current) {
        textareaRef.current.style.height = 'auto';
      }
      return;
    }
    if (composeRequest.autoSend) {
      pendingAutoSendRef.current = { id: composeRequest.id, text };
    }
    setInput(rawText);
    onContentChange?.(rawText);
    if (textareaRef.current) {
      textareaRef.current.style.height = 'auto';
      textareaRef.current.style.height = Math.min(textareaRef.current.scrollHeight, 200) + 'px';
      textareaRef.current.focus();
    }
  }, [composeRequest, disabled, isProcessing, onContentChange, onSend]);

  useEffect(() => {
    const pending = pendingAutoSendRef.current;
    if (!pending || isProcessing || disabled) return;
    const current = input.trim();
    if (!current || current !== pending.text.trim()) return;
    onSend(current);
    pendingAutoSendRef.current = null;
    setInput('');
    onContentChange?.('');
    if (textareaRef.current) {
      textareaRef.current.style.height = 'auto';
    }
  }, [disabled, input, isProcessing, onContentChange, onSend]);

  useEffect(() => {
    if (!quickActionsOpen) return;
    const handlePointerDown = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (target && quickActionsRef.current?.contains(target)) {
        return;
      }
      setQuickActionsOpen(false);
    };
    document.addEventListener('mousedown', handlePointerDown);
    return () => document.removeEventListener('mousedown', handlePointerDown);
  }, [quickActionsOpen]);

  useEffect(() => {
    return () => {
      if (blurTimerRef.current !== null) {
        window.clearTimeout(blurTimerRef.current);
      }
    };
  }, []);

  return (
    <div className="flex w-full min-w-0 items-end gap-1.5 bg-background px-2.5 py-2 sm:gap-2 sm:p-4">
      <textarea
        ref={textareaRef}
        value={input}
        onChange={handleInput}
        onKeyDown={handleKeyDown}
        onFocus={() => {
          if (blurTimerRef.current !== null) {
            window.clearTimeout(blurTimerRef.current);
            blurTimerRef.current = null;
          }
          onFocusChange?.(true);
        }}
        onBlur={() => {
          if (blurTimerRef.current !== null) {
            window.clearTimeout(blurTimerRef.current);
          }
          blurTimerRef.current = window.setTimeout(() => {
            onFocusChange?.(false);
            blurTimerRef.current = null;
          }, 90);
        }}
        placeholder={t('chat.inputPlaceholder', 'Type a message...')}
        disabled={disabled}
        rows={1}
        className="min-w-0 flex-1 resize-none rounded-[14px] border bg-background px-3 py-2 text-[12px]
          focus:outline-none focus:ring-2 focus:ring-ring min-h-[38px] max-h-[200px] sm:text-[13px] sm:min-h-[40px]"
      />
      <div className="flex shrink-0 items-end gap-1.5">
        {isProcessing ? (
          <Button
            variant="destructive"
            size="icon"
            onClick={onStop}
            title={t('chat.stopGenerating', 'Stop generating')}
            className="h-9 w-9 rounded-[12px] sm:h-10 sm:w-10"
          >
            <Square className="h-4 w-4" />
          </Button>
        ) : (
          <Button
            size="icon"
            onClick={handleSend}
            disabled={(!input.trim() && !canSendEmpty) || disabled}
            aria-label={t('chat.send', 'Send')}
            className="h-9 w-9 rounded-[12px] sm:h-10 sm:w-10"
          >
            <Send className="h-4 w-4" />
          </Button>
        )}
        {visibleQuickActionGroups.length > 0 && (
          <div ref={quickActionsRef} className="relative">
            <button
              type="button"
              onClick={() => setQuickActionsOpen((value) => !value)}
              className={`inline-flex h-9 w-9 items-center justify-center rounded-[12px] border transition-colors sm:h-10 sm:w-10 ${
                quickActionsOpen
                  ? 'border-primary/40 bg-primary/8 text-primary shadow-sm'
                  : 'border-border/60 bg-background text-muted-foreground hover:border-primary/25 hover:bg-muted/35 hover:text-foreground'
              }`}
              aria-expanded={quickActionsOpen}
              aria-label={t('chat.quickActions', '快捷指令')}
              title={t('chat.quickActions', '快捷指令')}
            >
              <Sparkles className="h-4 w-4" />
            </button>
            {quickActionsOpen && (
              <div className="absolute bottom-full right-0 z-20 mb-2 flex max-h-[min(72vh,560px)] w-[min(320px,calc(100vw-1rem))] flex-col overflow-hidden rounded-[18px] border border-border/70 bg-background/98 p-2 shadow-xl backdrop-blur sm:w-[336px]">
                <div className="border-b border-border/60 px-2 pb-2">
                  <div className="flex items-center gap-2">
                    <Sparkles className="h-4 w-4 text-primary" />
                    <p className="text-[12px] font-medium text-foreground">{t('chat.quickActions', '快捷指令')}</p>
                  </div>
                  <p className="mt-1 line-clamp-2 text-[10px] leading-4 text-muted-foreground">
                    {t('chat.quickActionsHint', '点击后先填入输入框，你确认后再发送，适合快速创建、审查和治理。')}
                  </p>
                </div>
                <div className="mt-2 flex-1 space-y-2.5 overflow-y-auto pr-1">
                  {visibleQuickActionGroups.map((group) => (
                    <div key={group.key} className="space-y-1.5">
                      <p className="px-2 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                        {group.label}
                      </p>
                      <div className="space-y-1.5">
                        {group.actions.map((action) => (
                          <button
                            key={action.key}
                            type="button"
                            className="w-full rounded-[14px] border border-border/60 px-3 py-2 text-left transition-colors hover:border-primary/40 hover:bg-muted/40"
                            onClick={() => {
                              action.onSelect();
                              setQuickActionsOpen(false);
                            }}
                          >
                            <div className="text-[12px] font-medium text-foreground">{action.label}</div>
                            {action.description && (
                              <div className="mt-0.5 line-clamp-2 text-[10px] leading-4 text-muted-foreground">
                                {action.description}
                              </div>
                            )}
                          </button>
                        ))}
                      </div>
                    </div>
                  ))}
                </div>
                <div className="mt-2 flex justify-end">
                  <button
                    type="button"
                    className="inline-flex items-center gap-1 px-2 text-[11px] text-muted-foreground hover:text-foreground"
                    onClick={() => setQuickActionsOpen(false)}
                  >
                    {t('common.close', '关闭')}
                    <ChevronUp className="h-3.5 w-3.5" />
                  </button>
                </div>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
