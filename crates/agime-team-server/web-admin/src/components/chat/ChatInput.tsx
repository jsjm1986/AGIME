import { useState, useRef, useCallback } from 'react';
import { Send, Square } from 'lucide-react';
import { Button } from '../ui/button';
import { useTranslation } from 'react-i18next';

interface ChatInputProps {
  onSend: (content: string) => void;
  onStop: () => void;
  isProcessing: boolean;
  disabled?: boolean;
}

export function ChatInput({ onSend, onStop, isProcessing, disabled }: ChatInputProps) {
  const [input, setInput] = useState('');
  const textareaRef = useRef<HTMLTextAreaElement>(null);
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
        className="flex-1 resize-none rounded-md border bg-background px-3 py-2 text-sm
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
