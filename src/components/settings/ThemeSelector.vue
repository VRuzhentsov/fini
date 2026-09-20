<script setup lang="ts">
import { ref, onMounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import SettingsListGroup from "./SettingsListGroup.vue";
import SettingsListItem from "./SettingsListItem.vue";

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

function themeOptionClass(mode: "system" | "light" | "dark") {
  return themeMode.value === mode ? "active" : "";
}
</script>

<template>
  <section class="rounded-xl bg-base-200 p-3">
    <h2 class="mb-3 text-sm font-semibold uppercase tracking-wide opacity-70">Appearance</h2>
    <SettingsListGroup>
      <SettingsListItem>
        <template #start>
          <span class="font-medium">Theme</span>
        </template>
        <template #end>
          <div class="dropdown dropdown-end">
            <button tabindex="0" class="inline-flex items-center gap-1 text-sm opacity-70">
              <span>{{ themeLabels[themeMode] }}</span>
              <span class="text-xs">⌄</span>
            </button>
            <ul tabindex="0" class="dropdown-content menu rounded-box bg-base-100 z-10 mt-2 w-48 p-2 shadow">
              <li><button :class="themeOptionClass('system')" @click="chooseTheme('system')">System</button></li>
              <li><button :class="themeOptionClass('light')" @click="chooseTheme('light')">Light</button></li>
              <li><button :class="themeOptionClass('dark')" @click="chooseTheme('dark')">Dark</button></li>
            </ul>
          </div>
        </template>
      </SettingsListItem>
    </SettingsListGroup>
  </section>
</template>
