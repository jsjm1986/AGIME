import React, { Fragment, memo, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import remarkBreaks from 'remark-breaks';
import remarkMath from 'remark-math';
import rehypeKatex from 'rehype-katex';
import 'katex/dist/katex.min.css';
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import { oneDark } from 'react-syntax-highlighter/dist/esm/styles/prism';
// Improved oneDark theme for better comment contrast and readability
const customOneDarkTheme = {
  ...oneDark,
  'code[class*="language-"]': {
    ...oneDark['code[class*="language-"]'],
    color: '#e6e6e6',
    fontSize: '14px',
  },
  'pre[class*="language-"]': {
    ...oneDark['pre[class*="language-"]'],
    color: '#e6e6e6',
    fontSize: '14px',
  },
  comment: { ...oneDark.comment, color: '#a0a0a0', fontStyle: 'italic' },
  prolog: { ...oneDark.prolog, color: '#a0a0a0' },
  doctype: { ...oneDark.doctype, color: '#a0a0a0' },
  cdata: { ...oneDark.cdata, color: '#a0a0a0' },
};

import { Check, Copy } from './icons';
import { wrapHTMLInCodeBlock } from '../utils/htmlSecurity';

interface CodeProps extends React.ClassAttributes<HTMLElement>, React.HTMLAttributes<HTMLElement> {
  inline?: boolean;
}

interface MarkdownContentProps {
  content: string;
  className?: string;
}

const codeLanguagePattern = /language-([^\s]+)/;

function normalizeCodeLanguage(language?: string): string {
  if (!language) {
    return 'text';
  }

  const normalized = language.toLowerCase();
  const aliases: Record<string, string> = {
    plaintext: 'text',
    txt: 'text',
    console: 'bash',
    shell: 'bash',
    'shell-session': 'bash',
    sh: 'bash',
    zsh: 'bash',
  };

  return aliases[normalized] ?? normalized;
}

// Memoized CodeBlock component to prevent re-rendering when props haven't changed
const CodeBlock = memo(function CodeBlock({
  language,
  children,
}: {
  language: string;
  children: string;
}) {
  const { t } = useTranslation('chat');
  const [copied, setCopied] = useState(false);
  const timeoutRef = useRef<number | null>(null);
  const displayLanguage = normalizeCodeLanguage(language);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(children);
      setCopied(true);

      if (timeoutRef.current) {
        window.clearTimeout(timeoutRef.current);
      }

      timeoutRef.current = window.setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.error('Failed to copy text: ', err);
    }
  };

  useEffect(() => {
    return () => {
      if (timeoutRef.current) {
        window.clearTimeout(timeoutRef.current);
      }
    };
  }, []);

  // Memoize the SyntaxHighlighter component to prevent re-rendering
  // Only re-render if language or children change
  const memoizedSyntaxHighlighter = useMemo(() => {
    // For very large code blocks, consider truncating or lazy loading
    const isLargeCodeBlock = children.length > 10000; // 10KB threshold

    if (isLargeCodeBlock) {
      console.log(`Large code block detected (${children.length} chars), consider optimization`);
    }

    return (
      <SyntaxHighlighter
        style={customOneDarkTheme}
        language={displayLanguage}
        PreTag="pre"
        customStyle={{
          margin: 0,
          width: 'fit-content',
          minWidth: '100%',
          maxWidth: 'none',
          background: 'transparent',
          padding: '1rem',
          overflow: 'visible',
        }}
        codeTagProps={{
          style: {
            whiteSpace: 'pre',
            wordBreak: 'normal',
            overflowWrap: 'normal',
            fontFamily: 'var(--font-mono)',
            fontSize: '14px',
            lineHeight: '1.6',
          },
        }}
        // Performance optimizations for SyntaxHighlighter
        showLineNumbers={false} // Disable line numbers for better performance
        wrapLines={false} // Disable line wrapping for better performance
        wrapLongLines={false}
        lineProps={undefined} // Don't add extra props to each line
      >
        {children}
      </SyntaxHighlighter>
    );
  }, [displayLanguage, children]);

  return (
    <div className="relative group my-4 w-full overflow-hidden rounded-xl border border-slate-200/70 bg-slate-950 shadow-sm dark:border-slate-700/60">
      <div className="flex items-center justify-between gap-3 border-b border-slate-800/80 bg-slate-900/95 px-3.5 py-2 text-xs text-slate-400">
        <span className="font-mono text-slate-200">{displayLanguage}</span>
        <button
          onClick={handleCopy}
          className="inline-flex items-center gap-1 rounded-md border border-slate-700/70 bg-slate-800/80 px-2 py-1 text-slate-300 transition-colors hover:border-slate-500 hover:text-white"
          title={t('copyCode')}
        >
          {copied ? (
            <>
              <Check className="h-3 w-3 text-emerald-400" />
              <span className="text-emerald-400">{t('thinkingBlock.copied')}</span>
            </>
          ) : (
            <>
              <Copy className="h-3 w-3" />
              <span>{t('thinkingBlock.copy')}</span>
            </>
          )}
        </button>
      </div>
      <div className="w-full overflow-x-auto rounded-b-xl">{memoizedSyntaxHighlighter}</div>
    </div>
  );
});

