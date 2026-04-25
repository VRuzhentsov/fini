import { createApp } from "vue";
import { createPinia } from "pinia";
import { invoke } from "@tauri-apps/api/core";
import router from "./router";
import i18n from "./i18n";
import App from "./App.vue";
import "./style.css";

async function bootstrap() {
  let theme = window.matchMedia("(prefers-color-scheme: dark)").matches
    ? "dark"
    : "light";

  try {
    const hint = await invoke<string>("theme_hint");
    if (hint === "dark" || hint === "light") {
      theme = hint;
    }
  } catch {
    // Fall back to the webview media query outside Tauri.
  }

  document.documentElement.setAttribute("data-theme", theme);
  document.documentElement.style.colorScheme = theme;
  document.body?.setAttribute("data-theme", theme);
  if (document.body) {
    document.body.style.colorScheme = theme;
  }

  createApp(App).use(createPinia()).use(router).use(i18n).mount("#app");
}

void bootstrap();
