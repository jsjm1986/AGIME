import React, {
  Fragment,
  memo,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { useParams } from "react-router-dom";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import "katex/dist/katex.min.css";
import { Prism as SyntaxHighlighter } from "react-syntax-highlighter";
import {
  oneDark,
  oneLight,
} from "react-syntax-highlighter/dist/esm/styles/prism";
import { Check, Copy } from "lucide-react";
import type {
  SemanticEntityType,
  SemanticIndexResponse,
} from "../api/semanticIndex";
import {
  getCachedSemanticIndex,
  loadSemanticIndex,
} from "../lib/semanticIndexCache";
import { cn } from "../utils";
import { copyText } from "../utils/clipboard";
import { wrapHTMLInCodeBlock } from "../utils/htmlSecurity";

const customDarkTheme = {
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

const customLightTheme = {
  ...oneLight,
  'code[class*="language-"]': {
    ...oneLight['code[class*="language-"]'],
    color: "hsl(var(--foreground))",
    background: "transparent",
    fontSize: "13px",
  },
  'pre[class*="language-"]': {
    ...oneLight['pre[class*="language-"]'],
    color: "hsl(var(--foreground))",
    background: "transparent",
    fontSize: "13px",
  },
  comment: {
    ...oneLight.comment,
    color: "hsl(var(--ui-code-muted))",
    fontStyle: "italic" as const,
  },
  prolog: { ...oneLight.prolog, color: "hsl(var(--ui-code-muted))" },
  doctype: { ...oneLight.doctype, color: "hsl(var(--ui-code-muted))" },
  cdata: { ...oneLight.cdata, color: "hsl(var(--ui-code-muted))" },
};

interface CodeProps
  extends
    React.ClassAttributes<HTMLElement>,
    React.HTMLAttributes<HTMLElement> {
  inline?: boolean;
}

const codeLanguagePattern = /language-([^\s]+)/;
const compactCodeBlockMaxChars = 80;
type SemanticEntityClass =
  | "portal"
  | "agent"
  | "document"
  | "folder"
  | "skill"
  | "extension"
  | "governance"
  | "workspace";

interface SemanticMatchEntry {
  term: string;
  normalizedTerm: string;
  className: string;
  entityClass: SemanticEntityClass;
  wholeWordOnly: boolean;
  priority: number;
}

interface SemanticMatcher {
  entriesByTerm: Map<string, SemanticMatchEntry>;
  pattern: RegExp | null;
}

type DocumentRefClass =
  | "markdown"
  | "office_doc"
  | "spreadsheet"
  | "slides"
  | "web"
  | "data"
  | "pdf"
  | "archive"
  | "document";
type SkillRefClass = "team" | "registry" | "imported" | "skill";
type ExtensionRefClass = "builtin" | "team" | "mcp" | "custom" | "extension";
type StructuredRefMarker =
  | {
      kind: "doc";
      docId: string;
      name: string;
      status: string;
      documentClass: string;
    }
  | {
      kind: "skill";
      skillId: string;
      name: string;
      skillClass: string;
      meta: string;
    }
  | {
      kind: "ext";
      extensionId: string;
      name: string;
      extensionClass: string;
      meta: string;
    };

interface StructuredRefRegistry {
  entries: Map<string, StructuredRefMarker>;
}

interface HastNode {
  type: string;
  value?: string;
  tagName?: string;
  properties?: Record<string, unknown>;
  children?: HastNode[];
}

const semanticEntityPriority: Record<SemanticEntityClass, number> = {
  portal: 70,
  agent: 60,
  document: 90,
  folder: 40,
  skill: 50,
  extension: 55,
  governance: 45,
  workspace: 65,
};

const persistentSemanticTerms: Array<{
  term: string;
  entityClass: SemanticEntityClass;
}> = [
  { term: "AI工作台", entityClass: "workspace" },
  { term: "AI 工作台", entityClass: "workspace" },
  { term: "AI Workspace", entityClass: "workspace" },
];

const SemanticMatcherContext = React.createContext<SemanticMatcher | null>(null);
const documentRefPattern =
  /\[\[doc:([^|\]]+)\|([^|\]]+)\|([^|\]]+)\|([^|\]]+)\]\]/giu;
const skillRefPattern =
  /\[\[skill:([^|\]]+)\|([^|\]]+)\|([^|\]]+)\|([^|\]]*)\]\]/giu;
const extensionRefPattern =
  /\[\[ext:([^|\]]+)\|([^|\]]+)\|([^|\]]+)\|([^|\]]*)\]\]/giu;
const structuredRefPattern =
  /\[\[(doc|skill|ext):([^|\]]+)\|([^|\]]+)\|([^|\]]+)\|([^|\]]*)\]\]/giu;
const structuredRefPlaceholderPattern = /@@AGIME_STRUCTURED_REF_(\d+)@@/g;

