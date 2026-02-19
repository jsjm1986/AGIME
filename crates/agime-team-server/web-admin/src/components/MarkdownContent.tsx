import React, { useState, useRef, memo, useMemo, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import remarkBreaks from 'remark-breaks';
import remarkMath from 'remark-math';
import rehypeKatex from 'rehype-katex';
import 'katex/dist/katex.min.css';
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import { oneDark } from 'react-syntax-highlighter/dist/esm/styles/prism';
import { Check, Copy } from 'lucide-react';
import { wrapHTMLInCodeBlock } from '../utils/htmlSecurity';

const customTheme = {
  ...oneDark,
  'code[class*="language-"]': {
    ...oneDark['code[class*="language-"]'],
    color: '#e6e6e6',
    fontSize: '13px',
  },
  'pre[class*="language-"]': {
    ...oneDark['pre[class*="language-"]'],
    color: '#e6e6e6',
    fontSize: '13px',
  },
  comment: { ...oneDark.comment, color: '#a0a0a0', fontStyle: 'italic' as const },
  prolog: { ...oneDark.prolog, color: '#a0a0a0' },
  doctype: { ...oneDark.doctype, color: '#a0a0a0' },
  cdata: { ...oneDark.cdata, color: '#a0a0a0' },
};

interface CodeProps extends React.ClassAttributes<HTMLElement>, React.HTMLAttributes<HTMLElement> {
  inline?: boolean;
}

const CodeBlock = memo(function CodeBlock({
  language,
  children,
}: {
  language: string;
  children: string;
}) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  const timeoutRef = useRef<number | null>(null);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(children);
      setCopied(true);
      if (timeoutRef.current) window.clearTimeout(timeoutRef.current);
      timeoutRef.current = window.setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.error('Failed to copy:', err);
    }
  };

  useEffect(() => {
    return () => {
      if (timeoutRef.current) window.clearTimeout(timeoutRef.current);
    };
  }, []);

  const highlighter = useMemo(
    () => (
      <SyntaxHighlighter
        style={customTheme}
        language={language}
        PreTag="div"
        customStyle={{ margin: 0, width: '100%', maxWidth: '100%' }}
        codeTagProps={{
          style: {
            whiteSpace: 'pre-wrap',
            wordBreak: 'break-all',
            overflowWrap: 'break-word',
            fontSize: '13px',
          },
        }}
        showLineNumbers={false}
        wrapLines={false}
      >
        {children}
      </SyntaxHighlighter>
    ),
    [language, children],
  );

  return (
    <div className="relative group w-full my-2">
      <div className="flex justify-between items-center px-3 py-1.5 bg-slate-800/90 dark:bg-slate-900/90 text-slate-400 text-xs rounded-t-lg border-b border-slate-700/50">
        <span className="font-mono text-slate-300">{language || 'code'}</span>
        <button
          onClick={handleCopy}
          className="flex items-center gap-1 text-slate-400 hover:text-slate-200 transition-colors"
          title={t('common.copy', 'Copy')}
        >
          {copied ? (
            <>
              <Check className="h-3 w-3 text-emerald-400" />
              <span className="text-emerald-400">Copied</span>
            </>
          ) : (
            <>
              <Copy className="h-3 w-3" />
              <span>Copy</span>
            </>
          )}
        </button>
      </div>
      <div className="w-full overflow-x-auto rounded-b-lg">{highlighter}</div>
    </div>
  );
});

const MarkdownCode = memo(
  React.forwardRef(function MarkdownCode(
    { inline, className, children, ...props }: CodeProps,
    ref: React.Ref<HTMLElement>,
  ) {
    const match = /language-(\w+)/.exec(className || '');
    return !inline && match ? (
      <CodeBlock language={match[1]}>{String(children).replace(/\n$/, '')}</CodeBlock>
    ) : (
      <code
        ref={ref}
        {...props}
        className="break-all bg-cyan-500/8 dark:bg-cyan-400/12 border border-cyan-500/15 dark:border-cyan-400/20 px-1.5 py-0.5 rounded text-[13px] whitespace-pre-wrap text-cyan-600 dark:text-cyan-400 font-medium"
      >
        {children}
      </code>
    );
  }),
);

interface MarkdownContentProps {
  content: string;
  className?: string;
}

const MarkdownContent = memo(function MarkdownContent({
  content,
  className = '',
}: MarkdownContentProps) {
  const processedContent = useMemo(() => {
    try {
      return wrapHTMLInCodeBlock(content);
    } catch {
      return content;
    }
  }, [content]);

  return (
    <div
      className={`w-full overflow-x-hidden prose prose-sm dark:prose-invert max-w-full
      prose-pre:p-0 prose-pre:m-0 prose-pre:rounded-lg prose-pre:overflow-hidden
      prose-code:break-all prose-code:whitespace-pre-wrap prose-code:text-[13px] prose-code:font-medium
      prose-a:break-all prose-a:overflow-wrap-anywhere prose-a:text-cyan-600 prose-a:dark:text-cyan-400 prose-a:no-underline prose-a:hover:underline
      prose-table:w-full prose-table:text-sm prose-table:rounded-lg prose-table:overflow-hidden prose-table:border prose-table:border-slate-200 prose-table:dark:border-slate-700/50
      prose-blockquote:border-l-2 prose-blockquote:border-slate-300 prose-blockquote:dark:border-slate-600 prose-blockquote:pl-4 prose-blockquote:italic
      prose-td:border-0 prose-td:border-b prose-td:border-slate-200 prose-td:dark:border-slate-700/50 prose-td:px-3 prose-td:py-2
      prose-th:border-0 prose-th:border-b prose-th:border-slate-300 prose-th:dark:border-slate-600 prose-th:px-3 prose-th:py-2 prose-th:text-left prose-th:font-medium prose-th:text-xs prose-th:uppercase prose-th:tracking-wide
      prose-thead:bg-slate-50 prose-thead:dark:bg-slate-800/50
      prose-h1:text-xl prose-h1:font-semibold prose-h1:mb-4 prose-h1:mt-0
      prose-h2:text-lg prose-h2:font-semibold prose-h2:mb-3 prose-h2:mt-3
      prose-h3:text-base prose-h3:font-semibold prose-h3:mb-2 prose-h3:mt-2
      prose-p:mt-0 prose-p:mb-2 prose-p:leading-relaxed
      prose-ol:my-2 prose-ul:mt-0 prose-ul:mb-3
      prose-li:m-0 ${className}`}
    >
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkBreaks, remarkMath]}
        rehypePlugins={[
          [rehypeKatex, { throwOnError: false, errorColor: '#cc0000', strict: false }],
        ]}
        components={{
          a: ({ ...props }) => <a {...props} target="_blank" rel="noopener noreferrer" />,
          code: MarkdownCode,
        }}
      >
        {processedContent}
      </ReactMarkdown>
    </div>
  );
});

export default MarkdownContent;
