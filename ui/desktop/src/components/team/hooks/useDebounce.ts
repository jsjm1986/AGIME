// Debounce and throttle utilities for optimizing API calls

import { useState, useEffect, useRef, useCallback } from 'react';

/**
 * Debounce hook - delays execution until after wait milliseconds
 * have elapsed since the last time the debounced function was invoked
 */
export function useDebounce<T>(value: T, delay: number): T {
  const [debouncedValue, setDebouncedValue] = useState<T>(value);

  useEffect(() => {
    const timer = setTimeout(() => {
      setDebouncedValue(value);
    }, delay);

    return () => {
      clearTimeout(timer);
    };
  }, [value, delay]);

  return debouncedValue;
}

/**
 * Debounced callback hook - returns a debounced version of the callback
 */
export function useDebouncedCallback<T extends (...args: unknown[]) => unknown>(
  callback: T,
  delay: number
): T {
  const timeoutRef = useRef<NodeJS.Timeout | null>(null);
  const callbackRef = useRef(callback);

  // Update callback ref on each render
  useEffect(() => {
    callbackRef.current = callback;
  }, [callback]);

  const debouncedCallback = useCallback(
    (...args: Parameters<T>) => {
      if (timeoutRef.current) {
        clearTimeout(timeoutRef.current);
      }

      timeoutRef.current = setTimeout(() => {
        callbackRef.current(...args);
      }, delay);
    },
    [delay]
  ) as T;

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      if (timeoutRef.current) {
        clearTimeout(timeoutRef.current);
      }
    };
  }, []);

  return debouncedCallback;
}

/**
 * Throttle hook - limits execution to once per wait milliseconds
 */
export function useThrottle<T>(value: T, limit: number): T {
  const [throttledValue, setThrottledValue] = useState<T>(value);
  const lastRan = useRef(Date.now());

  useEffect(() => {
    const handler = setTimeout(() => {
      if (Date.now() - lastRan.current >= limit) {
        setThrottledValue(value);
        lastRan.current = Date.now();
      }
    }, limit - (Date.now() - lastRan.current));

    return () => {
      clearTimeout(handler);
    };
  }, [value, limit]);

  return throttledValue;
}

/**
 * Async debounce - for async functions, cancels pending calls
 */
export function useAsyncDebounce<T extends (...args: unknown[]) => Promise<unknown>>(
  callback: T,
  delay: number
): { execute: T; cancel: () => void; pending: boolean } {
  const [pending, setPending] = useState(false);
  const timeoutRef = useRef<NodeJS.Timeout | null>(null);
  const callbackRef = useRef(callback);
  const abortControllerRef = useRef<AbortController | null>(null);

  useEffect(() => {
    callbackRef.current = callback;
  }, [callback]);

  const cancel = useCallback(() => {
    if (timeoutRef.current) {
      clearTimeout(timeoutRef.current);
      timeoutRef.current = null;
    }
    if (abortControllerRef.current) {
      abortControllerRef.current.abort();
      abortControllerRef.current = null;
    }
    setPending(false);
  }, []);

  const execute = useCallback(
    ((...args: Parameters<T>) => {
      cancel();
      setPending(true);

      return new Promise((resolve, reject) => {
        timeoutRef.current = setTimeout(async () => {
          abortControllerRef.current = new AbortController();
          try {
            const result = await callbackRef.current(...args);
            setPending(false);
            resolve(result);
          } catch (error) {
            setPending(false);
            reject(error);
          }
        }, delay);
      });
    }) as T,
    [delay, cancel]
  );

  useEffect(() => {
    return cancel;
  }, [cancel]);

  return { execute, cancel, pending };
}
