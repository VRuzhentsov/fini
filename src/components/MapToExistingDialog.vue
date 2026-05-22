<script setup lang="ts">
import { computed, ref } from "vue";
import { shortUuid } from "../utils/shortUuid";
import { SPACE_COLOR_CLASS, type Space } from "../stores/space";

const props = defineProps<{
  context: { kind: "backup-space" | "peer-space"; name: string };
  spaces: Space[];
}>();

const emit = defineEmits<{
  cancel: [];
  confirm: [spaceId: string];
}>();

const selectedId = ref("");
const canConfirm = computed(() => Boolean(selectedId.value));

const blurb = computed(() =>
  props.context.kind === "peer-space"
    ? `Pick the local space to keep in sync with ${props.context.name}.`
    : `Pick the local space to merge ${props.context.name} into. Imported quests and series will be reassigned to it.`,
);

function spaceColor(id: string): string {
  const cls = SPACE_COLOR_CLASS[id];
  if (!cls) return "oklch(var(--bc) / 0.35)";
  return `var(--${cls.replace("space-color-", "space-color-")})`;
}

function confirmSelection() {
  if (canConfirm.value) emit("confirm", selectedId.value);
}
</script>

<template>
  <Teleport to="body">
    <div class="fixed inset-0 z-[1300] flex items-end justify-center p-3 sm:items-center sm:p-4" data-testid="map-to-existing-dialog">
      <button type="button" class="absolute inset-0 bg-black/60" aria-label="Close" @click="emit('cancel')"></button>
      <div class="relative w-full max-w-sm rounded-xl bg-base-100 shadow-2xl">
        <div class="p-4 pb-3">
          <h3 class="text-base font-semibold">Map to existing space</h3>
          <p class="mt-1 text-sm opacity-60">{{ blurb }}</p>
        </div>

        <ul class="max-h-64 overflow-y-auto border-t border-base-200" role="listbox" aria-label="Pick a local space">
          <li
            v-for="space in spaces"
            :key="space.id"
            role="option"
            :aria-selected="selectedId === space.id"
            class="flex cursor-pointer items-center gap-3 px-4 py-3 transition-colors hover:bg-base-200"
            :class="selectedId === space.id ? 'bg-base-200' : ''"
            @click="selectedId = space.id"
          >
            <span
              class="inline-block h-3 w-3 flex-shrink-0 rounded-full"
              :style="{ backgroundColor: spaceColor(space.id) }"
            ></span>
            <span class="flex-1 text-sm font-medium">{{ space.name }}</span>
            <span class="font-mono text-xs opacity-40" :title="space.id">{{ shortUuid(space.id, 8) }}</span>
            <span
              class="inline-flex h-4 w-4 items-center justify-center rounded-full border-2 flex-shrink-0"
              :class="selectedId === space.id ? 'border-primary bg-primary' : 'border-base-content/30'"
              aria-hidden="true"
            >
              <span v-if="selectedId === space.id" class="inline-block h-1.5 w-1.5 rounded-full bg-primary-content"></span>
            </span>
          </li>
          <li v-if="spaces.length === 0" class="px-4 py-6 text-center text-sm opacity-50">No local custom spaces yet.</li>
        </ul>

        <div class="flex items-center justify-end gap-2 border-t border-base-200 p-4 pt-3">
          <button class="btn btn-ghost btn-sm" @click="emit('cancel')">Cancel</button>
          <span class="flex-1"></span>
          <button class="btn btn-primary btn-sm" :disabled="!canConfirm" @click="confirmSelection">
            Map<template v-if="selectedId"> to {{ spaces.find(s => s.id === selectedId)?.name }}</template>
          </button>
        </div>
      </div>
    </div>
  </Teleport>
</template>
