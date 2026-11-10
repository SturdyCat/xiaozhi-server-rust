/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** 覆盖默认同源相对路径 /api，一般只在独立联调时使用。 */
  readonly VITE_API_BASE_URL?: string;
  /** Vite dev proxy 的上游地址，默认 http://127.0.0.1:8000。 */
  readonly VITE_PROXY_TARGET?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
