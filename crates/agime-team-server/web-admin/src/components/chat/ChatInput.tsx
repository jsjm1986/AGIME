import { useState, useRef, useCallback, useEffect } from 'react';
import { Send, Square } from 'lucide-react';
import { Button } from '../ui/button';
import { useTranslation } from 'react-i18next';

export interface ChatInputComposeRequest {
  id: string;
  text: string;
  autoSend?: boolean;
}

interface ChatInputProps {
  onSend: (content: string) => void;
  onStop: () => void;
  isProcessing: boolean;
  disabled?: boolean;
  composeRequest?: ChatInputComposeRequest | null;
}

export function ChatInput({
  onSend,
  onStop,
  isProcessing,
  disabled,
  composeRequest,
}: ChatInputProps) {
  const [input, setInput] = useState('');
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const lastComposeIdRef = useRef<string | null>(null);
  const pendingAutoSendRef = useRef<{ id: string; text: string } | null>(null);
  const { t } = useTranslation();

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
    </div>
  );
}
