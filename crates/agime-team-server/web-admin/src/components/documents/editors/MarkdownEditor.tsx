import { useCallback } from 'react';
import Editor from '@monaco-editor/react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

interface MarkdownEditorProps {
  value: string;
  onChange: (value: string) => void;
}

function useSystemTheme(): 'vs-dark' | 'light' {
  if (typeof window === 'undefined') return 'light';
  return document.documentElement.classList.contains('dark') ? 'vs-dark' : 'light';
}

export function MarkdownEditor({ value, onChange }: MarkdownEditorProps) {
  const theme = useSystemTheme();

  const handleChange = useCallback(
    (val: string | undefined) => {
      onChange(val ?? '');
    },
    [onChange],
  );

  return (
    <div className="flex h-full">
      {/* Editor pane */}
      <div className="flex-1 flex flex-col border-r">
        <div className="px-3 py-1 text-xs text-muted-foreground border-b bg-muted/20">
          Markdown
        </div>
        <div className="flex-1">
          <Editor
            language="markdown"
            theme={theme}
            value={value}
            onChange={handleChange}
            options={{
              minimap: { enabled: false },
              fontSize: 14,
              wordWrap: 'on',
              scrollBeyondLastLine: false,
              automaticLayout: true,
              tabSize: 2,
            }}
          />
        </div>
      </div>
      {/* Preview pane */}
      <div className="flex-1 flex flex-col">
        <div className="px-3 py-1 text-xs text-muted-foreground border-b bg-muted/20">
          Preview
        </div>
        <div className="flex-1 overflow-auto p-4 prose prose-sm dark:prose-invert max-w-none">
          <ReactMarkdown remarkPlugins={[remarkGfm]}>{value}</ReactMarkdown>
        </div>
      </div>
    </div>
  );
}
