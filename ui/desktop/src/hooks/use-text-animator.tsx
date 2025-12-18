import SplitType from 'split-type';
import { useEffect, useRef } from 'react';

interface TextSplitterOptions {
  resizeCallback?: () => void;
  splitTypeTypes?: ('lines' | 'words' | 'chars')[];
}

// Class to split text into lines, words, and characters for animation
export class TextSplitter {
  textElement: HTMLElement;
  onResize: (() => void) | null;
  splitText: SplitType;
  previousContainerWidth: number | null = null;
  private resizeObserver: ResizeObserver | null = null;

  constructor(textElement: HTMLElement, options: TextSplitterOptions = {}) {
    if (!textElement || !(textElement instanceof HTMLElement)) {
      throw new Error('Invalid text element provided.');
    }

    const { resizeCallback, splitTypeTypes } = options;
    this.textElement = textElement;
    this.onResize = typeof resizeCallback === 'function' ? resizeCallback : null;

    const splitOptions = splitTypeTypes ? { types: splitTypeTypes } : {};
    this.splitText = new SplitType(this.textElement, splitOptions);

    if (this.onResize) {
      this.initResizeObserver();
    }
  }

  initResizeObserver() {
    // Use a simpler approach to avoid type issues
    this.resizeObserver = new ResizeObserver(() => {
      // Just check the current width directly from the element
      if (this.textElement) {
        const currentWidth = Math.floor(this.textElement.getBoundingClientRect().width);

        if (this.previousContainerWidth && this.previousContainerWidth !== currentWidth) {
          this.splitText.split({ types: ['chars'] });
          this.onResize?.();
        }

        this.previousContainerWidth = currentWidth;
      }
    });

    this.resizeObserver.observe(this.textElement);
  }

  getLines(): HTMLElement[] {
    return this.splitText.lines ?? [];
  }

  getChars(): HTMLElement[] {
    return this.splitText.chars ?? [];
  }

  // Clean up resources
  destroy() {
    if (this.resizeObserver) {
      this.resizeObserver.disconnect();
      this.resizeObserver = null;
    }
    // SplitType has a revert method to restore original HTML
    if (this.splitText) {
      this.splitText.revert();
    }
  }
}

// Text animation class for hover effects
const lettersAndSymbols = [
  'a',
  'b',
  'c',
  'd',
  'e',
  'f',
  'g',
  'h',
  'i',
  'j',
  'k',
  'l',
  'm',
  'n',
  'o',
  'p',
  'q',
  'r',
  's',
  't',
  'u',
  'v',
  'w',
  'x',
  'y',
  'z',
  '!',
  '@',
  '#',
  '$',
  '%',
  '^',
  '&',
  '*',
  '-',
  '_',
  '+',
  '=',
  ';',
  ':',
  '<',
  '>',
  ',',
];

// Check if a character is CJK (Chinese, Japanese, Korean) or should skip scramble animation
const isCJKOrSpecial = (char: string): boolean => {
  if (!char || char.length === 0) return true; // Skip empty chars

  // Use codePointAt for proper Unicode handling (supports surrogate pairs)
  const code = char.codePointAt(0) || 0;

  return (
    // CJK Unified Ideographs (Chinese, Japanese Kanji, Korean Hanja)
    (code >= 0x4e00 && code <= 0x9fff) ||
    // CJK Unified Ideographs Extension A
    (code >= 0x3400 && code <= 0x4dbf) ||
    // CJK Unified Ideographs Extension B-F (rare characters, emoji components)
    (code >= 0x20000 && code <= 0x2ceaf) ||
    // CJK Compatibility Ideographs
    (code >= 0xf900 && code <= 0xfaff) ||
    // CJK Symbols and Punctuation
    (code >= 0x3000 && code <= 0x303f) ||
    // Fullwidth Forms (fullwidth ASCII, punctuation)
    (code >= 0xff00 && code <= 0xffef) ||
    // Japanese Hiragana
    (code >= 0x3040 && code <= 0x309f) ||
    // Japanese Katakana
    (code >= 0x30a0 && code <= 0x30ff) ||
    // Korean Hangul Syllables
    (code >= 0xac00 && code <= 0xd7af) ||
    // Korean Hangul Jamo
    (code >= 0x1100 && code <= 0x11ff) ||
    // Emoji and symbols (skip scramble for these too)
    (code >= 0x1f000 && code <= 0x1ffff) ||
    // Misc symbols
    (code >= 0x2600 && code <= 0x26ff) ||
    // Dingbats
    (code >= 0x2700 && code <= 0x27bf)
  );
};

