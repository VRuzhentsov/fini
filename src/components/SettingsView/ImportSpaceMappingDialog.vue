<script setup lang="ts">
import { ref } from "vue";
import { SPACE_COLOR_CLASS, type Space } from "../../stores/space";
import { shortUuid } from "../../utils/shortUuid";
import MapToExistingDialog from "../MapToExistingDialog.vue";

defineProps<{
  incoming: { backup_space_id: string; backup_space_name: string };
  localSpaces: Space[];
  index: number;
  total: number;
}>();

const emit = defineEmits<{
  cancel: [];
  resolve: [resolution: { mode: "create_new" | "use_existing"; local_space_id?: string }];
}>();

const pickerOpen = ref(false);

function spaceColor(id: string): string {
  const cls = SPACE_COLOR_CLASS[id];
  if (!cls) return "oklch(var(--bc) / 0.35)";
  return `var(--${cls})`;
}

function onCreate() {
  emit("resolve", { mode: "create_new" });
}

function onMap(spaceId: string) {
  pickerOpen.value = false;
  emit("resolve", { mode: "use_existing", local_space_id: spaceId });
}
</script>

<template>
  <Teleport to="body">
    <div class="fixed inset-0 z-[1200] flex items-end justify-center p-3 sm:items-center sm:p-4" data-testid="import-space-mapping-dialog">
      <button type="button" class="absolute inset-0 bg-black/45" aria-label="Close" @click="emit('cancel')"></button>
      <div class="relative w-full max-w-md rounded-xl bg-base-100 shadow-2xl">
        <div class="p-4 pb-3">
          <div class="flex items-start gap-3">
            <div class="flex-1">
              <h3 class="text-base font-semibold">Map incoming space</h3>
              <p class="mt-1 text-sm opacity-60">This backup contains a space you don't have locally. Create it as a new space, or merge its quests into one you already have.</p>
            </div>
            <span
              v-if="total > 1"
              class="rounded-full bg-base-200 px-2 py-0.5 text-xs font-semibold"
              :aria-label="`${index + 1} of ${total} spaces`"
              data-testid="mapping-counter"
            >{{ index + 1 }}/{{ total }}</span>
          </div>
        </div>

        <div class="border-t border-base-200 px-4 py-3">
          <div class="mb-3 flex items-center gap-2 text-sm">
            <span class="opacity-50">Backup space</span>
            <span
              class="inline-block h-2.5 w-2.5 flex-shrink-0 rounded-full"
              :style="{ backgroundColor: spaceColor(incoming.backup_space_id) }"
            ></span>
            <span class="font-medium">{{ incoming.backup_space_name }}</span>
            <span class="font-mono text-xs opacity-40" :title="incoming.backup_space_id">{{ shortUuid(incoming.backup_space_id, 8) }}</span>
          </div>

          <div class="flex flex-col gap-2">
            <div class="flex items-start gap-3 rounded-lg border border-base-300 p-3">
              <span class="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-full bg-primary/10 text-primary">
                <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 5v14M5 12h14"/></svg>
              </span>
              <div>
                <div class="text-sm font-medium">Create</div>
                <div class="mt-0.5 text-xs opacity-60">A new local space <strong>{{ incoming.backup_space_name }}</strong> is created using the backup's ID. Quests and series keep their IDs.</div>
              </div>
            </div>
            <div class="flex items-start gap-3 rounded-lg border border-base-300 p-3">
              <span class="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-full bg-base-200">
                <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3 12h18M14 5l7 7-7 7"/></svg>
              </span>
              <div>
                <div class="text-sm font-medium">Map to existing</div>
                <div class="mt-0.5 text-xs opacity-60">Reassign imported quests and series to a local space you already have.</div>
              </div>
            </div>
          </div>
        </div>

        <div class="flex items-center gap-2 border-t border-base-200 px-4 py-3">
          <button class="btn btn-ghost btn-sm" @click="emit('cancel')">Cancel</button>
          <span class="flex-1"></span>
          <button class="btn btn-sm" @click="pickerOpen = true">Map to existing</button>
          <button class="btn btn-primary btn-sm" @click="onCreate">Create</button>
        </div>
      </div>
    </div>

    <MapToExistingDialog
      v-if="pickerOpen"
      :context="{ kind: 'backup-space', name: incoming.backup_space_name }"
      :spaces="localSpaces"
      @cancel="pickerOpen = false"
      @confirm="onMap"
    />
  </Teleport>
</template>
