/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_PLATFORM?: 'web' | 'electron';
  readonly VITE_APP_VERSION?: string;
}

// eslint-disable-next-line @typescript-eslint/no-unused-vars -- Required for TypeScript declaration merging with Vite's ImportMeta
interface ImportMeta {
  readonly env: ImportMetaEnv;
}

declare module '*.json' {
  const value: Record<string, unknown>;
  export default value;
}

declare module '*.png' {
  const value: string;
  export default value;
}

declare module '*.jpg' {
  const value: string;
  export default value;
}

declare module '*.jpeg' {
  const value: string;
  export default value;
}

declare module '*.gif' {
  const value: string;
  export default value;
}

declare module '*.svg' {
  const value: string;
  export default value;
}

declare module '*.mp3' {
  const value: string;
  export default value;
}

declare module '*.mp4' {
  const value: string;
  export default value;
}

declare module '*.md?raw' {
  const value: string;
  export default value;
}

// Extend Window interface to include global recipe creation flag
declare global {
  interface Window {
    isCreatingRecipe?: boolean;
  }
}

export {};
