import { describe, it, expect } from 'vitest';
import { classifyProviderError } from './providerErrorHints';

describe('classifyProviderError', () => {
  it('detects rate limit / quota errors', () => {
    expect(
      classifyProviderError('Ran into this error: Rate limit exceeded: quota exhausted.')
    ).toBe('rateLimit');
    expect(classifyProviderError('Error: Too many requests')).toBe('rateLimit');
    expect(classifyProviderError('status 429')).toBe('rateLimit');
    expect(classifyProviderError('insufficient_quota')).toBe('rateLimit');
  });

  it('detects auth / key errors', () => {
    expect(classifyProviderError('Invalid API key')).toBe('auth');
    expect(classifyProviderError('401 Unauthorized')).toBe('auth');
    expect(classifyProviderError('authentication failed')).toBe('auth');
  });

  it('detects context-limit errors', () => {
    expect(classifyProviderError('context length exceeded')).toBe('contextLimit');
    expect(classifyProviderError('This exceeds the model context window')).toBe('contextLimit');
    expect(classifyProviderError('token limit reached')).toBe('contextLimit');
  });

  it('detects timeout / network errors', () => {
    expect(classifyProviderError('Request timed out')).toBe('timeout');
    expect(classifyProviderError('upstream timeout')).toBe('timeout');
    expect(classifyProviderError('connection refused')).toBe('timeout');
    expect(classifyProviderError('status 503')).toBe('timeout');
  });

  it('returns null for normal assistant text', () => {
    expect(classifyProviderError('Tool execution completed. Result: True True')).toBeNull();
    expect(classifyProviderError('Here is the summary of your spreadsheet.')).toBeNull();
    expect(classifyProviderError('')).toBeNull();
    expect(classifyProviderError(null)).toBeNull();
    expect(classifyProviderError(undefined)).toBeNull();
  });

  it('does not misclassify 501 as a timeout 5xx', () => {
    // 501 is intentionally excluded from the 50[02-4] pattern.
    expect(classifyProviderError('error 501 not implemented')).toBeNull();
  });

  it('prioritizes rate limit over other categories when both could match', () => {
    // A 429 with a timeout-sounding word should still be rate limit (rule order).
    expect(classifyProviderError('429 rate limit, connection will reset')).toBe('rateLimit');
  });
});
