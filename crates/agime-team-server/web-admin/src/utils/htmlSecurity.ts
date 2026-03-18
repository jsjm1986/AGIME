/**
 * HTML Security Detection Utilities
 *
 * Detects potentially dangerous HTML content in markdown
 * and wraps it safely in code blocks to prevent execution.
 */

const structuredRefLinePattern = /\[\[(doc|skill|ext):/i;
const structuredRefPlaceholderPattern = /@@AGIME_STRUCTURED_REF_\d+@@/;

export function containsHTML(str: string): boolean {
  const withoutCodeBlocks = str.replace(/```[\s\S]*?```/g, '').replace(/`[^`]*`/g, '');

  const commentRegex = /<!--[\s\S]*?-->/;
  const hasComments = commentRegex.test(withoutCodeBlocks);

  const dangerousHTMLRegex =
    /<(script|style|iframe|object|embed|form|input|button|link|meta|base|br|hr|img|div|span|p|h[1-6]|a|strong|em|b|i|u|s|pre|code|blockquote|section|article|header|footer|nav|aside|main|table|tr|td|th|ul|ol|li)(?:\s[^>]*)?(?:\s*\/?>|>[^<]*<\/\1>)/i;
  const hasDangerousHTML = dangerousHTMLRegex.test(withoutCodeBlocks);

  return hasComments || hasDangerousHTML;
}

export function wrapHTMLInCodeBlock(content: string): string {
  const lines = content.split('\n');
  let insideCodeBlock = false;

  const processedLines = lines.map((line) => {
    if (line.trim().startsWith('```')) {
      insideCodeBlock = !insideCodeBlock;
      return line;
    }
    if (insideCodeBlock) return line;

    // Preserve structured refs so they can be tokenized later by MarkdownContent.
    // ReactMarkdown does not render raw HTML without rehypeRaw, so leaving these
    // lines untouched is still safe while preventing capability/doc refs from
    // being trapped inside generated fenced code blocks.
    if (
      structuredRefLinePattern.test(line) ||
      structuredRefPlaceholderPattern.test(line)
    ) {
      return line;
    }

    if (containsHTML(line)) {
      return `\`\`\`html\n${line}\n\`\`\``;
    }
    return line;
  });

  return processedLines.join('\n');
}
