<script setup lang="ts">
import { invoke } from "@tauri-apps/api/core";
import { computed, ref, onMounted } from "vue";
import packageJson from "../../package.json";
import AboutCard from "../components/SettingsView/AboutCard.vue";
import ExportSpacesDialog from "../components/SettingsView/ExportSpacesDialog.vue";
import ImportSpaceMappingDialog from "../components/SettingsView/ImportSpaceMappingDialog.vue";
import MergeConflictDialog from "../components/SettingsView/MergeConflictDialog.vue";
import SettingsListGroup from "../components/SettingsView/SettingsListGroup.vue";
import SettingsListItem from "../components/SettingsView/SettingsListItem.vue";
import ThemeSelector from "../components/SettingsView/ThemeSelector.vue";
import { useSpaceStore, isBuiltinSpace } from "../stores/space";
import { useDeviceStore, type PairedDevice } from "../stores/device";
import { useContextMenu, type MenuItem } from "../composables/useContextMenu";
import { useBackupImport } from "../composables/useBackupImport";
import ActionsBtn from "../components/ActionsBtn.vue";
import { PencilIcon, TrashIcon } from "@heroicons/vue/24/outline";

const spaceStore = useSpaceStore();
const deviceStore = useDeviceStore();
const contextMenu = useContextMenu();

const backupImport = useBackupImport();

function openSpaceMenu(e: MouseEvent, spaceId: string, spaceName: string) {
  const items: MenuItem[] = [{ label: "Edit", icon: PencilIcon, action: () => startEdit(spaceId, spaceName) }];
  if (!isBuiltinSpace(spaceId)) {
    items.push({ separator: true });
    items.push({ label: "Delete", icon: TrashIcon, danger: true, action: () => spaceStore.deleteSpace(spaceId) });
  }
  contextMenu.open(e, items);
}

const newSpaceName = ref("");
const editingId = ref<string | null>(null);
const editingName = ref("");
const settingsSearchQuery = ref("");
const showBackupExport = ref(false);
const autoUpdatesSupported = ref(false);
const autoUpdatesEnabled = ref(true);
const autoUpdatesLoading = ref(false);
const autoUpdatesSaving = ref(false);
const autoUpdatesError = ref<string | null>(null);
const appVersion = packageJson.version;
const sourceUrl = "https://github.com/VRuzhentsov/fini";

type SettingsSearchAction = "overview" | "export-backup" | "import-backup";

interface SettingsSearchResult {
  id: string;
  title: string;
  description?: string;
  action?: SettingsSearchAction;
}

interface SettingsSearchGroup {
  id: string;
  title: string;
  results: SettingsSearchResult[];
}

const normalizedSettingsSearchQuery = computed(() => settingsSearchQuery.value.trim().toLocaleLowerCase());

function matchesSettingsSearch(parts: Array<string | null | undefined>) {
  const query = normalizedSettingsSearchQuery.value;
  if (!query) return true;
  return parts.some((part) => part?.toLocaleLowerCase().includes(query));
}

function visibleSearchResults(results: SettingsSearchResult[]) {
  return results.filter((result) => matchesSettingsSearch([result.title, result.description]));
}

const renderLists = computed(() => ({
  spaces: spaceStore.spaces,
  pairedDevices: deviceStore.pairedDevices,
  searchResultGroups: [
    {
      id: "spaces",
      title: "Spaces",
      results: visibleSearchResults(spaceStore.spaces.map((space) => ({
        id: `space-${space.id}`,
        title: space.name,
        description: "Manage named contexts",
        action: "overview" as const,
      }))),
    },
    {
      id: "devices",
      title: "Devices",
      results: visibleSearchResults(deviceStore.pairedDevices.map((device) => ({
        id: `device-${device.peer_device_id}`,
        title: device.display_name,
        description: devicePresenceLabel(device),
        action: "overview" as const,
      }))),
    },
    {
      id: "appearance",
      title: "Appearance",
      results: visibleSearchResults([{ id: "theme", title: "Theme", description: "System, Light, or Dark", action: "overview" }]),
    },
    {
      id: "updates",
      title: "Updates",
      results: autoUpdatesSupported.value
        ? visibleSearchResults([{ id: "automatic-updates", title: "Automatic updates", description: "Install updates automatically on restart", action: "overview" }])
        : [],
    },
    {
      id: "backup",
      title: "Backup",
      results: visibleSearchResults([
        { id: "export-backup", title: "Export backup", description: "Save spaces and quests to a portable file", action: "export-backup" },
        { id: "import-backup", title: "Import backup", description: "Restore from a portable backup file", action: "import-backup" },
      ]),
    },
    {
      id: "about",
      title: "About",
      results: visibleSearchResults([{ id: "version", title: "Version", description: appVersion, action: "overview" }]),
    },
  ].filter((group): group is SettingsSearchGroup => group.results.length > 0),
}));

