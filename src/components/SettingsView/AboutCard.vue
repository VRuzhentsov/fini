<script setup lang="ts">
defineProps<{
  version: string;
  sourceUrl: string;
}>();
</script>

<template>
  <section class="rounded-xl bg-base-200 p-3">
    <h2 class="mb-3 text-sm font-semibold uppercase tracking-wide opacity-70">About</h2>
    <div class="flex flex-col gap-2 rounded-lg bg-base-100 px-3 py-2 text-sm">
      <div class="flex items-center justify-between gap-3">
        <span class="font-medium">Version</span>
        <span class="opacity-70">{{ version }}</span>
      </div>
      <a
        :href="sourceUrl"
        target="_blank"
        rel="noreferrer noopener"
        class="flex items-center justify-between gap-3 text-primary"
      >
        <span class="font-medium">Source code</span>
        <span class="opacity-60">↗</span>
      </a>
    </div>
  </section>
</template>
