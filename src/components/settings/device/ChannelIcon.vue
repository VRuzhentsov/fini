<script setup lang="ts">
// Heroicons v2 outline is the app's icon set and has no Bluetooth glyph, so
// the Bluetooth mark is the design bundle's own path (ui_kits/device-pairing
// /screens.jsx). Both are drawn here at Heroicons' stroke weight and cap
// style so the two channel rows look like siblings rather than one borrowed
// icon next to one native one.
defineProps<{ kind: "network" | "bluetooth" }>();

const PATHS = {
  network: "M4.5 11.5a10.5 10.5 0 0 1 15 0M7.75 15a6 6 0 0 1 8.5 0",
  bluetooth: "M12 3v18m0-18 4.5 4.5L8 15m4 6 4.5-4.5L8 9",
} as const;
</script>

<template>
  <svg
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.5"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <path :d="PATHS[kind]" />
    <!-- The dot under the network arcs -- without it the glyph reads as a
         generic signal fan rather than Wi-Fi. -->
    <circle v-if="kind === 'network'" cx="12" cy="18.5" r="1.4" fill="currentColor" stroke="none" />
  </svg>
</template>
