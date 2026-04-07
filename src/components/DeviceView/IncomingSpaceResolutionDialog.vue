<script setup lang="ts">
import { shortUuid } from "../../utils/shortUuid";

const EMBEDDED_SPACE_IDS = new Set(["1", "2", "3"]);

function isEmbeddedSpaceId(spaceId: string): boolean {
  return EMBEDDED_SPACE_IDS.has(spaceId);
}

type ResolutionMode = "create_new" | "use_existing";

interface IncomingSpace {
  space_id: string;
  name: string;
}

interface SelectableSpace {
  id: string;
  name: string;
}

defineProps<{
  open: boolean;
  incomingSpace: IncomingSpace | null;
  resolutionMode: ResolutionMode;
  existingSpaceId: string;
  selectableSpaces: SelectableSpace[];
  resolving: boolean;
  canResolve: boolean;
  error: string | null;
}>();

const emit = defineEmits<{
  (e: "cancel"): void;
  (e: "setMode", mode: ResolutionMode): void;
  (e: "setExistingSpaceId", spaceId: string): void;
  (e: "confirm"): void;
}>();

function requestCancel() {
  emit("cancel");
}

function onSelectExistingSpace(event: Event) {
  const target = event.target as HTMLSelectElement;
  emit("setExistingSpaceId", target.value);
}
</script>

<template>
  <Teleport to="body">
    <div v-if="open && incomingSpace" class="fixed inset-0 z-[1200] flex items-end justify-center p-3 sm:items-center sm:p-4">
      <button
        type="button"
        class="absolute inset-0 bg-black/45"
        aria-label="Close dialog"
        @click="requestCancel"
      ></button>

      <div class="relative w-full max-w-md rounded-xl bg-base-100 p-4 shadow-2xl">
        <h3 class="text-base font-semibold">New shared space invite</h3>
        <p class="mt-2 text-sm opacity-70">
          Incoming space: <span class="font-medium">{{ incomingSpace.name }}</span>
        </p>
        <p class="font-mono text-xs opacity-60" :title="incomingSpace.space_id">{{ shortUuid(incomingSpace.space_id) }}</p>

        <div class="mt-4 flex flex-wrap gap-2">
          <button
            class="btn btn-sm"
            :class="resolutionMode === 'create_new' ? 'btn-primary' : 'btn-ghost'"
            @click="emit('setMode', 'create_new')"
          >
            Create
          </button>
          <button
            class="btn btn-sm"
            :class="resolutionMode === 'use_existing' ? 'btn-primary' : 'btn-ghost'"
            @click="emit('setMode', 'use_existing')"
          >
            Select space to map
          </button>
        </div>

        <div v-if="resolutionMode === 'use_existing'" class="mt-3">
          <select
            class="select select-bordered w-full"
            :value="existingSpaceId"
            @change="onSelectExistingSpace"
          >
            <option disabled value="">Select local space</option>
            <option v-for="space in selectableSpaces" :key="space.id" :value="space.id">
              {{ space.name }}<template v-if="!isEmbeddedSpaceId(space.id)"> ({{ shortUuid(space.id) }})</template>
            </option>
          </select>
        </div>

        <div v-if="error" class="mt-3 text-xs text-error">{{ error }}</div>

        <div class="mt-4 flex justify-end gap-2">
          <button class="btn btn-ghost btn-sm" @click="requestCancel">Cancel</button>
          <button class="btn btn-primary btn-sm" :disabled="!canResolve || resolving" @click="emit('confirm')">
            {{ resolving ? "Resolving..." : resolutionMode === "create_new" ? "Create" : "Select space to map" }}
          </button>
        </div>
      </div>
    </div>
  </Teleport>
</template>
