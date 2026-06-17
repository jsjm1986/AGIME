/**
 * Maps raw provider/model error text (often surfaced verbatim in English from
 * the backend, e.g. "Ran into this error: Rate limit exceeded: quota
 * exhausted") to a friendly, localized hint key. Non-technical users can't tell
 * that "quota exhausted" means "your API credits ran out" — this classifies the
 * common cases so the UI can show an actionable explanation above the raw text.
 *
 * Returns the i18n key suffix under the `errors:providerHints` namespace, or
 * null when the text doesn't look like a recognized provider error (in which
 * case the caller should render the text as-is).
 */
export type ProviderErrorHintKey = 'rateLimit' | 'auth' | 'contextLimit' | 'timeout';

interface HintRule {
  key: ProviderErrorHintKey;
  // Matched case-insensitively against the message text.
  patterns: RegExp[];
}

// Order matters: more specific / higher-signal categories first. The first
// rule whose pattern matches wins.
const HINT_RULES: HintRule[] = [
  {
    key: 'rateLimit',
    patterns: [
      /rate[\s_-]?limit/i,
      /quota\s*exhausted/i,
      /too\s*many\s*requests/i,
      /\b429\b/,
      /insufficient_quota/i,
    ],
  },
  {
    key: 'auth',
    patterns: [
      /invalid\s*api\s*key/i,
      /\bunauthorized\b/i,
      /authentication\s*fail/i,
      /\b401\b/,
      /\b403\b/,
      /api\s*key\s*(not\s*found|missing|invalid)/i,
    ],
  },
  {
    key: 'contextLimit',
    patterns: [
      /context[\s_-]?(length|window|limit)/i,
      /maximum\s*context/i,
      /token\s*limit/i,
      /too\s*many\s*tokens/i,
      /exceeds?\s*the\s*(model'?s\s*)?(maximum\s*)?context/i,
    ],
  },
  {
    key: 'timeout',
    patterns: [
      /\btimed?\s*out\b/i,
      /\btimeout\b/i,
      /connection\s*(refused|reset|closed|error|failed)/i,
      /\b50[02-4]\b/, // 500/502/503/504, but not 501
      /network\s*error/i,
    ],
  },
];

/**
 * Classify provider error text into a friendly-hint category, or null if it
 * isn't a recognized provider error. Safe to call on any assistant text.
 */
export function classifyProviderError(text: string | null | undefined): ProviderErrorHintKey | null {
  if (!text) {
    return null;
  }
  for (const rule of HINT_RULES) {
    if (rule.patterns.some((p) => p.test(text))) {
      return rule.key;
    }
  }
  return null;
}
