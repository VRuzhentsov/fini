import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import tailwindcss from "@tailwindcss/vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [vue(), tailwindcss()],
  optimizeDeps: {
    // Vite's dep crawler can CPU-spin on the Tauri plugin graph and leave the
    // dev server accepting connections without responding, which whitescreens
    // the Tauri webview. Transform deps on demand instead.
    noDiscovery: true,
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
      //
      // `.claude/worktrees` holds full checkouts of this repo, so without it
      // Vite watches every agent worktree's `src/` and `dist/` alongside the
      // real one. Observed in a dev session: an unrelated worktree's build
      // wrote its `dist/index.html` and this app full-page reloaded, twice,
      // mid-session. A reload is not HMR -- it throws away the state you were
      // looking at, which is the thing HMR exists to keep.
      //
      // `dist` and `tmp` are this checkout's own outputs, for the same
      // reason: nothing in dev is served from either, and the only thing
      // watching them can do is reload the app while a build is writing.
      ignored: [
        "**/src-tauri/**",
        "**/.claude/worktrees/**",
        "**/dist/**",
        "**/tmp/**",
        "**/builddir/**",
        "**/.flatpak-builder/**",
        "**/flatpak-build/**",
      ],
    },
  },
}));
