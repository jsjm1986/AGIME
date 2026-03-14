import React, {
  Fragment,
  memo,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkBreaks from "remark-breaks";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import "katex/dist/katex.min.css";
import { Prism as SyntaxHighlighter } from "react-syntax-highlighter";
import { oneDark } from "react-syntax-highlighter/dist/esm/styles/prism";
import { Check, Copy } from "lucide-react";
import { wrapHTMLInCodeBlock } from "../utils/htmlSecurity";

const customTheme = {
  ...oneDark,
  'code[class*="language-"]': {
    ...oneDark['code[class*="language-"]'],
    color: "hsl(var(--ui-code-foreground))",
    fontSize: "13px",
  },
  'pre[class*="language-"]': {
    ...oneDark['pre[class*="language-"]'],
    color: "hsl(var(--ui-code-foreground))",
    fontSize: "13px",
  },
  comment: {
    ...oneDark.comment,
    color: "hsl(var(--ui-code-muted))",
    fontStyle: "italic" as const,
  },
  prolog: { ...oneDark.prolog, color: "hsl(var(--ui-code-muted))" },
  doctype: { ...oneDark.doctype, color: "hsl(var(--ui-code-muted))" },
  cdata: { ...oneDark.cdata, color: "hsl(var(--ui-code-muted))" },
};

interface CodeProps
  extends
    React.ClassAttributes<HTMLElement>,
    React.HTMLAttributes<HTMLElement> {
  inline?: boolean;
}

const codeLanguagePattern = /language-([^\s]+)/;

function normalizeCodeLanguage(language?: string): string {
  if (!language) {
    return "text";
  }

  const normalized = language.toLowerCase();
  const aliases: Record<string, string> = {
    plaintext: "text",
    txt: "text",
    console: "bash",
    shell: "bash",
    "shell-session": "bash",
    sh: "bash",
    zsh: "bash",
  };

  return aliases[normalized] ?? normalized;
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
  const displayLanguage = normalizeCodeLanguage(language);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(children);
      setCopied(true);
      if (timeoutRef.current) window.clearTimeout(timeoutRef.current);
      timeoutRef.current = window.setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.error("Failed to copy:", err);
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
        language={displayLanguage}
        PreTag="pre"
        customStyle={{
          margin: 0,
          width: "fit-content",
          minWidth: "100%",
          maxWidth: "none",
          background: "transparent",
          padding: "1rem",
          overflow: "visible",
        }}
        codeTagProps={{
          style: {
            whiteSpace: "pre",
            wordBreak: "normal",
            overflowWrap: "normal",
            fontFamily:
              "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, Liberation Mono, Courier New, monospace",
            fontSize: "13px",
            lineHeight: "1.6",
          },
        }}
        showLineNumbers={false}
        wrapLines={false}
        wrapLongLines={false}
      >
        {children}
      </SyntaxHighlighter>
    ),
    [displayLanguage, children],
  );

  return (
    <div className="relative group my-3 w-full overflow-hidden rounded-xl border border-[hsl(var(--ui-line-soft))/0.78] bg-[hsl(var(--ui-code-surface))/0.98] shadow-sm">
      <div className="flex items-center justify-between gap-3 border-b border-[hsl(var(--ui-line-soft))/0.6] bg-[hsl(var(--ui-code-surface))/0.98] px-3.5 py-2 text-xs text-[hsl(var(--ui-code-muted))]">
        <span className="font-mono text-[hsl(var(--ui-code-foreground))]">
          {displayLanguage}
        </span>
        <button
          onClick={handleCopy}
          className="inline-flex items-center gap-1 rounded-md border border-[hsl(var(--ui-line-soft))/0.66] bg-[hsl(var(--ui-code-surface))/0.72] px-2 py-1 text-[hsl(var(--ui-code-muted))] transition-colors hover:text-[hsl(var(--ui-code-foreground))]"
          title={t("common.copy", "Copy")}
        >
          {copied ? (
            <>
              <Check className="h-3 w-3 text-status-success-text" />
              <span className="text-status-success-text">
                {t("common.copied", "Copied")}
              </span>
            </>
          ) : (
            <>
              <Copy className="h-3 w-3" />
              <span>{t("common.copy", "Copy")}</span>
            </>
          )}
        </button>
      </div>
      <div className="w-full overflow-x-auto rounded-b-xl">{highlighter}</div>
    </div>
  );
});