const renderFlags = computed(() => ({
  settingsOverview: !normalizedSettingsSearchQuery.value,
  settingsSearchResults: Boolean(normalizedSettingsSearchQuery.value) && renderLists.value.searchResultGroups.length > 0,
  spacesError: Boolean(spaceStore.error),
  spaceEditor: (spaceId: string) => editingId.value === spaceId,
  emptyPairedDevices: deviceStore.pairedDevices.length === 0,
  automaticUpdatesSection: autoUpdatesSupported.value,
  automaticUpdatesError: Boolean(autoUpdatesError.value),
  backupImportError: Boolean(backupImport.error.value),
  settingsSearchEmptyState: Boolean(normalizedSettingsSearchQuery.value) && renderLists.value.searchResultGroups.length === 0,
  backupExportDialog: showBackupExport.value,
  backupImportMappingDialog: Boolean(backupImport.activeMapping.value),
  backupMergeConflictDialog: Boolean(backupImport.showConflicts.value),
}));

onMounted(() => {
  spaceStore.fetchSpaces();
  void deviceStore.hydrate();
  void loadAutoUpdateSettings();
});

async function loadAutoUpdateSettings() {
  autoUpdatesLoading.value = true;
  autoUpdatesError.value = null;
  try {
    autoUpdatesSupported.value = await invoke<boolean>("startup_auto_update_supported");
    if (autoUpdatesSupported.value) {
      await loadAutoUpdatePreference();
    }
  } catch (error) {
    autoUpdatesError.value = String(error);
  } finally {
    autoUpdatesLoading.value = false;
  }
}

async function loadAutoUpdatePreference() {
  autoUpdatesEnabled.value = await invoke<boolean>("get_auto_update_enabled");
}

async function setAutoUpdatesEnabled(event: Event) {
  const target = event.target as HTMLInputElement;
  const previous = autoUpdatesEnabled.value;
  const enabled = target.checked;

  autoUpdatesEnabled.value = enabled;
  autoUpdatesSaving.value = true;
  autoUpdatesError.value = null;

  try {
    autoUpdatesEnabled.value = await invoke<boolean>("set_auto_update_enabled", { enabled });
  } catch (error) {
    autoUpdatesEnabled.value = previous;
    autoUpdatesError.value = String(error);
  } finally {
    autoUpdatesSaving.value = false;
  }
}

async function addSpace() {
  const name = newSpaceName.value.trim();
  if (!name) return;
  await spaceStore.createSpace(name);
  newSpaceName.value = "";
}

function startEdit(id: string, name: string) {
  editingId.value = id;
  editingName.value = name;
}

async function confirmEdit(id: string) {
  const name = editingName.value.trim();
  if (name) await spaceStore.updateSpace(id, { name });
  editingId.value = null;
}

function cancelEdit() {
  editingId.value = null;
}

function devicePresenceLabel(device: PairedDevice) {
  return deviceStore.isDeviceOnline(device) ? "Online" : "Offline";
}

function openSearchResult(result: SettingsSearchResult) {
  if (result.action === "export-backup") {
    showBackupExport.value = true;
    return;
  }
  if (result.action === "import-backup") {
    void backupImport.startImport();
    return;
  }
  settingsSearchQuery.value = "";
}
</script>

