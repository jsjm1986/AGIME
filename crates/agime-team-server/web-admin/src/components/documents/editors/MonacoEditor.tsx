import { useCallback } from 'react';
import Editor from '@monaco-editor/react';

interface MonacoEditorWrapperProps {
  value: string;
  onChange: (value: string) => void;
  fileName: string;
}

function getLanguageFromFileName(fileName: string): string {
  const ext = fileName.split('.').pop()?.toLowerCase() || '';
  const map: Record<string, string> = {
    js: 'javascript',
    jsx: 'javascript',
    ts: 'typescript',
    tsx: 'typescript',
    py: 'python',
    rs: 'rust',
    go: 'go',
    java: 'java',
    json: 'json',
    xml: 'xml',
    yaml: 'yaml',
    yml: 'yaml',
    html: 'html',
    css: 'css',
    scss: 'scss',
    md: 'markdown',
    sql: 'sql',
    sh: 'shell',
    bat: 'bat',
    ps1: 'powershell',
    toml: 'ini',
    csv: 'plaintext',
    txt: 'plaintext',
  };
  return map[ext] || 'plaintext';
}

function useSystemTheme(): 'vs-dark' | 'light' {
  if (typeof window === 'undefined') return 'light';
  return document.documentElement.classList.contains('dark') ? 'vs-dark' : 'light';
}

export function MonacoEditorWrapper({
  value,
  onChange,
  fileName,
}: MonacoEditorWrapperProps) {
  const language = getLanguageFromFileName(fileName);
  const theme = useSystemTheme();

  const handleChange = useCallback(
    (val: string | undefined) => {
      onChange(val ?? '');
    },
    [onChange],
  );

  return (
    <div className="h-full flex flex-col">
      <div className="px-3 py-1 text-xs text-muted-foreground border-b bg-muted/20">
        {language}
      </div>
      <div className="flex-1">
        <Editor
          language={language}
          theme={theme}
          value={value}
          onChange={handleChange}
          options={{
            minimap: { enabled: false },
            fontSize: 14,
            lineNumbers: 'on',
            wordWrap: 'on',
            scrollBeyondLastLine: false,
            automaticLayout: true,
            tabSize: 2,
          }}
        />
      </div>
    </div>
  );
}