const MarkdownCode = memo(
  React.forwardRef(function MarkdownCode(
    { inline, className, children, ...props }: CodeProps,
    ref: React.Ref<HTMLElement>,
  ) {
    const match = codeLanguagePattern.exec(className || "");

    if (!inline) {
      return (
        <CodeBlock language={match?.[1] ?? "text"}>
          {String(children).replace(/\n$/, "")}
        </CodeBlock>
      );
    }

    return (
      <code
        ref={ref}
        {...props}
        className="rounded-md px-1.5 py-0.5 font-mono text-[13px] font-medium whitespace-pre-wrap"
        style={{
          backgroundColor: "hsl(var(--ui-inline-code-bg))",
          border: "1px solid hsl(var(--ui-inline-code-border))",
          color: "hsl(var(--ui-inline-code-text))",
        }}
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

function parsePipeTableCells(line: string): string[] | null {
  const trimmed = line.trim();
  if (!trimmed.startsWith("|") || !trimmed.endsWith("|")) {
    return null;
  }
  const cells = trimmed
    .slice(1, -1)
    .split("|")
    .map((cell) => cell.trim());
  return cells.length >= 2 ? cells : null;
}

function isPipeTableSeparator(line: string): boolean {
  const cells = parsePipeTableCells(line);
  if (!cells) return false;
  return cells.every((cell) => /^:?-{3,}:?$/.test(cell.replace(/\s+/g, "")));
}

function buildPipeTableSeparator(
  headerLine: string,
  candidate?: string,
): string {
  const headerCells = parsePipeTableCells(headerLine) || [];
  const candidateCells = candidate ? parsePipeTableCells(candidate) || [] : [];
  const separatorCells = headerCells.map((_, index) => {
    const alignment = candidateCells[index]?.replace(/\s+/g, "") || "---";
    const left = alignment.startsWith(":");
    const right = alignment.endsWith(":");
    if (left && right) return ":---:";
    if (left) return ":---";
    if (right) return "---:";
    return "---";
  });
  return `| ${separatorCells.join(" | ")} |`;
}

function normalizeLoosePipeTables(source: string): string {
  const lines = source.split("\n");
  const normalized: string[] = [];
  let inFence = false;

  for (let index = 0; index < lines.length; ) {
    const line = lines[index];
    if (/^\s*(```|~~~)/.test(line)) {
      inFence = !inFence;
      normalized.push(line);
      index += 1;
      continue;
    }

    if (inFence) {
      normalized.push(line);
      index += 1;
      continue;
    }

    const headerCells = parsePipeTableCells(line);
    if (!headerCells) {
      normalized.push(line);
      index += 1;
      continue;
    }

    const group: string[] = [line];
    let cursor = index + 1;
    while (cursor < lines.length && parsePipeTableCells(lines[cursor])) {
      group.push(lines[cursor]);
      cursor += 1;
    }

    if (group.length < 2) {
      normalized.push(line);
      index += 1;
      continue;
    }

    const nextLine = group[1];
    const dataRows = isPipeTableSeparator(nextLine)
      ? group.slice(2)
      : group.slice(1);
    const hasCompatibleRows = dataRows.some((row) => {
      const rowCells = parsePipeTableCells(row);
      return rowCells && rowCells.length === headerCells.length;
    });

    if (!isPipeTableSeparator(nextLine) && !hasCompatibleRows) {
      normalized.push(line);
      index += 1;
      continue;
    }

    if (
      normalized.length > 0 &&
      normalized[normalized.length - 1].trim() !== ""
    ) {
      normalized.push("");
    }

    normalized.push(group[0]);
    normalized.push(
      buildPipeTableSeparator(
        group[0],
        isPipeTableSeparator(nextLine) ? nextLine : undefined,
      ),
    );
    normalized.push(
      ...(isPipeTableSeparator(nextLine) ? group.slice(2) : group.slice(1)),
    );

    if (cursor < lines.length && lines[cursor].trim() !== "") {
      normalized.push("");
    }

    index = cursor;
  }

  return normalized.join("\n");
}

const MarkdownContent = memo(function MarkdownContent({
  content,
  className = "",
}: MarkdownContentProps) {
  const processedContent = useMemo(() => {
    try {
      return normalizeLoosePipeTables(wrapHTMLInCodeBlock(content));
    } catch {
      return content;
    }
  }, [content]);

  return (
    <div
      className={`w-full max-w-full overflow-x-hidden break-words [overflow-wrap:anywhere] prose prose-sm dark:prose-invert
      [&]:text-inherit prose-headings:text-inherit prose-headings:font-semibold prose-headings:tracking-tight prose-strong:text-inherit prose-em:text-inherit
      prose-pre:m-0 prose-pre:bg-transparent prose-pre:p-0 prose-pre:shadow-none prose-pre:overflow-visible
      prose-code:font-mono prose-code:text-[13px] prose-code:font-medium prose-code:before:content-none prose-code:after:content-none
      prose-a:break-words prose-a:[overflow-wrap:anywhere] prose-a:text-cyan-600 prose-a:dark:text-cyan-400 prose-a:no-underline prose-a:hover:underline
      prose-p:break-words prose-p:[overflow-wrap:anywhere]
      prose-li:break-words prose-li:[overflow-wrap:anywhere]
      prose-td:break-words prose-td:[overflow-wrap:anywhere]
      prose-table:my-0 prose-table:w-full prose-table:text-sm
      prose-blockquote:rounded-r-lg prose-blockquote:border-l-2 prose-blockquote:border-[hsl(var(--ui-line-strong))/0.72] prose-blockquote:bg-[hsl(var(--ui-surface-panel-muted))/0.44] prose-blockquote:py-1 prose-blockquote:pr-4 prose-blockquote:pl-4 prose-blockquote:font-normal
      prose-td:border-0 prose-td:border-b prose-td:border-[hsl(var(--ui-line-soft))/0.8] prose-td:px-3 prose-td:py-2
      prose-th:border-0 prose-th:border-b prose-th:border-[hsl(var(--ui-line-strong))/0.72] prose-th:px-3 prose-th:py-2 prose-th:text-left prose-th:font-medium prose-th:text-xs prose-th:uppercase prose-th:tracking-wide
      prose-thead:bg-[hsl(var(--ui-surface-panel-muted))/0.72]
      prose-h1:text-xl prose-h1:font-semibold prose-h1:mb-4 prose-h1:mt-0
      prose-h2:text-lg prose-h2:font-semibold prose-h2:mb-3 prose-h2:mt-3
      prose-h3:text-base prose-h3:font-semibold prose-h3:mb-2 prose-h3:mt-2
      prose-p:mt-0 prose-p:mb-3 prose-p:leading-7
      prose-ol:my-3 prose-ol:pl-5 prose-ul:my-3 prose-ul:pl-5
      prose-li:my-1.5 prose-hr:my-5 ${className}`}
    >
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkBreaks, remarkMath]}
        rehypePlugins={[
          [
            rehypeKatex,
            {
              throwOnError: false,
              errorColor: "hsl(var(--status-error-text))",
              strict: false,
            },
          ],
        ]}
        components={{
          a: ({ ...props }) => (
            <a {...props} target="_blank" rel="noopener noreferrer" />
          ),
          pre: ({ children }) => <Fragment>{children}</Fragment>,
          table: ({ className: tableClassName, ...props }) => (
            <div className="my-4 w-full overflow-x-auto rounded-xl border border-[hsl(var(--ui-line-soft))/0.8] bg-[hsl(var(--ui-surface-panel-strong))/0.88] shadow-sm">
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
