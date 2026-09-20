<script setup lang="ts">
import { ref } from "vue";
import { useBackupImport } from "../../composables/useBackupImport";
import ExportSpacesDialog from "./ExportSpacesDialog.vue";
import ImportSpaceMappingDialog from "./ImportSpaceMappingDialog.vue";
import MergeConflictDialog from "./MergeConflictDialog.vue";
import SettingsListGroup from "./SettingsListGroup.vue";
import SettingsListItem from "./SettingsListItem.vue";

const backupImport = useBackupImport();
const showBackupExport = ref(false);
</script>

<template>
  <section class="rounded-xl bg-base-200 p-3" data-testid="settings-backup">
    <h2 class="mb-3 text-sm font-semibold uppercase tracking-wide opacity-70">Backup</h2>
    <div>
    <p class="mb-2 text-xs opacity-60">Save your spaces and quests to a portable file, or restore from one.</p>
    <SettingsListGroup>
      <SettingsListItem button data-testid="backup-export-row" @click="showBackupExport = true">
        <template #leading><svg class="h-5 w-5 flex-shrink-0 opacity-60" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 3v12"/><path d="M7 10l5 5 5-5"/><path d="M5 21h14"/></svg></template>
        <div><span class="block font-medium">Export backup</span><span class="block text-xs opacity-60">Saves a <code class="font-mono">.zip</code> with quests and quest series for the spaces you pick.</span></div>
        <template #trailing><span class="text-sm opacity-50">›</span></template>
      </SettingsListItem>
      <SettingsListItem button data-testid="backup-import-row" @click="void backupImport.startImport()">
        <template #leading><svg class="h-5 w-5 flex-shrink-0 opacity-60" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 21V9"/><path d="M7 14l5-5 5 5"/><path d="M5 3h14"/></svg></template>
        <div><span class="block font-medium">Import backup</span><span class="block text-xs opacity-60">Restore from a <code class="font-mono">.zip</code>. Conflicts will ask before overwriting.</span></div>
        <template #trailing><span class="text-sm opacity-50">›</span></template>
      </SettingsListItem>
    </SettingsListGroup>
    <div v-if="backupImport.error.value" class="mt-2 text-xs text-error">{{ backupImport.error.value }}</div>
    <ExportSpacesDialog v-if="showBackupExport" @close="showBackupExport = false" />
    <ImportSpaceMappingDialog v-if="backupImport.activeMapping.value" :incoming="backupImport.activeMapping.value" :local-spaces="backupImport.selectableSpaces.value" :index="backupImport.mappingIndex.value" :total="backupImport.totalMappings.value" @cancel="backupImport.cancelImport()" @resolve="(resolution) => void backupImport.confirmMapping(resolution)" />
    <MergeConflictDialog v-if="backupImport.showConflicts.value" :conflicts="backupImport.conflicts.value" @cancel="backupImport.cancelImport()" @apply="(resolution) => void backupImport.applyImport(resolution)" />
    </div>
  </section>
</template>
