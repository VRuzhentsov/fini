import { createApp } from "vue";
import { createPinia } from "pinia";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import router from "./router";
import i18n from "./i18n";
import App from "./App.vue";
import "./style.css";

type ThemeMode = "dark" | "light" | "system";
type ThemeSource = "bootstrap" | "native" | "portal";

const themeSourceRank: Record<ThemeSource, number> = {
  bootstrap: 0,
  native: 1,
  portal: 2,
};

let currentThemeSource: ThemeSource = "bootstrap";

function resolveTheme(theme: ThemeMode): "dark" | "light" {
  if (theme === "system") {
    return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  }

  return theme;
}

async function applyTheme(theme: ThemeMode, source: ThemeSource) {
  if (themeSourceRank[source] < themeSourceRank[currentThemeSource]) {
    return;
  }

  currentThemeSource = source;

  const resolved = resolveTheme(theme);
  document.documentElement.setAttribute("data-theme", resolved);
  document.documentElement.style.colorScheme = resolved;
  document.body?.setAttribute("data-theme", resolved);
  if (document.body) {
    document.body.style.colorScheme = resolved;
  }

  try {
    await getCurrentWindow().setTheme(resolved);
  } catch {
    // Keep the DOM theme even if the native window theme call is unavailable.
  }

  try {
    await invoke("sync_native_theme", { theme: resolved });
  } catch {
    // Keep the DOM theme even if the native sync call is unavailable.
  }
}

async function bootstrap() {
  let theme: ThemeMode = window.matchMedia("(prefers-color-scheme: dark)").matches
    ? "dark"
    : "light";

  try {
    const hint = await invoke<string>("theme_hint");
    if (hint === "dark" || hint === "light") {
      theme = hint;
    } else if (hint === "system") {
      theme = "system";
    }
  } catch {
    // Keep the webview media query when the backend hint is unavailable.
  }

  await applyTheme(theme, "bootstrap");

  createApp(App).use(createPinia()).use(router).use(i18n).mount("#app");

  try {
    await getCurrentWindow().onThemeChanged(({ payload }) => {
      if (payload === "dark" || payload === "light") {
        void applyTheme(payload, "native");
      }
    });
  } catch {
    // Keep the initial theme if the native theme listener is unavailable.
  }

  const unlistenTheme = await listen<string>("theme://changed", (event) => {
    if (event.payload === "dark" || event.payload === "light" || event.payload === "system") {
      void applyTheme(event.payload, "portal");
    }
  });

  window.addEventListener("beforeunload", () => {
    unlistenTheme();
  });
}

void bootstrap();
