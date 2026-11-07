/// <reference types="vite/client" />
interface ImportMetaEnv {
  readonly VITE_BRIDGE?: "mock" | "browser";
  readonly VITE_PRISM_TWIN_BASE?: string;
  readonly VITE_PRISM_CHAT_API?: string;
  readonly VITE_PRISM_OAUTH_AUTH?: string;
  readonly VITE_PRISM_OAUTH_TOKEN?: string;
  readonly VITE_PRISM_USERINFO?: string;
  readonly VITE_PRISM_RELAY_WS?: string;
  readonly VITE_PRISM_CLIENT_ID?: string;
  readonly VITE_PRISM_TWIN_USER?: string;
}
interface ImportMeta { readonly env: ImportMetaEnv }

// The PDF.js worker module, imported on the main thread (see Viewers.tsx).
declare module "pdfjs-dist/legacy/build/pdf.worker.min.mjs";