export class TextAnimator {
  textElement: HTMLElement;
  splitter!: TextSplitter;
  originalChars!: string[];
  activeAnimations: globalThis.Animation[] = [];
  activeTimeouts: ReturnType<typeof setTimeout>[] = [];

  constructor(textElement: HTMLElement) {
    if (!textElement || !(textElement instanceof HTMLElement)) {
      throw new Error('Invalid text element provided.');
    }

    this.textElement = textElement;
    this.splitText();
  }

  private splitText() {
    this.splitter = new TextSplitter(this.textElement, {
      splitTypeTypes: ['words', 'chars'],
    });
    this.originalChars = this.splitter.getChars().map((char) => char.textContent || '');
  }

  animate() {
    this.reset();

    const chars = this.splitter.getChars();

    chars.forEach((char, position) => {
      const initialText = char.textContent || '';

      char.style.opacity = '1';
      char.style.display = 'inline-block';
      char.style.position = 'relative';

      const animation = char.animate(
        [
          {
            opacity: 1,
            color: '#666',
            fontFamily: 'Cash Sans Mono',
            fontWeight: '300',
          },
          {
            opacity: 0.5,
            color: '#999',
          },
          {
            opacity: 1,
            color: 'inherit',
            fontFamily: 'inherit',
            fontWeight: 'inherit',
          },
        ],
        {
          duration: 300, // Total duration for all iterations
          easing: 'ease-in-out',
          delay: position * 30, // Stagger the start of each animation
          iterations: 1,
        }
      );

      this.activeAnimations.push(animation);

      let iteration = 0;
      const maxIterations = 2;

      // Skip scramble animation for Chinese characters
      const shouldScramble = !isCJKOrSpecial(initialText);

      const animateCharacterChange = () => {
        if (shouldScramble && iteration < maxIterations) {
          char.textContent =
            lettersAndSymbols[Math.floor(Math.random() * lettersAndSymbols.length)];
          const timeoutId = setTimeout(animateCharacterChange, 100);
          this.activeTimeouts.push(timeoutId);
          iteration++;
        } else {
          char.textContent = initialText;
        }
      };

      const timeoutId = setTimeout(animateCharacterChange, position * 30);
      this.activeTimeouts.push(timeoutId);

      animation.onfinish = () => {
        char.textContent = initialText;
        char.style.color = '';
        char.style.fontFamily = '';
        char.style.opacity = '1';
      };
    });
  }

  reset() {
    // Clear all timeouts
    this.activeTimeouts.forEach((timeoutId) => clearTimeout(timeoutId));
    this.activeTimeouts = [];

    // Cancel all animations
    this.activeAnimations.forEach((animation) => animation.cancel());
    this.activeAnimations = [];

    // Reset text content
    const chars = this.splitter.getChars();
    chars.forEach((char, index) => {
      if (this.originalChars[index]) {
        char.textContent = this.originalChars[index];
      }
    });
  }

  // Clean up all resources
  destroy() {
    this.reset();
    if (this.splitter) {
      this.splitter.destroy();
    }
  }
}

interface UseTextAnimatorProps {
  text: string;
}

export function useTextAnimator({ text }: UseTextAnimatorProps) {
  const elementRef = useRef<HTMLDivElement>(null);
  const animator = useRef<TextAnimator | null>(null);

  useEffect(() => {
    if (!elementRef.current) return;

    // Create animator
    animator.current = new TextAnimator(elementRef.current);

    // Small delay to ensure content is ready
    const timeoutId = setTimeout(() => {
      animator.current?.animate();
    }, 100);

    // Cleanup - use destroy() to properly clean up ResizeObserver and prevent memory leaks
    return () => {
      window.clearTimeout(timeoutId);
      if (animator.current) {
        animator.current.destroy();
        animator.current = null;
      }
    };
  }, [text]); // Re-run when text changes

  return elementRef;
}
