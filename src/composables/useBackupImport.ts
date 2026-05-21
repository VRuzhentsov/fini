import { computed, onMounted, ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { useBackupStore, type BackupConflict, type BackupConflictResolutionInput, type BackupSpaceMappingInput, type BackupSpaceMappingRequest } from "../stores/backup";
import { useQuestStore } from "../stores/quest";
import { useSpaceStore, isBuiltinSpace } from "../stores/space";
import { useToast } from "./useToast";

export function useBackupImport(onSuccess?: () => void) {
  const backupStore = useBackupStore();
  const spaceStore = useSpaceStore();
  const questStore = useQuestStore();
  const toast = useToast();

  const path = ref<string | null>(null);
  const mappings = ref<BackupSpaceMappingInput[]>([]);
  const activeMapping = ref<BackupSpaceMappingRequest | null>(null);
  const mappingIndex = ref(0);
  const totalMappings = ref(0);
  const conflicts = ref<BackupConflict[]>([]);
  const showConflicts = ref(false);
  const loading = ref(false);
  const applying = ref(false);
  const error = ref<string | null>(null);

  const selectableSpaces = computed(() => spaceStore.spaces.filter((s) => !isBuiltinSpace(s.id)));

  onMounted(() => { void spaceStore.fetchSpaces(); });

  async function startImport() {
    error.value = null;
    let selected: string | null;
    // E2E test hook: bypasses native OS dialog
    const e2eHook = (window as Window & { __FINI_E2E_OPEN_PATH__?: string | null }).__FINI_E2E_OPEN_PATH__;
    if (e2eHook !== undefined) {
      selected = e2eHook ?? null;
      delete (window as Window & { __FINI_E2E_OPEN_PATH__?: string | null }).__FINI_E2E_OPEN_PATH__;
    } else {
      const picked = await open({
        multiple: false,
        filters: [{ name: "Fini backup", extensions: ["zip"] }],
      });
      selected = Array.isArray(picked) ? null : picked;
    }
    if (!selected) return;
    path.value = selected;
    mappings.value = [];
    mappingIndex.value = 0;
    await runPreflight();
  }

  async function runPreflight() {
    if (!path.value) return;
    loading.value = true;
    error.value = null;
    try {
      const result = await backupStore.preflightImport(path.value, mappings.value);
      const pending = result.required_space_mappings;
      totalMappings.value = pending.length;

      if (pending.length > 0) {
        activeMapping.value = pending[0];
        conflicts.value = [];
        showConflicts.value = false;
      } else if (result.conflicts.length > 0) {
        activeMapping.value = null;
        conflicts.value = result.conflicts;
        showConflicts.value = true;
      } else {
        // No mappings needed, no conflicts — apply immediately
        activeMapping.value = null;
        conflicts.value = [];
        showConflicts.value = false;
        await applyImport([]);
      }
    } catch (err) {
      error.value = String(err);
      toast.error("Backup import failed validation");
    } finally {
      loading.value = false;
    }
  }

  async function confirmMapping(resolution: { mode: "create_new" | "use_existing"; local_space_id?: string }) {
    if (!activeMapping.value) return;
    mappings.value = [
      ...mappings.value,
      {
        backup_space_id: activeMapping.value.backup_space_id,
        mode: resolution.mode,
        local_space_id: resolution.local_space_id,
      },
    ];
    mappingIndex.value++;
    await runPreflight();
  }

  function cancelImport() {
    path.value = null;
    mappings.value = [];
    activeMapping.value = null;
    conflicts.value = [];
    showConflicts.value = false;
    error.value = null;
  }

  async function applyImport(resolutions: BackupConflictResolutionInput[] = []) {
    if (!path.value) return;
    applying.value = true;
    error.value = null;
    try {
      await backupStore.applyImport(path.value, mappings.value, resolutions);
      await Promise.all([spaceStore.fetchSpaces(), questStore.fetchQuests(), questStore.fetchActiveQuest()]);
      toast.show("Backup imported", "success", 3000);
      cancelImport();
      onSuccess?.();
    } catch (err) {
      error.value = String(err);
      toast.error("Backup import failed");
    } finally {
      applying.value = false;
      showConflicts.value = false;
    }
  }

  return {
    path,
    activeMapping,
    mappingIndex,
    totalMappings,
    conflicts,
    showConflicts,
    loading,
    applying,
    error,
    selectableSpaces,
    startImport,
    confirmMapping,
    cancelImport,
    applyImport,
  };
}