const MarkdownCode = memo(
  React.forwardRef(function MarkdownCode(
    { inline, className, children, ...props }: CodeProps,
    ref: React.Ref<HTMLElement>
  ) {
    const match = codeLanguagePattern.exec(className || '');

    if (!inline) {
      return (
        <CodeBlock language={match?.[1] ?? 'text'}>{String(children).replace(/\n$/, '')}</CodeBlock>
      );
    }

    return (
      <code
        ref={ref}
        {...props}
        className="rounded-md border border-cyan-500/15 bg-cyan-500/8 px-1.5 py-0.5 font-mono text-[0.92em] font-medium whitespace-pre-wrap text-cyan-700 dark:border-cyan-400/20 dark:bg-cyan-400/12 dark:text-cyan-300"
      >
        {children}
      </code>
    );
  })
);

const MarkdownContent = memo(function MarkdownContent({
  content,
  className = '',
}: MarkdownContentProps) {
  const processedContent = useMemo(() => {
    try {
      return wrapHTMLInCodeBlock(content);
    } catch (error) {
      console.error('Error processing content:', error);
      // Fallback to original content if processing fails
      return content;
    }
  }, [content]);

  return (
    <div
      className={`w-full overflow-x-hidden prose prose-sm text-text-default dark:prose-invert max-w-full word-break font-sans
      prose-headings:font-semibold prose-headings:tracking-tight prose-headings:text-text-default dark:prose-headings:text-white
      prose-pre:m-0 prose-pre:bg-transparent prose-pre:p-0 prose-pre:shadow-none prose-pre:overflow-visible !p-0
      prose-code:font-mono prose-code:text-[0.92em] prose-code:font-medium prose-code:before:content-none prose-code:after:content-none
      prose-a:break-words prose-a:[overflow-wrap:anywhere] prose-a:text-cyan-600 prose-a:dark:text-cyan-400 prose-a:no-underline prose-a:hover:underline
      prose-table:my-0 prose-table:w-full prose-table:text-sm
      prose-blockquote:rounded-r-lg prose-blockquote:border-l-2 prose-blockquote:border-slate-300 prose-blockquote:bg-slate-50/80 prose-blockquote:py-1 prose-blockquote:pr-4 prose-blockquote:pl-4 prose-blockquote:font-normal prose-blockquote:text-text-muted dark:prose-blockquote:border-slate-600 dark:prose-blockquote:bg-slate-900/40
      prose-td:border-0 prose-td:border-b prose-td:border-slate-200 prose-td:dark:border-slate-700/50 prose-td:px-4 prose-td:py-2.5
      prose-th:border-0 prose-th:border-b prose-th:border-slate-300 prose-th:dark:border-slate-600 prose-th:px-4 prose-th:py-2.5 prose-th:text-left prose-th:font-medium prose-th:text-xs prose-th:uppercase prose-th:tracking-wide prose-th:text-slate-600 prose-th:dark:text-slate-400
      prose-thead:bg-slate-50 prose-thead:dark:bg-slate-800/50
      prose-tr:border-0
      prose-h1:mb-5 prose-h1:mt-0 prose-h1:text-2xl prose-h1:font-sans
      prose-h2:mb-4 prose-h2:mt-5 prose-h2:text-xl prose-h2:font-sans
      prose-h3:mb-3 prose-h3:mt-4 prose-h3:text-lg prose-h3:font-sans
      prose-p:mt-0 prose-p:mb-3 prose-p:font-sans prose-p:leading-7
      prose-ol:my-3 prose-ol:pl-5 prose-ol:font-sans
      prose-ul:my-3 prose-ul:pl-5 prose-ul:font-sans
      prose-li:my-1.5 prose-li:font-sans
      prose-strong:text-inherit prose-hr:my-5 prose-hr:border-slate-200 dark:prose-hr:border-slate-700 ${className}`}
    >
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkBreaks, remarkMath]}
        rehypePlugins={[
          [
            rehypeKatex,
            {
              throwOnError: false,
              errorColor: '#cc0000',
              strict: false,
            },
          ],
        ]}
        components={{
          a: ({ ...props }) => <a {...props} target="_blank" rel="noopener noreferrer" />,
          pre: ({ children }) => <Fragment>{children}</Fragment>,
          table: ({ className: tableClassName, ...props }) => (
            <div className="my-4 w-full overflow-x-auto rounded-xl border border-slate-200/70 bg-background-card shadow-sm dark:border-slate-700/60">
              <table {...props} className={tableClassName} />
            </div>
          ),
          code: MarkdownCode,
        }}
      >
        {processedContent}
      </ReactMarkdown>
    </div>
  );
});

export default MarkdownContent;
