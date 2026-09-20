<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { save } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { useBackupStore } from "../../stores/backup";
import { useSpaceStore, SPACE_COLOR_CLASS } from "../../stores/space";
import { useQuestStore } from "../../stores/quest";
import { useToast } from "../../composables/useToast";

const emit = defineEmits<{ close: []; exported: [path: string] }>();

const backupStore = useBackupStore();
const spaceStore = useSpaceStore();
const questStore = useQuestStore();
const toast = useToast();

const selected = ref<Set<string>>(new Set());
const saving = ref(false);
const error = ref<string | null>(null);

const canExport = computed(() => selected.value.size > 0 && !saving.value);
const allSelected = computed(() => spaceStore.spaces.length > 0 && selected.value.size === spaceStore.spaces.length);

const today = new Date().toISOString().slice(0, 10);
const filename = `fini-backup-${today}.zip`;

function questsInSpace(spaceId: string): number {
  return questStore.quests.filter((q) => q.space_id === spaceId).length;
}

function spaceColor(id: string): string {
  const cls = SPACE_COLOR_CLASS[id];
  if (!cls) return "oklch(var(--bc) / 0.35)";
  return `var(--${cls})`;
}

function toggleSpace(id: string) {
  const next = new Set(selected.value);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  selected.value = next;
}

function toggleAll() {
  if (allSelected.value) {
    selected.value = new Set();
  } else {
    selected.value = new Set(spaceStore.spaces.map((s) => s.id));
  }
}

async function exportSelected() {
  if (!canExport.value) return;
  error.value = null;
  let path: string | null;
  // E2E test hook: bypasses native OS dialog
  const e2eHook = (window as Window & { __FINI_E2E_SAVE_PATH__?: string | null }).__FINI_E2E_SAVE_PATH__;
  if (e2eHook !== undefined) {
    path = e2eHook ?? null;
    delete (window as Window & { __FINI_E2E_SAVE_PATH__?: string | null }).__FINI_E2E_SAVE_PATH__;
  } else {
    try {
      path = await save({
        defaultPath: filename,
        filters: [{ name: "Fini backup", extensions: ["zip"] }],
      });
    } catch (err) {
      error.value = String(err);
      toast.error("Could not open file picker");
      return;
    }
  }
  if (!path) return;

  saving.value = true;
  try {
    await backupStore.exportBackup(path, Array.from(selected.value));
    toast.show("Backup exported", "success", 4000, {
      label: "Open location",
      onClick: () => void revealItemInDir(path),
    });
    emit("exported", path);
    emit("close");
  } catch (err) {
    error.value = String(err);
    toast.error("Backup export failed");
  } finally {
    saving.value = false;
  }
}

onMounted(() => {
  void spaceStore.fetchSpaces();
  void questStore.fetchQuests();
});
</script>

<template>
  <Teleport to="body">
    <div class="fixed inset-0 z-[1200] flex items-end justify-center p-3 sm:items-center sm:p-4" data-testid="export-spaces-dialog">
      <button type="button" class="absolute inset-0 bg-black/45" aria-label="Close export dialog" @click="emit('close')"></button>
      <div class="relative w-full max-w-md rounded-xl bg-base-100 shadow-2xl">
        <div class="p-4 pb-3">
          <h3 class="text-base font-semibold">Export backup</h3>
          <p class="mt-1 text-sm opacity-60">Pick the spaces to include. The backup contains every quest in those spaces — active, completed, and abandoned — plus their quest series.</p>
        </div>

        <div class="border-t border-base-200 px-4 py-3">
          <div class="mb-2 flex items-center justify-between">
            <span class="text-xs font-medium uppercase tracking-wide opacity-50">Spaces</span>
            <button class="btn btn-ghost btn-xs" @click="toggleAll">
              {{ allSelected ? "Clear all" : "Select all" }}
            </button>
          </div>
          <ul class="flex flex-col gap-1" role="listbox" aria-label="Spaces to back up">
            <li
              v-for="space in spaceStore.spaces"
              :key="space.id"
              role="option"
              :aria-selected="selected.has(space.id)"
              class="flex cursor-pointer items-center gap-3 rounded-lg border border-base-300 px-3 py-2 transition-colors"
              :class="selected.has(space.id) ? 'border-primary/40 bg-primary/5' : 'hover:bg-base-200'"
              @click="toggleSpace(space.id)"
            >
              <input
                type="checkbox"
                class="checkbox checkbox-sm checkbox-primary"
                :checked="selected.has(space.id)"
                @change="toggleSpace(space.id)"
                @click.stop
              />
              <span
                class="inline-block h-2.5 w-2.5 flex-shrink-0 rounded-full"
                :style="{ backgroundColor: spaceColor(space.id) }"
              ></span>
              <span class="flex-1 text-sm font-medium">{{ space.name }}</span>
              <span class="text-xs opacity-40">{{ questsInSpace(space.id) }} quest{{ questsInSpace(space.id) === 1 ? "" : "s" }}</span>
            </li>
          </ul>
        </div>

        <div class="mx-4 mb-3 flex items-start gap-2 rounded-lg bg-warning/10 px-3 py-2 text-xs text-base-content/80">
          <svg class="mt-0.5 h-3.5 w-3.5 flex-shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <circle cx="12" cy="12" r="9"/><path d="M12 8v5"/><circle cx="12" cy="16" r="0.6" fill="currentColor"/>
          </svg>
          <span class="opacity-80">The backup file contains your quest data in plain SQLite. It is not password-protected — keep it somewhere safe.</span>
        </div>

        <div v-if="error" class="mx-4 mb-3 text-xs text-error">{{ error }}</div>

        <div class="flex items-center gap-2 border-t border-base-200 px-4 py-3">
          <span class="text-xs opacity-40">Saves as <code class="font-mono">{{ filename }}</code></span>
          <span class="flex-1"></span>
          <button class="btn btn-ghost btn-sm" @click="emit('close')">Cancel</button>
          <button class="btn btn-primary btn-sm" :disabled="!canExport" @click="void exportSelected()">
            <template v-if="saving">Exporting…</template>
            <template v-else>Export{{ selected.size ? ` ${selected.size} space${selected.size === 1 ? "" : "s"}` : "" }}</template>
          </button>
        </div>
      </div>
    </div>
  </Teleport>
</template>
