<script setup lang="ts">
import { invoke } from "@tauri-apps/api/core";
import { onMounted, ref } from "vue";
import SettingsListGroup from "./SettingsListGroup.vue";
import SettingsListItem from "./SettingsListItem.vue";

const supported = ref(false);
const enabled = ref(true);
const loading = ref(false);
const saving = ref(false);
const error = ref<string | null>(null);

onMounted(async () => {
  loading.value = true;
  try {
    supported.value = await invoke<boolean>("startup_auto_update_supported");
    if (supported.value) enabled.value = await invoke<boolean>("get_auto_update_enabled");
  } catch (caught) {
    error.value = String(caught);
  } finally {
    loading.value = false;
  }
});

async function setEnabled(event: Event) {
  const next = (event.target as HTMLInputElement).checked;
  const previous = enabled.value;
  enabled.value = next;
  saving.value = true;
  error.value = null;
  try {
    enabled.value = await invoke<boolean>("set_auto_update_enabled", { enabled: next });
  } catch (caught) {
    enabled.value = previous;
    error.value = String(caught);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <section v-if="supported" class="rounded-xl bg-base-200 p-3" data-testid="settings-updates">
    <h2 class="mb-3 text-sm font-semibold uppercase tracking-wide opacity-70">Updates</h2>
    <SettingsListGroup>
      <SettingsListItem>
        <template #start><div><span class="block font-medium">Automatic updates</span><span class="block text-xs opacity-60">When this is off, Fini will not install updates automatically on the next restart.</span></div></template>
        <template #end><input type="checkbox" class="toggle toggle-primary" data-testid="automatic-updates-toggle" aria-label="Automatic updates" :checked="enabled" :disabled="loading || saving" @change="setEnabled" /></template>
      </SettingsListItem>
    </SettingsListGroup>
    <div v-if="error" class="mt-2 text-xs text-error">{{ error }}</div>
  </section>
</template>