<template>
  <div class="flex flex-col gap-3 pb-24">

    <label class="input input-bordered flex items-center gap-2 bg-base-100">
      <span class="text-sm opacity-60">Search</span>
      <input
        v-model="settingsSearchQuery"
        type="search"
        class="grow"
        placeholder="Search settings"
        aria-label="Search settings"
        data-testid="settings-search-input"
      />
    </label>

    <div v-if="renderFlags.settingsSearchResults" class="flex flex-col gap-4" data-testid="settings-search-results">
      <section
        v-for="group in renderLists.searchResultGroups"
        :key="group.id"
        class="rounded-xl bg-base-200 p-3"
        data-testid="settings-search-group"
      >
        <h2 class="mb-3 text-sm font-semibold uppercase tracking-wide opacity-70">{{ group.title }}</h2>
        <SettingsListGroup>
          <SettingsListItem
            v-for="result in group.results"
            :key="result.id"
            button
            @click="openSearchResult(result)"
          >
            <template #start>
              <div>
                <span class="block font-medium">{{ result.title }}</span>
                <span v-if="result.description" class="block text-xs opacity-60">{{ result.description }}</span>
              </div>
            </template>
            <template #trailing>
              <span class="text-sm opacity-50">›</span>
            </template>
          </SettingsListItem>
        </SettingsListGroup>
      </section>
    </div>

    <section v-if="renderFlags.settingsOverview" class="rounded-xl bg-base-200 p-3" data-testid="settings-spaces">
      <h2 class="mb-3 text-sm font-semibold uppercase tracking-wide opacity-70">Spaces</h2>
      <div class="flex flex-col gap-3">
        <div v-if="renderFlags.spacesError" class="text-error text-sm">{{ spaceStore.error }}</div>
        <SettingsListGroup v-if="renderLists.spaces.length > 0">
          <template v-for="space in renderLists.spaces" :key="space.id">
            <SettingsListItem v-if="renderFlags.spaceEditor(space.id)">
              <template #start>
                <input
                  v-model="editingName"
                  class="input input-bordered input-sm w-full"
                  @keyup.enter="confirmEdit(space.id)"
                  @keyup.escape="cancelEdit"
                  autofocus
                />
              </template>
              <template #end>
                <div class="flex items-center justify-end gap-2">
                  <button class="btn btn-sm btn-primary" @click="confirmEdit(space.id)">Save</button>
                  <button class="btn btn-sm btn-ghost" @click="cancelEdit">Cancel</button>
                </div>
              </template>
            </SettingsListItem>
            <SettingsListItem v-else>
              <template #start>
                <span class="block truncate font-medium">{{ space.name }}</span>
              </template>
              <template #end>
                <ActionsBtn
                  :aria-label="`Actions for ${space.name}`"
                  @click="openSpaceMenu($event, space.id, space.name)"
                />
              </template>
            </SettingsListItem>
          </template>
        </SettingsListGroup>
        <form @submit.prevent="addSpace">
          <SettingsListGroup>
            <SettingsListItem>
              <template #start>
                <input v-model="newSpaceName" class="input input-bordered input-sm w-full" placeholder="New space name" />
              </template>
              <template #end>
                <button type="submit" class="btn btn-sm btn-primary">Add</button>
              </template>
            </SettingsListItem>
          </SettingsListGroup>
        </form>
      </div>
    </section>

    <section v-if="renderFlags.settingsOverview" class="rounded-xl bg-base-200 p-3" data-testid="settings-devices">
      <h2 class="mb-3 text-sm font-semibold uppercase tracking-wide opacity-70">Devices</h2>
      <SettingsListGroup>
        <SettingsListItem
          v-for="device in renderLists.pairedDevices"
          :key="device.peer_device_id"
          :to="`/settings/device/${device.peer_device_id}`"
          data-testid="paired-device-row"
          :data-peer-device-id="device.peer_device_id"
        >
          <template #leading>
            <span
              class="h-2.5 w-2.5 rounded-full"
              :class="deviceStore.isDeviceOnline(device) ? 'bg-green-500' : 'bg-gray-400'"
            />
          </template>
          <template #start>
            <span class="block truncate font-medium" data-testid="paired-device-name">{{ device.display_name }}</span>
          </template>
          <template #end>
            <span class="text-xs opacity-60">{{ devicePresenceLabel(device) }}</span>
          </template>
          <template #trailing>
            <span class="text-sm opacity-50">›</span>
          </template>
        </SettingsListItem>
        <SettingsListItem
          v-if="renderFlags.emptyPairedDevices"
        >
          <span class="opacity-70">No paired devices yet.</span>
        </SettingsListItem>
        <SettingsListItem to="/settings/add-device" data-testid="add-device-link">
          <template #leading>
            <span class="text-base leading-none">+</span>
          </template>
          <template #start>
            <span class="font-medium">Add device</span>
          </template>
          <template #trailing>
            <span class="text-sm opacity-50">›</span>
          </template>
        </SettingsListItem>
      </SettingsListGroup>
    </section>

    <ThemeSelector v-if="renderFlags.settingsOverview" />

    <section v-if="renderFlags.settingsOverview && renderFlags.automaticUpdatesSection" class="rounded-xl bg-base-200 p-3" data-testid="settings-updates">
      <h2 class="mb-3 text-sm font-semibold uppercase tracking-wide opacity-70">Updates</h2>
      <SettingsListGroup>
        <SettingsListItem>
          <template #start>
            <div>
              <span class="block font-medium">Automatic updates</span>
              <span class="block text-xs opacity-60">
                When this is off, Fini will not install updates automatically on the next restart.
              </span>
            </div>
          </template>
          <template #end>
            <input
              type="checkbox"
              class="toggle toggle-primary"
              data-testid="automatic-updates-toggle"
              aria-label="Automatic updates"
              :checked="autoUpdatesEnabled"
              :disabled="autoUpdatesLoading || autoUpdatesSaving"
              @change="setAutoUpdatesEnabled"
            />
          </template>
        </SettingsListItem>
      </SettingsListGroup>
      <div v-if="renderFlags.automaticUpdatesError" class="mt-2 text-xs text-error">{{ autoUpdatesError }}</div>
    </section>

    <section v-if="renderFlags.settingsOverview" class="rounded-xl bg-base-200 p-3" data-testid="settings-backup">
      <h2 class="mb-3 text-sm font-semibold uppercase tracking-wide opacity-70">Backup</h2>
      <p class="mb-2 text-xs opacity-60">Save your spaces and quests to a portable file, or restore from one.</p>
      <SettingsListGroup>
        <SettingsListItem button data-testid="backup-export-row" @click="showBackupExport = true">
          <template #leading>
            <svg class="h-5 w-5 flex-shrink-0 opacity-60" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M12 3v12"/><path d="M7 10l5 5 5-5"/><path d="M5 21h14"/>
            </svg>
          </template>
          <div>
            <span class="block font-medium">Export backup</span>
            <span class="block text-xs opacity-60">Saves a <code class="font-mono">.zip</code> with quests and quest series for the spaces you pick.</span>
          </div>
          <template #trailing>
            <span class="text-sm opacity-50">›</span>
          </template>
        </SettingsListItem>
        <SettingsListItem button data-testid="backup-import-row" @click="void backupImport.startImport()">
          <template #leading>
            <svg class="h-5 w-5 flex-shrink-0 opacity-60" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M12 21V9"/><path d="M7 14l5-5 5 5"/><path d="M5 3h14"/>
            </svg>
          </template>
          <div>
            <span class="block font-medium">Import backup</span>
            <span class="block text-xs opacity-60">Restore from a <code class="font-mono">.zip</code>. Conflicts will ask before overwriting.</span>
          </div>
          <template #trailing>
            <span class="text-sm opacity-50">›</span>
          </template>
        </SettingsListItem>
      </SettingsListGroup>
      <div v-if="renderFlags.backupImportError" class="mt-2 text-xs text-error">{{ backupImport.error.value }}</div>
    </section>

    <AboutCard v-if="renderFlags.settingsOverview" :version="appVersion" :source-url="sourceUrl" />

    <section v-if="renderFlags.settingsSearchEmptyState" class="rounded-xl bg-base-200 p-6 text-center" data-testid="settings-search-empty">
      <h2 class="text-sm font-semibold">No settings found</h2>
      <p class="mt-1 text-xs opacity-60">Try a different search term.</p>
    </section>

    <ExportSpacesDialog v-if="renderFlags.backupExportDialog" @close="showBackupExport = false" />

    <ImportSpaceMappingDialog
      v-if="renderFlags.backupImportMappingDialog"
      :incoming="backupImport.activeMapping.value!"
      :local-spaces="backupImport.selectableSpaces.value"
      :index="backupImport.mappingIndex.value"
      :total="backupImport.totalMappings.value"
      @cancel="backupImport.cancelImport()"
      @resolve="(r) => void backupImport.confirmMapping(r)"
    />

    <MergeConflictDialog
      v-if="renderFlags.backupMergeConflictDialog"
      :conflicts="backupImport.conflicts.value"
      @cancel="backupImport.cancelImport()"
      @apply="(r) => void backupImport.applyImport(r)"
    />
  </div>
</template>
