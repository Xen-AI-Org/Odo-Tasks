import { defineConfig } from "vite";
import { realpathSync } from "node:fs";
import { fileURLToPath } from "node:url";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;
// On Windows, Desktop is commonly redirected into OneDrive. Resolve the
// workspace once so Vite does not mix the displayed and canonical paths.
const root = realpathSync(fileURLToPath(new URL(".", import.meta.url)));

// https://vite.dev/config/
export default defineConfig(async () => ({

  root,
  // The Tauri API already ships as browser-ready ESM. Keeping it out of
  // Vite's pre-bundler avoids a Node 25 optimizer crash on redirected
  // OneDrive workspaces while preserving normal dev-server behavior.
  optimizeDeps: {
    exclude: ["@tauri-apps/api/core", "@tauri-apps/api/event"],
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
