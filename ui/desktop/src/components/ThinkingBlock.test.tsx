import { describe, it, expect } from 'vitest';
import { normalizeThinkingContent } from './ThinkingBlock';

describe('normalizeThinkingContent', () => {
  it('rejoins one-token-per-line fragmentation (the screenshot bug)', () => {
    // Each BPE token on its own line, blank artifact lines between them.
    // Tokens carry their own leading space, mirroring real model output.
    const tokens = [
      'The', ' issue', ' is', ' that', ' `get', '_db', '0', '`', ' is',
      ' imported', ' from', ' scrm', '_api', '.py', ' but', ' missing',
    ];
    const fragmented = tokens.join('\n\n');

    const result = normalizeThinkingContent(fragmented);

    expect(result).toBe('The issue is that `get_db0` is imported from scrm_api.py but missing');
    expect(result.split(/\r?\n/).length).toBe(1);
  });

  it('rejoins fragments separated by single newlines too', () => {
    const tokens = Array.from({ length: 20 }, (_, i) => (i === 0 ? `tok${i}` : ` tok${i}`));
    const fragmented = tokens.join('\n');

    const result = normalizeThinkingContent(fragmented);

    expect(result).toBe(tokens.join(''));
    expect(result.includes('\n')).toBe(false);
  });

  it('leaves normal multi-line prose untouched', () => {
    const prose = [
      'Let me analyze this problem step by step.',
      'First, I need to check the database connection.',
      'Then I will verify the API routes are registered.',
    ].join('\n');

    expect(normalizeThinkingContent(prose)).toBe(prose);
  });

  it('leaves a short fragmented-looking block untouched (below line threshold)', () => {
    // Fewer than 12 non-empty lines: not confidently pathological, pass through.
    const short = ['a', 'b', 'c', 'd'].join('\n');
    expect(normalizeThinkingContent(short)).toBe(short);
  });

  it('collapses fragmented token runs into flowing text (structure not recoverable)', () => {
    const para1 = Array.from({ length: 8 }, (_, i) => (i === 0 ? `one${i}` : ` one${i}`)).join('\n');
    const para2 = Array.from({ length: 8 }, (_, i) => (i === 0 ? `two${i}` : ` two${i}`)).join('\n');
    // Blank line between runs is indistinguishable from per-token separators
    // when content is fragmented, so it collapses to one readable line.
    const input = `${para1}\n\n${para2}`;

    const result = normalizeThinkingContent(input);

    expect(result.split(/\r?\n/).length).toBe(1);
    expect(result.startsWith('one0')).toBe(true);
    expect(result.includes('two0')).toBe(true);
    expect(result.includes(' ')).toBe(true);
  });

  it('does not touch prose with long lines even if many', () => {
    const longLines = Array.from({ length: 30 }, (_, i) => `This is sentence number ${i} with several words.`).join('\n');
    expect(normalizeThinkingContent(longLines)).toBe(longLines);
  });

  it('handles empty and whitespace-only content', () => {
    expect(normalizeThinkingContent('')).toBe('');
    expect(normalizeThinkingContent('   \n  \n ')).toBe('   \n  \n ');
  });

  it('keeps a non-fragment line verbatim amid token runs', () => {
    const head = Array.from({ length: 14 }, (_, i) => (i === 0 ? `x${i}` : ` x${i}`)).join('\n');
    const input = `${head}\nThis whole sentence stays intact.`;

    const result = normalizeThinkingContent(input);
    const lines = result.split(/\r?\n/);

    expect(lines[lines.length - 1]).toBe('This whole sentence stays intact.');
  });
});
