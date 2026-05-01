<script setup lang="ts">
import { ref, onMounted } from "vue";
import { invoke } from "@tauri-apps/api/core";

const themeMode = ref<"system" | "light" | "dark">("system");

const themeLabels: Record<typeof themeMode.value, string> = {
  system: "System",
  light: "Light",
  dark: "Dark",
};

onMounted(() => {
  void loadThemeMode();
});

async function loadThemeMode() {
  try {
    const mode = await invoke<string>("get_theme_mode");
    if (mode === "system" || mode === "light" || mode === "dark") {
      themeMode.value = mode;
    }
  } catch {
    // Keep the default when the backend setting is unavailable.
  }
}

async function chooseTheme(mode: "system" | "light" | "dark") {
  themeMode.value = mode;

  try {
    const saved = await invoke<string>("set_theme_mode", { mode });
    if (saved === "system" || saved === "light" || saved === "dark") {
      themeMode.value = saved;
    }
  } catch {
    // Keep the current UI selection if the backend write fails.
  }
}
</script>

<template>
  <section class="rounded-xl bg-base-200 p-3">
    <h2 class="mb-3 text-sm font-semibold uppercase tracking-wide opacity-70">Appearance</h2>
    <div class="dropdown dropdown-right">
      <button tabindex="0" class="btn btn-outline btn-sm min-w-32 justify-between">
        <span>Theme</span>
        <span class="opacity-70">{{ themeLabels[themeMode] }}</span>
      </button>
      <ul tabindex="0" class="dropdown-content menu rounded-box bg-base-100 z-10 mt-2 w-48 p-2 shadow">
        <li><button :class="themeMode === 'system' ? 'active' : ''" @click="chooseTheme('system')">System</button></li>
        <li><button :class="themeMode === 'light' ? 'active' : ''" @click="chooseTheme('light')">Light</button></li>
        <li><button :class="themeMode === 'dark' ? 'active' : ''" @click="chooseTheme('dark')">Dark</button></li>
      </ul>
    </div>
  </section>
</template>
