// Global declarations for Tauri webview environment.
//
// Тонкая прослойка чтобы main.ts собирался при `tsc --noEmit` пока
// мы не типизировали code базу детально. На след. итерациях заменим
// `any` на конкретные типы из @tauri-apps/api.

declare global {
  interface Window {
    __TAURI__: {
      core: {
        invoke: <T = unknown>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
      };
      event: {
        listen: <T = unknown>(
          event: string,
          handler: (event: { payload: T }) => void,
        ) => Promise<() => void>;
      };
      window: {
        getCurrentWindow: () => any;
      };
    };
    i18n: {
      t: (path: string, vars?: Record<string, unknown>) => string;
      setLanguage: (lang: 'ru' | 'en') => void;
      currentLanguage: () => 'ru' | 'en';
    };
  }
}

export {};
