<script setup lang="ts">
import SettingsListGroup from "./SettingsListGroup.vue";
import SettingsListItem from "./SettingsListItem.vue";

defineProps<{
  version: string;
  sourceUrl: string;
}>();
</script>

<template>
  <section class="rounded-xl bg-base-200 p-3">
    <h2 class="mb-3 text-sm font-semibold uppercase tracking-wide opacity-70">About</h2>
    <SettingsListGroup>
      <SettingsListItem>
        <template #start>
          <span class="font-medium">Version</span>
        </template>
        <template #end>
          <span class="opacity-70">{{ version }}</span>
        </template>
      </SettingsListItem>
      <SettingsListItem :href="sourceUrl" class="text-primary">
        <template #start>
          <span class="font-medium">Source code</span>
        </template>
        <template #end>
          <span class="opacity-60">↗</span>
        </template>
      </SettingsListItem>
    </SettingsListGroup>
  </section>
</template>