function normalizeEntityTerm(value: string): string {
  return value.trim().toLowerCase();
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function isSemanticWordLike(term: string): boolean {
  return /^[A-Za-z0-9._-]+$/.test(term);
}

function isSemanticWordChar(char?: string): boolean {
  return Boolean(char && /[A-Za-z0-9._-]/.test(char));
}

function shouldIndexSemanticTerm(term: string): boolean {
  const trimmed = term.trim();
  if (!trimmed || trimmed.length < 2) {
    return false;
  }
  if (trimmed.startsWith("/") || trimmed.includes("\\")) {
    return false;
  }
  return true;
}

function semanticEntityClassName(type: SemanticEntityType | "builtin_extension"): SemanticEntityClass | null {
  switch (type) {
    case "portal":
      return "portal";
    case "agent":
      return "agent";
    case "document":
      return null;
    case "folder":
      return "folder";
    case "skill":
      return null;
    case "extension":
    case "builtin_extension":
      return null;
    case "governance_request":
      return "governance";
    default:
      return null;
  }
}

function registerSemanticTerm(
  registry: Map<string, SemanticMatchEntry>,
  rawTerm: string,
  entityClass: SemanticEntityClass,
) {
  if (!shouldIndexSemanticTerm(rawTerm)) {
    return;
  }

  const term = rawTerm.trim();
  const normalizedTerm = normalizeEntityTerm(term);
  const className = `markdown-semantic-token markdown-semantic-token--${entityClass}`;
  const candidate: SemanticMatchEntry = {
    term,
    normalizedTerm,
    className,
    entityClass,
    wholeWordOnly: isSemanticWordLike(term),
    priority: semanticEntityPriority[entityClass],
  };
  const existing = registry.get(normalizedTerm);
  if (!existing || candidate.priority > existing.priority) {
    registry.set(normalizedTerm, candidate);
  }
}

function buildSemanticMatcher(index: SemanticIndexResponse | null): SemanticMatcher | null {
  const registry = new Map<string, SemanticMatchEntry>();

  for (const entry of persistentSemanticTerms) {
    registerSemanticTerm(registry, entry.term, entry.entityClass);
  }

  if (!index) {
    const entries = Array.from(registry.values()).sort(
      (left, right) => right.term.length - left.term.length,
    );
    return entries.length === 0
      ? null
      : {
          entriesByTerm: registry,
          pattern: new RegExp(
            entries.map((entry) => escapeRegExp(entry.term)).join("|"),
            "giu",
          ),
        };
  }

  const addEntityTerms = (
    type: SemanticEntityType | "builtin_extension",
    names: Array<string | null | undefined>,
  ) => {
    const entityClass = semanticEntityClassName(type);
    if (!entityClass) {
      return;
    }
    for (const name of names) {
      if (!name) continue;
      registerSemanticTerm(registry, name, entityClass);
    }
  };

  for (const entity of index.entities) {
    addEntityTerms(entity.type, [
      entity.displayName,
      entity.name,
      ...(Array.isArray(entity.aliases) ? entity.aliases : []),
    ]);
  }

  for (const builtin of index.builtinCatalog) {
    addEntityTerms("builtin_extension", [
      builtin.displayName,
      builtin.name,
      ...(Array.isArray(builtin.aliases) ? builtin.aliases : []),
    ]);
  }

  const entries = Array.from(registry.values()).sort(
    (left, right) => right.term.length - left.term.length,
  );

  if (entries.length === 0) {
    return null;
  }

  return {
    entriesByTerm: registry,
    pattern: new RegExp(entries.map((entry) => escapeRegExp(entry.term)).join("|"), "giu"),
  };
}

function highlightSemanticMatches(
  text: string,
  matcher: SemanticMatcher | null,
  keyPrefix: string,
): React.ReactNode {
  if (!matcher?.pattern || !text) {
    return text;
  }

  matcher.pattern.lastIndex = 0;
  const parts: React.ReactNode[] = [];
  let cursor = 0;
  let matchIndex = 0;
  let match: RegExpExecArray | null;

  while ((match = matcher.pattern.exec(text)) !== null) {
    const matchedText = match[0];
    const start = match.index;
    const end = start + matchedText.length;
    const entry = matcher.entriesByTerm.get(normalizeEntityTerm(matchedText));
    if (!entry) {
      continue;
    }

    if (entry.wholeWordOnly) {
      const previousChar = text[start - 1];
      const nextChar = text[end];
      if (isSemanticWordChar(previousChar) || isSemanticWordChar(nextChar)) {
        continue;
      }
    }

    if (entry.entityClass === "extension") {
      const previousChar = text[start - 1];
      const nextChar = text[end];
      if (previousChar === "/" || nextChar === "/") {
        continue;
      }
    }

    if (start > cursor) {
      parts.push(text.slice(cursor, start));
    }
    parts.push(
      <span
        key={`${keyPrefix}-${matchIndex}`}
        className={entry.className}
      >
        {matchedText}
      </span>,
    );
    cursor = end;
    matchIndex += 1;
  }

  if (cursor === 0) {
    return text;
  }

  if (cursor < text.length) {
    parts.push(text.slice(cursor));
  }

  return parts;
}

function normalizeDocumentRefClass(value: string): DocumentRefClass {
  switch (value.trim().toLowerCase()) {
    case "markdown":
      return "markdown";
    case "office_doc":
      return "office_doc";
    case "spreadsheet":
      return "spreadsheet";
    case "slides":
      return "slides";
    case "web":
      return "web";
    case "data":
      return "data";
    case "pdf":
      return "pdf";
    case "archive":
      return "archive";
    default:
      return "document";
  }
}

function renderDocumentRef(
  docId: string,
  name: string,
  status: string,
  documentClass: string,
  key: string,
): React.ReactNode {
  return (
    <span
      key={key}
      className={`markdown-doc-ref markdown-doc-ref--${normalizeDocumentRefClass(documentClass)}`}
      data-doc-id={docId}
      data-doc-status={status}
      data-doc-class={documentClass}
    >
      {name}
    </span>
  );
}

function normalizeSkillRefClass(value: string): SkillRefClass {
  switch (value.trim().toLowerCase()) {
    case "team":
      return "team";
    case "registry":
      return "registry";
    case "imported":
      return "imported";
    default:
      return "skill";
  }
}

function renderSkillRef(
  skillId: string,
  name: string,
  skillClass: string,
  meta: string,
  key: string,
): React.ReactNode {
  return (
    <span
      key={key}
      className={`markdown-skill-ref markdown-skill-ref--${normalizeSkillRefClass(skillClass)}`}
      data-skill-id={skillId}
      data-skill-class={skillClass}
      data-skill-meta={meta}
    >
      {name}
    </span>
  );
}

function normalizeExtensionRefClass(value: string): ExtensionRefClass {
  switch (value.trim().toLowerCase()) {
    case "builtin":
      return "builtin";
    case "team":
      return "team";
    case "mcp":
      return "mcp";
    case "custom":
      return "custom";
    default:
      return "extension";
  }
}

function renderExtensionRef(
  extensionId: string,
  name: string,
  extensionClass: string,
  meta: string,
  key: string,
): React.ReactNode {
  return (
    <span
      key={key}
      className={`markdown-extension-ref markdown-extension-ref--${normalizeExtensionRefClass(
        extensionClass,
      )}`}
      data-extension-id={extensionId}
      data-extension-class={extensionClass}
      data-extension-meta={meta}
    >
      {name}
    </span>
  );
}

function createStructuredRefPlaceholder(index: number): string {
  return `@@AGIME_STRUCTURED_REF_${index}@@`;
}

function replaceStructuredRefsInPlainText(
  text: string,
  registry: StructuredRefRegistry,
): string {
  structuredRefPattern.lastIndex = 0;
  return text.replace(
    structuredRefPattern,
    (_, kind, refId, name, refClass, meta) => {
      const placeholder = createStructuredRefPlaceholder(registry.entries.size);
      if (kind === "doc") {
        registry.entries.set(placeholder, {
          kind: "doc",
          docId: refId,
          name,
          status: refClass,
          documentClass: meta,
        });
      } else if (kind === "skill") {
        registry.entries.set(placeholder, {
          kind: "skill",
          skillId: refId,
          name,
          skillClass: refClass,
          meta,
        });
      } else {
        registry.entries.set(placeholder, {
          kind: "ext",
          extensionId: refId,
          name,
          extensionClass: refClass,
          meta,
        });
      }
      return placeholder;
    },
  );
}

function preprocessStructuredRefs(source: string): {
  content: string;
  registry: StructuredRefRegistry;
} {
  const registry: StructuredRefRegistry = {
    entries: new Map<string, StructuredRefMarker>(),
  };
  const lines = source.split("\n");
  const processed: string[] = [];
  let inFence = false;

  for (const line of lines) {
    if (/^\s*(```|~~~)/.test(line)) {
      inFence = !inFence;
      processed.push(line);
      continue;
    }

    if (inFence) {
      processed.push(line);
      continue;
    }

    let cursor = 0;
    let output = "";
    while (cursor < line.length) {
      if (line[cursor] === "`") {
        let tickCount = 1;
        while (line[cursor + tickCount] === "`") {
          tickCount += 1;
        }
        const fence = "`".repeat(tickCount);
        const closingIndex = line.indexOf(fence, cursor + tickCount);
        if (closingIndex === -1) {
          output += replaceStructuredRefsInPlainText(
            line.slice(cursor),
            registry,
          );
          cursor = line.length;
          break;
        }
        output += line.slice(cursor, closingIndex + tickCount);
        cursor = closingIndex + tickCount;
        continue;
      }

      const nextTick = line.indexOf("`", cursor);
      const plainSegment =
        nextTick === -1 ? line.slice(cursor) : line.slice(cursor, nextTick);
      output += replaceStructuredRefsInPlainText(plainSegment, registry);
      cursor = nextTick === -1 ? line.length : nextTick;
    }
    processed.push(output);
  }

  return {
    content: processed.join("\n"),
    registry,
  };
}

function renderStructuredRefMarker(
  marker: StructuredRefMarker,
  key: string,
): React.ReactNode {
  switch (marker.kind) {
    case "doc":
      return renderDocumentRef(
        marker.docId,
        marker.name,
        marker.status,
        marker.documentClass,
        key,
      );
    case "skill":
      return renderSkillRef(
        marker.skillId,
        marker.name,
        marker.skillClass,
        marker.meta,
        key,
      );
    case "ext":
      return renderExtensionRef(
        marker.extensionId,
        marker.name,
        marker.extensionClass,
        marker.meta,
        key,
      );
    default:
      return null;
  }
}

function replaceStructuredRefPlaceholdersInHast(
  node: HastNode,
  structuredRefRegistry: StructuredRefRegistry | null,
): void {
  if (
    !node ||
    typeof node !== "object" ||
    !structuredRefRegistry ||
    structuredRefRegistry.entries.size === 0
  ) {
    return;
  }

  const children = Array.isArray(node.children) ? node.children : null;
  if (!children || children.length === 0) {
    return;
  }

  if (node.type === "element" && (node.tagName === "code" || node.tagName === "pre")) {
    return;
  }

  const nextChildren: HastNode[] = [];

  for (const child of children) {
    if (!child || typeof child !== "object") {
      continue;
    }

    if (child.type === "text" && typeof child.value === "string") {
      structuredRefPlaceholderPattern.lastIndex = 0;
      let cursor = 0;
      let match: RegExpExecArray | null;
      let replaced = false;

      while ((match = structuredRefPlaceholderPattern.exec(child.value)) !== null) {
        const placeholder = match[0];
        const marker = structuredRefRegistry.entries.get(placeholder);
        if (!marker) {
          continue;
        }

        replaced = true;
        const start = match.index;
        const end = start + placeholder.length;

        if (start > cursor) {
          nextChildren.push({
            type: "text",
            value: child.value.slice(cursor, start),
          });
        }

        nextChildren.push({
          type: "element",
          tagName: "agime-ref",
          properties: {
            "data-placeholder": placeholder,
          },
          children: [],
        });

        cursor = end;
      }

      if (!replaced) {
        nextChildren.push(child);
        continue;
      }

      if (cursor < child.value.length) {
        nextChildren.push({
          type: "text",
          value: child.value.slice(cursor),
        });
      }

      continue;
    }

    replaceStructuredRefPlaceholdersInHast(child, structuredRefRegistry);
    nextChildren.push(child);
  }

  node.children = nextChildren;
}

function rehypeStructuredRefs(structuredRefRegistry: StructuredRefRegistry | null) {
  return function transform(tree: HastNode) {
    if (!tree || typeof tree !== "object") {
      return;
    }
    replaceStructuredRefPlaceholdersInHast(tree, structuredRefRegistry);
  };
}

function highlightRawProjectText(
  text: string,
  matcher: SemanticMatcher | null,
  keyPrefix: string,
): React.ReactNode {
  if (!text) {
    return text;
  }

  documentRefPattern.lastIndex = 0;
  skillRefPattern.lastIndex = 0;
  extensionRefPattern.lastIndex = 0;
  const docParts: React.ReactNode[] = [];
  let cursor = 0;
  let markerIndex = 0;
  const markers: Array<
    | {
        start: number;
        end: number;
        node: React.ReactNode;
      }
  > = [];
  let markerMatch: RegExpExecArray | null;

  while ((markerMatch = documentRefPattern.exec(text)) !== null) {
    const [rawMatch, docId, name, status, documentClass] = markerMatch;
    const start = markerMatch.index;
    const end = start + rawMatch.length;
    markers.push({
      start,
      end,
      node: renderDocumentRef(
        docId,
        name,
        status,
        documentClass,
        `${keyPrefix}-doc-${markers.length}`,
      ),
    });
  }

  while ((markerMatch = skillRefPattern.exec(text)) !== null) {
    const [rawMatch, skillId, name, skillClass, meta] = markerMatch;
    const start = markerMatch.index;
    const end = start + rawMatch.length;
    markers.push({
      start,
      end,
      node: renderSkillRef(
        skillId,
        name,
        skillClass,
        meta,
        `${keyPrefix}-skill-${markers.length}`,
      ),
    });
  }

  while ((markerMatch = extensionRefPattern.exec(text)) !== null) {
    const [rawMatch, extensionId, name, extensionClass, meta] = markerMatch;
    const start = markerMatch.index;
    const end = start + rawMatch.length;
    markers.push({
      start,
      end,
      node: renderExtensionRef(
        extensionId,
        name,
        extensionClass,
        meta,
        `${keyPrefix}-ext-${markers.length}`,
      ),
    });
  }

  markers.sort((left, right) => left.start - right.start || left.end - right.end);

  for (const marker of markers) {
    if (marker.start < cursor) {
      continue;
    }

    if (marker.start > cursor) {
      docParts.push(
        highlightSemanticMatches(
          text.slice(cursor, marker.start),
          matcher,
          `${keyPrefix}-text-${markerIndex}`,
        ),
      );
    }

    docParts.push(marker.node);
    cursor = marker.end;
    markerIndex += 1;
  }

  if (cursor === 0) {
    return highlightSemanticMatches(text, matcher, keyPrefix);
  }

  if (cursor < text.length) {
    docParts.push(
      highlightSemanticMatches(
        text.slice(cursor),
        matcher,
        `${keyPrefix}-tail`,
      ),
    );
  }

  return docParts;
}

function highlightProjectText(
  text: string,
  matcher: SemanticMatcher | null,
  structuredRefRegistry: StructuredRefRegistry | null,
  keyPrefix: string,
): React.ReactNode {
  if (!text) {
    return text;
  }

  if (!structuredRefRegistry || structuredRefRegistry.entries.size === 0) {
    return highlightRawProjectText(text, matcher, keyPrefix);
  }

  structuredRefPlaceholderPattern.lastIndex = 0;
  const parts: React.ReactNode[] = [];
  let cursor = 0;
  let markerIndex = 0;
  let match: RegExpExecArray | null;

  while ((match = structuredRefPlaceholderPattern.exec(text)) !== null) {
    const placeholder = match[0];
    const marker = structuredRefRegistry.entries.get(placeholder);
    if (!marker) {
      continue;
    }
    const start = match.index;
    const end = start + placeholder.length;

    if (start > cursor) {
      parts.push(
        highlightRawProjectText(
          text.slice(cursor, start),
          matcher,
          `${keyPrefix}-text-${markerIndex}`,
        ),
      );
    }

    parts.push(
      renderStructuredRefMarker(marker, `${keyPrefix}-ref-${markerIndex}`),
    );
    cursor = end;
    markerIndex += 1;
  }

  if (cursor === 0) {
    return highlightRawProjectText(text, matcher, keyPrefix);
  }

  if (cursor < text.length) {
    parts.push(
      highlightRawProjectText(
        text.slice(cursor),
        matcher,
        `${keyPrefix}-tail`,
      ),
    );
  }

  return parts;
}

function renderSemanticChildren(
  node: React.ReactNode,
  matcher: SemanticMatcher | null,
  structuredRefRegistry: StructuredRefRegistry | null,
  keyPrefix: string,
): React.ReactNode {
  if (node === null || node === undefined || typeof node === "boolean") {
    return node;
  }

  if (typeof node === "string" || typeof node === "number") {
    return highlightProjectText(
      String(node),
      matcher,
      structuredRefRegistry,
      keyPrefix,
    );
  }

  if (Array.isArray(node)) {
    return node.map((child, index) =>
      renderSemanticChildren(
        child,
        matcher,
        structuredRefRegistry,
        `${keyPrefix}-${index}`,
      ),
    );
  }

  if (React.isValidElement<{ children?: React.ReactNode }>(node)) {
    if (typeof node.type !== "string") {
      return node;
    }

    if (node.type === "code" || node.type === "pre") {
      return node;
    }

    const child = renderSemanticChildren(
      node.props.children,
      matcher,
      structuredRefRegistry,
      `${keyPrefix}-child`,
    );
    return React.cloneElement(node, undefined, child);
  }

  return node;
}

function extractInlineText(node: React.ReactNode): string {
  if (node === null || node === undefined || typeof node === "boolean") {
    return "";
  }

  if (typeof node === "string" || typeof node === "number") {
    return String(node);
  }

  if (Array.isArray(node)) {
    return node.map((child) => extractInlineText(child)).join("");
  }

  return "";
}

const fileLinkPattern =
  /(?:^|[\\/])[^\\/]+\.(?:txt|md|markdown|html?|json|csv|pdf|docx?|xlsx?|pptx?|ptx|xml|ya?ml)$/i;

function shouldRenderFileLikeLinkAsPlainText(
  href: string | undefined,
  children: React.ReactNode,
): boolean {
  const text = extractInlineText(children).trim();
  if (text && fileLinkPattern.test(text)) {
    return true;
  }

  if (!href) {
    return false;
  }

  const normalizedHref = href.split(/[?#]/, 1)[0]?.trim() || "";
  return fileLinkPattern.test(normalizedHref);
}

function normalizeCodeLanguage(language?: string | null): string {
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

function shouldRenderCompactCodeBlock(
  sourceLanguage: string | null,
  content: string,
): boolean {
  const trimmed = content.trim();

  return (
    sourceLanguage === null &&
    trimmed.length > 0 &&
    trimmed.length <= compactCodeBlockMaxChars &&
    !trimmed.includes("\n")
  );
}

function useDarkModeClass(): boolean {
  const [isDark, setIsDark] = useState(() =>
    typeof document !== "undefined"
      ? document.documentElement.classList.contains("dark")
      : false,
  );

  useEffect(() => {
    if (typeof document === "undefined") return undefined;
    const root = document.documentElement;
    const observer = new MutationObserver(() => {
      setIsDark(root.classList.contains("dark"));
    });
    observer.observe(root, {
      attributes: true,
      attributeFilter: ["class"],
    });
    return () => observer.disconnect();
  }, []);

  return isDark;
}

const CodeBlock = memo(function CodeBlock({
  language,
  sourceLanguage,
  children,
}: {
  language?: string | null;
  sourceLanguage: string | null;
  children: string;
}) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  const timeoutRef = useRef<number | null>(null);
  const isDarkMode = useDarkModeClass();
  const displayLanguage = normalizeCodeLanguage(language);
  const trimmedChildren = children.trim();
  const isCompactBlock = shouldRenderCompactCodeBlock(sourceLanguage, children);
  const showLanguageLabel = Boolean(
    sourceLanguage && displayLanguage !== "text",
  );

  const handleCopy = async () => {
    if (await copyText(children)) {
      setCopied(true);
      if (timeoutRef.current) window.clearTimeout(timeoutRef.current);
      timeoutRef.current = window.setTimeout(() => setCopied(false), 2000);
    } else {
      console.error("Failed to copy");
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
        style={isDarkMode ? customDarkTheme : customLightTheme}
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
    [displayLanguage, children, isDarkMode],
  );

  const copyButtonLabel = copied
    ? t("common.copied", "Copied")
    : t("common.copy", "Copy");

  const copyButton = (
    <button
      onClick={handleCopy}
      className={`inline-flex shrink-0 items-center justify-center rounded-md border transition-all ${
        copied
          ? "border-status-success-text/30 text-status-success-text opacity-100"
          : "border-[hsl(var(--ui-line-soft))/0.66] text-[hsl(var(--ui-code-muted))]"
      }`}
      title={copyButtonLabel}
      aria-label={copyButtonLabel}
    >
      {copied ? (
        <Check className="h-3.5 w-3.5" />
      ) : (
        <Copy className="h-3.5 w-3.5" />
      )}
    </button>
  );

  if (isCompactBlock) {
    return <code className="markdown-context-token">{trimmedChildren}</code>;
  }

  return (
    <div
      className={`group/code relative my-3 w-full overflow-hidden rounded-xl border ${
        isDarkMode
          ? "border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-code-surface))/0.98]"
          : "border-[hsl(var(--ui-line-soft))/0.88] bg-[hsl(var(--ui-surface-panel-strong))/0.98]"
      }`}
    >
      {showLanguageLabel ? (
        <div
          className={`flex items-center justify-between gap-3 border-b px-3.5 py-1.5 text-[11px] ${
            isDarkMode
              ? "border-[hsl(var(--ui-line-soft))/0.54] bg-[hsl(var(--ui-code-surface))/0.98] text-[hsl(var(--ui-code-muted))]"
              : "border-[hsl(var(--ui-line-soft))/0.74] bg-[hsl(var(--ui-surface-panel-muted))/0.86] text-[hsl(var(--ui-text-tertiary))]"
          }`}
        >
          <span
            className={`font-mono ${
              isDarkMode
                ? "text-[hsl(var(--ui-code-foreground))]"
                : "text-[hsl(var(--foreground))]"
            }`}
          >
            {displayLanguage}
          </span>
          {React.cloneElement(copyButton, {
            className: `${copyButton.props.className} h-7 w-7 ${
              isDarkMode
                ? "bg-[hsl(var(--ui-code-surface))/0.74] hover:text-[hsl(var(--ui-code-foreground))]"
                : "bg-[hsl(var(--ui-surface-panel-strong))/0.92] hover:text-[hsl(var(--foreground))]"
            } ${copied ? "opacity-100" : "opacity-0 group-hover/code:opacity-100 focus-within:opacity-100"}`,
          })}
        </div>
      ) : (
        <div className="pointer-events-none absolute right-2 top-2 z-10 opacity-0 transition-opacity group-hover/code:opacity-100 focus-within:opacity-100">
          {React.cloneElement(copyButton, {
            className: `${copyButton.props.className} pointer-events-auto h-7 w-7 ${
              isDarkMode
                ? "bg-[hsl(var(--ui-code-surface))/0.86] backdrop-blur-sm hover:text-[hsl(var(--ui-code-foreground))]"
                : "bg-[hsl(var(--ui-surface-panel-strong))/0.95] hover:text-[hsl(var(--foreground))]"
            }`,
          })}
        </div>
      )}
      <div className="w-full overflow-x-auto">{highlighter}</div>
    </div>
  );
});

const MarkdownCode = memo(
  React.forwardRef(function MarkdownCode(
    { inline, className, children, ...props }: CodeProps,
    ref: React.Ref<HTMLElement>,
  ) {
    const match = codeLanguagePattern.exec(className || "");
    const textContent = extractInlineText(children);

    if (!inline) {
      return (
        <CodeBlock
          language={match?.[1] ?? null}
          sourceLanguage={match?.[1] ?? null}
        >
          {textContent.replace(/\n$/, "")}
        </CodeBlock>
      );
    }

    return (
      <code
        ref={ref}
        {...props}
        className="markdown-context-token font-mono text-[13px] font-medium whitespace-pre-wrap"
      >
        {textContent}
      </code>
    );
  }),
);

interface MarkdownContentProps {
  content: string;
  className?: string;
}

type SemanticRenderableTag =
  | "blockquote"
  | "h1"
  | "h2"
  | "h3"
  | "h4"
  | "h5"
  | "h6"
  | "li"
  | "p"
  | "td"
  | "th";

function renderSemanticTag(
  tag: SemanticRenderableTag,
  children: React.ReactNode,
  matcher: SemanticMatcher | null,
  structuredRefRegistry: StructuredRefRegistry | null,
  keyPrefix: string,
  className?: string,
): React.ReactNode {
  return React.createElement(
    tag,
    className ? { className } : undefined,
    renderSemanticChildren(
      children,
      matcher,
      structuredRefRegistry,
      keyPrefix,
    ),
  );
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

function normalizeLatexMathInTextBlock(source: string): string {
  let result = "";
  let currentInlineCodeFence = "";

  for (let index = 0; index < source.length; ) {
    if (source[index] === "`") {
      let fenceLength = 1;
      while (source[index + fenceLength] === "`") {
        fenceLength += 1;
      }
      const fence = "`".repeat(fenceLength);
      if (!currentInlineCodeFence) {
        currentInlineCodeFence = fence;
      } else if (currentInlineCodeFence === fence) {
        currentInlineCodeFence = "";
      }
      result += fence;
      index += fenceLength;
      continue;
    }

    if (!currentInlineCodeFence) {
      if (source.startsWith("\\[", index) || source.startsWith("\\]", index)) {
        result += "$$";
        index += 2;
        continue;
      }
      if (source.startsWith("\\(", index) || source.startsWith("\\)", index)) {
        result += "$";
        index += 2;
        continue;
      }
    }

    result += source[index];
    index += 1;
  }

  return result;
}

function normalizeLatexMathDelimiters(source: string): string {
  const lines = source.split("\n");
  const normalized: string[] = [];
  const textBuffer: string[] = [];
  let inFence = false;

  const flushTextBuffer = () => {
    if (textBuffer.length === 0) {
      return;
    }
    normalized.push(normalizeLatexMathInTextBlock(textBuffer.join("\n")));
    textBuffer.length = 0;
  };

  for (const line of lines) {
    if (/^\s*(```|~~~)/.test(line)) {
      flushTextBuffer();
      inFence = !inFence;
      normalized.push(line);
      continue;
    }

    if (inFence) {
      normalized.push(line);
      continue;
    }

    textBuffer.push(line);
  }

  flushTextBuffer();
  return normalized.join("\n");
}

const MarkdownContent = memo(function MarkdownContent({
  content,
  className = "",
}: MarkdownContentProps) {
  const { teamId } = useParams<{ teamId?: string }>();
  const [semanticIndex, setSemanticIndex] =
    useState<SemanticIndexResponse | null>(() =>
      teamId ? getCachedSemanticIndex(teamId) : null,
    );

  useEffect(() => {
    if (!teamId) {
      setSemanticIndex(null);
      return;
    }

    let active = true;
    const cached = getCachedSemanticIndex(teamId);
    if (cached) {
      setSemanticIndex(cached);
    }

    loadSemanticIndex(teamId)
      .then((payload) => {
        if (active) {
          setSemanticIndex(payload);
        }
      })
      .catch(() => {
        if (active && !cached) {
          setSemanticIndex(null);
        }
      });

    return () => {
      active = false;
    };
  }, [teamId]);

  const processedContent = useMemo(() => {
    try {
      const normalized = normalizeLoosePipeTables(
        normalizeLatexMathDelimiters(wrapHTMLInCodeBlock(content)),
      );
      return preprocessStructuredRefs(normalized);
    } catch {
      return preprocessStructuredRefs(normalizeLatexMathDelimiters(content));
    }
  }, [content]);

  const semanticMatcher = useMemo(
    () => buildSemanticMatcher(semanticIndex),
    [semanticIndex],
  );

  const rehypeStructuredRefPlugin = useMemo(
    () => rehypeStructuredRefs(processedContent.registry),
    [processedContent.registry],
  );

  const markdownComponents = useMemo(
    () =>
      ({
        a: ({ className, children, href, ...props }: any) => {
          const treatAsPlainText = shouldRenderFileLikeLinkAsPlainText(
            href,
            children,
          );
          return (
            <a
              {...props}
              href={href}
              target="_blank"
              rel="noopener noreferrer"
              className={cn(
                className,
                treatAsPlainText &&
                  "!text-[hsl(var(--ui-text-secondary))] !no-underline hover:!text-[hsl(var(--foreground))] hover:underline",
              )}
            >
              {children}
            </a>
          );
        },
        h1: ({ children }: any) =>
          renderSemanticTag(
            "h1",
            children,
            semanticMatcher,
            processedContent.registry,
            "h1",
          ),
        h2: ({ children }: any) =>
          renderSemanticTag(
            "h2",
            children,
            semanticMatcher,
            processedContent.registry,
            "h2",
          ),
        h3: ({ children }: any) =>
          renderSemanticTag(
            "h3",
            children,
            semanticMatcher,
            processedContent.registry,
            "h3",
          ),
        h4: ({ children }: any) =>
          renderSemanticTag(
            "h4",
            children,
            semanticMatcher,
            processedContent.registry,
            "h4",
          ),
        h5: ({ children }: any) =>
          renderSemanticTag(
            "h5",
            children,
            semanticMatcher,
            processedContent.registry,
            "h5",
          ),
        h6: ({ children }: any) =>
          renderSemanticTag(
            "h6",
            children,
            semanticMatcher,
            processedContent.registry,
            "h6",
          ),
        p: ({ children }: any) =>
          renderSemanticTag(
            "p",
            children,
            semanticMatcher,
            processedContent.registry,
            "p",
          ),
        li: ({ children }: any) =>
          renderSemanticTag(
            "li",
            children,
            semanticMatcher,
            processedContent.registry,
            "li",
          ),
        strong: ({ children, ...props }: any) => (
          <strong
            {...props}
            className="font-semibold tracking-[-0.01em] text-[hsl(var(--ui-markdown-strong))]"
          >
            {renderSemanticChildren(
              children,
              semanticMatcher,
              processedContent.registry,
              "strong",
            )}
          </strong>
        ),
        blockquote: ({ children }: any) =>
          renderSemanticTag(
            "blockquote",
            children,
            semanticMatcher,
            processedContent.registry,
            "blockquote",
          ),
        td: ({ children }: any) =>
          renderSemanticTag(
            "td",
            children,
            semanticMatcher,
            processedContent.registry,
            "td",
          ),
        th: ({ children }: any) =>
          renderSemanticTag(
            "th",
            children,
            semanticMatcher,
            processedContent.registry,
            "th",
          ),
        hr: () => (
          <hr className="my-6 border-0 border-t border-[hsl(var(--ui-line-soft))/0.72]" />
        ),
        pre: ({ children }: any) => <Fragment>{children}</Fragment>,
        table: ({ className: tableClassName, ...props }: any) => (
          <div className="my-4 w-full overflow-x-auto border-y border-[hsl(var(--ui-line-soft))/0.74] bg-[hsl(var(--ui-surface-panel-strong))/0.58]">
            <table {...props} className={tableClassName} />
          </div>
        ),
        "agime-ref": ({
          node,
        }: {
          node?: { properties?: Record<string, unknown> };
        }) => {
          const placeholder = String(node?.properties?.["data-placeholder"] ?? "");
          const marker = processedContent.registry.entries.get(placeholder);
          if (!marker) {
            return null;
          }
          return renderStructuredRefMarker(marker, `ref-${placeholder}`);
        },
        code: MarkdownCode,
      }) as Record<string, unknown>,
    [processedContent.registry, semanticMatcher],
  );

  return (
    <SemanticMatcherContext.Provider value={semanticMatcher}>
      <div
        className={`w-full max-w-full overflow-x-hidden break-words prose prose-sm dark:prose-invert
      [&]:text-inherit prose-headings:text-inherit prose-headings:font-semibold prose-headings:tracking-tight prose-em:text-inherit
      prose-pre:m-0 prose-pre:bg-transparent prose-pre:p-0 prose-pre:shadow-none prose-pre:overflow-visible
      prose-code:font-mono prose-code:text-[13px] prose-code:font-medium prose-code:before:content-none prose-code:after:content-none
      prose-a:break-words prose-a:text-[hsl(var(--primary))] prose-a:no-underline prose-a:underline-offset-2 prose-a:hover:underline
      prose-p:break-words prose-li:break-words prose-td:break-words
      prose-table:my-0 prose-table:w-full prose-table:text-sm
      prose-blockquote:border-0 prose-blockquote:bg-transparent prose-blockquote:py-0 prose-blockquote:pr-0 prose-blockquote:pl-0 prose-blockquote:font-normal prose-blockquote:not-italic prose-blockquote:text-[hsl(var(--ui-text-secondary))]
      prose-td:border-0 prose-td:border-b prose-td:border-[hsl(var(--ui-line-soft))/0.8] prose-td:px-3 prose-td:py-2
      prose-th:border-0 prose-th:border-b prose-th:border-[hsl(var(--ui-line-strong))/0.72] prose-th:px-3 prose-th:py-2 prose-th:text-left prose-th:font-medium prose-th:text-xs prose-th:uppercase prose-th:tracking-wide
      prose-thead:bg-[hsl(var(--ui-surface-panel-muted))/0.72]
      prose-h1:font-display prose-h1:text-[1.08rem] prose-h1:font-semibold prose-h1:mb-4 prose-h1:mt-0
      prose-h2:font-display prose-h2:text-[0.98rem] prose-h2:font-semibold prose-h2:mb-3 prose-h2:mt-5
      prose-h3:text-[0.88rem] prose-h3:font-semibold prose-h3:mb-2 prose-h3:mt-4 prose-h3:uppercase prose-h3:tracking-[0.08em] prose-h3:text-[hsl(var(--ui-text-tertiary))]
      prose-p:mt-0 prose-p:mb-3 prose-p:leading-7 prose-p:text-[hsl(var(--ui-text-secondary))]
      prose-ol:my-3 prose-ol:pl-5 prose-ul:my-3 prose-ul:pl-5 prose-ul:list-disc prose-ol:list-decimal
      prose-li:my-1 prose-li:text-[hsl(var(--ui-text-secondary))] prose-li:marker:text-[hsl(var(--ui-line-strong))]
      prose-li:[&>p]:my-1 prose-li:[&>div]:my-1 prose-hr:my-5 ${className}`}
    >
        <ReactMarkdown
          remarkPlugins={[remarkGfm, remarkMath]}
          rehypePlugins={[
            rehypeStructuredRefPlugin,
            [
              rehypeKatex,
              {
                throwOnError: false,
                errorColor: "hsl(var(--status-error-text))",
                strict: false,
              },
            ],
          ]}
          components={markdownComponents}
        >
          {processedContent.content}
        </ReactMarkdown>
      </div>
    </SemanticMatcherContext.Provider>
  );
});

export default MarkdownContent;
