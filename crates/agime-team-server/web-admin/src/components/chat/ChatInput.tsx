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
  composeRequest?: ChatInputComposeRequest | null;
  quickActionGroups?: ChatInputQuickActionGroup[];
}

export function ChatInput({
  onSend,
  onStop,
  isProcessing,
  disabled,
  composeRequest,
  quickActionGroups = [],
}: ChatInputProps) {
  const [input, setInput] = useState('');
  const [quickActionsOpen, setQuickActionsOpen] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const quickActionsRef = useRef<HTMLDivElement>(null);
  const lastComposeIdRef = useRef<string | null>(null);
  const pendingAutoSendRef = useRef<{ id: string; text: string } | null>(null);
  const { t } = useTranslation();
  const visibleQuickActionGroups = quickActionGroups.filter((group) => group.actions.length > 0);

  const handleSend = useCallback(() => {
    const trimmed = input.trim();
    if (!trimmed || isProcessing || disabled) return;
    onSend(trimmed);
    setInput('');
    if (textareaRef.current) {
      textareaRef.current.style.height = 'auto';
    }
  }, [input, isProcessing, disabled, onSend]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const handleInput = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    setInput(e.target.value);
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
    const text = (composeRequest.text || '').trim();
    if (!text) return;
    if (composeRequest.autoSend && !isProcessing && !disabled) {
      onSend(text);
      setInput('');
      pendingAutoSendRef.current = null;
      if (textareaRef.current) {
        textareaRef.current.style.height = 'auto';
      }
      return;
    }
    if (composeRequest.autoSend) {
      pendingAutoSendRef.current = { id: composeRequest.id, text };
    }
    setInput(text);
    if (textareaRef.current) {
      textareaRef.current.style.height = 'auto';
      textareaRef.current.style.height = Math.min(textareaRef.current.scrollHeight, 200) + 'px';
      textareaRef.current.focus();
    }
  }, [composeRequest, disabled, isProcessing, onSend]);

  useEffect(() => {
    const pending = pendingAutoSendRef.current;
    if (!pending || isProcessing || disabled) return;
    const current = input.trim();
    if (!current || current !== pending.text.trim()) return;
    onSend(current);
    pendingAutoSendRef.current = null;
    setInput('');
    if (textareaRef.current) {
      textareaRef.current.style.height = 'auto';
    }
  }, [disabled, input, isProcessing, onSend]);

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

  return (
    <div className="flex items-end gap-2 p-4 bg-background">
      <textarea
        ref={textareaRef}
        value={input}
        onChange={handleInput}
        onKeyDown={handleKeyDown}
        placeholder={t('chat.inputPlaceholder', 'Type a message...')}
        disabled={disabled}
        rows={1}
        className="flex-1 resize-none rounded-md border bg-background px-3 py-2 text-[13px]
          focus:outline-none focus:ring-2 focus:ring-ring min-h-[40px] max-h-[200px]"
      />
      <div className="flex items-end gap-1.5 shrink-0">
        {isProcessing ? (
          <Button
            variant="destructive"
            size="icon"
            onClick={onStop}
            title={t('chat.stopGenerating', 'Stop generating')}
          >
            <Square className="h-4 w-4" />
          </Button>
        ) : (
          <Button
            size="icon"
            onClick={handleSend}
            disabled={!input.trim() || disabled}
            aria-label={t('chat.send', 'Send')}
          >
            <Send className="h-4 w-4" />
          </Button>
        )}
        {visibleQuickActionGroups.length > 0 && (
          <div ref={quickActionsRef} className="relative">
            <button
              type="button"
              onClick={() => setQuickActionsOpen((value) => !value)}
              className={`inline-flex h-9 w-9 items-center justify-center rounded-lg border transition-colors ${
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
              <div className="absolute bottom-full right-0 z-20 mb-2 flex max-h-[min(72vh,560px)] w-[336px] flex-col overflow-hidden rounded-xl border border-border/70 bg-background/98 p-2 shadow-xl backdrop-blur">
                <div className="border-b border-border/60 px-2 pb-2">
                  <div className="flex items-center gap-2">
                    <Sparkles className="h-4 w-4 text-primary" />
                    <p className="text-[12px] font-medium text-foreground">{t('chat.quickActions', '快捷指令')}</p>
                  </div>
                  <p className="mt-1 text-[11px] leading-4 text-muted-foreground">
                    {t('chat.quickActionsHint', '点击后先填入输入框，你确认后再发送，适合快速创建、审查和治理。')}
                  </p>
                </div>
                <div className="mt-2 flex-1 space-y-3 overflow-y-auto pr-1">
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
                            className="w-full rounded-lg border border-border/60 px-3 py-2 text-left transition-colors hover:border-primary/40 hover:bg-muted/40"
                            onClick={() => {
                              action.onSelect();
                              setQuickActionsOpen(false);
                            }}
                          >
                            <div className="text-[12px] font-medium text-foreground">{action.label}</div>
                            {action.description && (
                              <div className="mt-0.5 text-[11px] leading-4 text-muted-foreground">
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
