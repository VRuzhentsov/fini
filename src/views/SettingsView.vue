<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import packageJson from "../../package.json";
import AboutCard from "../components/SettingsView/AboutCard.vue";
import AutomaticUpdatesSettingsSection from "../components/SettingsView/AutomaticUpdatesSettingsSection.vue";
import BackupSettingsSection from "../components/SettingsView/BackupSettingsSection.vue";
import DevicesSettingsSection from "../components/SettingsView/DevicesSettingsSection.vue";
import SettingsListGroup from "../components/SettingsView/SettingsListGroup.vue";
import SettingsListItem from "../components/SettingsView/SettingsListItem.vue";
import SpacesSettingsSection from "../components/SettingsView/SpacesSettingsSection.vue";
import ThemeSelector from "../components/SettingsView/ThemeSelector.vue";
import { useDeviceStore, type PairedDevice } from "../stores/device";
import { useSpaceStore } from "../stores/space";

const spaceStore = useSpaceStore();
const deviceStore = useDeviceStore();
const settingsSearchQuery = ref("");
const appVersion = packageJson.version;
const sourceUrl = "https://github.com/VRuzhentsov/fini";

type SettingsSearchAction = "overview";
interface SettingsSearchResult { id: string; title: string; description?: string; action?: SettingsSearchAction; }
interface SettingsSearchGroup { id: string; title: string; results: SettingsSearchResult[]; }

const normalizedSettingsSearchQuery = computed(() => settingsSearchQuery.value.trim().toLocaleLowerCase());
function matchesSettingsSearch(parts: Array<string | null | undefined>) {
  const query = normalizedSettingsSearchQuery.value;
  return !query || parts.some((part) => part?.toLocaleLowerCase().includes(query));
}
function devicePresenceLabel(device: PairedDevice) {
  return deviceStore.isDeviceOnline(device) ? "Online" : "Offline";
}
function visibleSearchResults(results: SettingsSearchResult[]) {
  return results.filter((result) => matchesSettingsSearch([result.title, result.description]));
}

const renderLists = computed(() => ({
  settingsSections: [
    { id: "spaces", component: SpacesSettingsSection },
    { id: "devices", component: DevicesSettingsSection },
    { id: "appearance", component: ThemeSelector },
    { id: "updates", component: AutomaticUpdatesSettingsSection },
    { id: "backup", component: BackupSettingsSection },
    { id: "about", component: AboutCard, props: { version: appVersion, sourceUrl } },
  ],
  searchResultGroups: [
    { id: "spaces", title: "Spaces", results: visibleSearchResults(spaceStore.spaces.map((space) => ({ id: `space-${space.id}`, title: space.name, description: "Manage named contexts", action: "overview" as const }))) },
    { id: "devices", title: "Devices", results: visibleSearchResults(deviceStore.pairedDevices.map((device) => ({ id: `device-${device.peer_device_id}`, title: device.display_name, description: devicePresenceLabel(device), action: "overview" as const }))) },
    { id: "appearance", title: "Appearance", results: visibleSearchResults([{ id: "theme", title: "Theme", description: "System, Light, or Dark", action: "overview" }]) },
    { id: "backup", title: "Backup", results: visibleSearchResults([{ id: "export-backup", title: "Export backup", description: "Save spaces and quests to a portable file", action: "overview" }, { id: "import-backup", title: "Import backup", description: "Restore from a portable backup file", action: "overview" }]) },
    { id: "about", title: "About", results: visibleSearchResults([{ id: "version", title: "Version", description: appVersion, action: "overview" }]) },
  ].filter((group): group is SettingsSearchGroup => group.results.length > 0),
}));

const renderFlags = computed(() => ({
  settingsOverview: !normalizedSettingsSearchQuery.value,
  settingsSearchResults: Boolean(normalizedSettingsSearchQuery.value) && renderLists.value.searchResultGroups.length > 0,
  settingsSearchEmptyState: Boolean(normalizedSettingsSearchQuery.value) && renderLists.value.searchResultGroups.length === 0,
}));

onMounted(() => {
  void spaceStore.fetchSpaces();
  void deviceStore.hydrate();
});

function openSearchResult() {
  settingsSearchQuery.value = "";
}
</script>

<template>
  <div class="flex flex-col gap-3 pb-24">
    <label class="input input-bordered flex w-full items-center bg-base-100"><input v-model="settingsSearchQuery" type="search" class="w-full" placeholder="Search settings" aria-label="Search settings" data-testid="settings-search-input" /></label>

    <div v-if="renderFlags.settingsSearchResults" class="flex flex-col gap-4" data-testid="settings-search-results">
      <section v-for="group in renderLists.searchResultGroups" :key="group.id" class="rounded-xl bg-base-200 p-3" data-testid="settings-search-group">
        <h2 class="mb-3 text-sm font-semibold uppercase tracking-wide opacity-70">{{ group.title }}</h2>
        <SettingsListGroup><SettingsListItem v-for="result in group.results" :key="result.id" button @click="openSearchResult"><template #start><div><span class="block font-medium">{{ result.title }}</span><span v-if="result.description" class="block text-xs opacity-60">{{ result.description }}</span></div></template><template #trailing><span class="text-sm opacity-50">›</span></template></SettingsListItem></SettingsListGroup>
      </section>
    </div>

    <component v-for="section in renderLists.settingsSections" v-else-if="renderFlags.settingsOverview" :is="section.component" :key="section.id" v-bind="section.props" />

    <section v-if="renderFlags.settingsSearchEmptyState" class="rounded-xl bg-base-200 p-6 text-center" data-testid="settings-search-empty"><h2 class="text-sm font-semibold">No settings found</h2><p class="mt-1 text-xs opacity-60">Try a different search term.</p></section>
  </div>
</template>
