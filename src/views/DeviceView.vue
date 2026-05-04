<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import SettingsListGroup from "../components/SettingsView/SettingsListGroup.vue";
import SettingsListItem from "../components/SettingsView/SettingsListItem.vue";
import { useDeviceStore } from "../stores/device";
import { useSpaceStore } from "../stores/space";

const route = useRoute();
const router = useRouter();
const deviceStore = useDeviceStore();
const spaceStore = useSpaceStore();
const unpairDialog = ref<HTMLDialogElement | null>(null);
const mappedSelection = ref<string[]>([]);
const mappingsLoaded = ref(false);
const savingMappings = ref(false);
const mappingError = ref<string | null>(null);
const mappingsDirty = ref(false);

const deviceId = computed(() => String(route.params.id ?? ""));
const device = computed(() => deviceStore.findPairedDevice(deviceId.value));
const online = computed(() => (device.value ? deviceStore.isDeviceOnline(device.value) : false));
const syncStatus = computed(() => {
  if (!deviceId.value) return null;
  return deviceStore.getSpaceSyncStatus(deviceId.value);
});
const hasPendingSync = computed(() => (syncStatus.value?.pending_event_count ?? 0) > 0);
const presenceLabel = computed(() => (online.value ? "Online" : "Offline"));
const lastSyncedAtBySpace = computed<Record<string, string | null>>(() => {
  if (!deviceId.value) return {};
  return deviceStore.getLastSyncedAtBySpace(deviceId.value);
});
const lastSyncedLabelBySpace = computed<Record<string, string>>(() => {
  const labels: Record<string, string> = {};
  for (const [spaceId, syncedAt] of Object.entries(lastSyncedAtBySpace.value)) {
    if (!syncedAt) continue;
    labels[spaceId] = new Date(syncedAt).toLocaleString();
  }
  return labels;
});

const savedMappedSelection = computed(() => {
  if (!deviceId.value) return [];
  return deviceStore.getMappedSpaceIds(deviceId.value);
});
const unresolvedCustomSpaces = computed(() => {
  if (!deviceId.value) return [];
  return deviceStore.getUnresolvedCustomSpaces(deviceId.value);
});

const hasMappingChanges = computed(() => {
  if (!deviceId.value) return false;
  const saved = [...deviceStore.getMappedSpaceIds(deviceId.value)].sort();
  const current = [...mappedSelection.value].sort();
  return saved.join(",") !== current.join(",");
});

onMounted(() => {
  void deviceStore.hydrate();
  void spaceStore.fetchSpaces();
  void deviceStore.runSpaceSyncTick();
  void loadMappings();
});

watch(deviceId, () => {
  mappingsDirty.value = false;
  void loadMappings();
});

watch(savedMappedSelection, (next) => {
  if (savingMappings.value || mappingsDirty.value) return;
  mappedSelection.value = [...next];
});

function toggleMappedSpace(spaceId: string) {
  mappingsDirty.value = true;
  if (mappedSelection.value.includes(spaceId)) {
    mappedSelection.value = mappedSelection.value.filter((id) => id !== spaceId);
    return;
  }
  mappedSelection.value = [...mappedSelection.value, spaceId];
}

async function loadMappings() {
  mappingError.value = null;
  mappingsLoaded.value = false;

  if (!deviceId.value) {
    mappedSelection.value = [];
    mappingsLoaded.value = true;
    return;
  }

  try {
    mappedSelection.value = await deviceStore.loadMappedSpaces(deviceId.value);
    await deviceStore.refreshSpaceSyncStatus(deviceId.value);
    mappingsDirty.value = false;
  } catch (error) {
    mappingError.value = String(error);
  } finally {
    mappingsLoaded.value = true;
  }
}

async function saveMappings() {
  if (!deviceId.value) return;
  savingMappings.value = true;
  mappingError.value = null;

  try {
    const unique = [...new Set(mappedSelection.value)];
    mappedSelection.value = await deviceStore.saveMappedSpaces(deviceId.value, unique);
    mappingsDirty.value = false;
  } catch (error) {
    mappingError.value = String(error);
  } finally {
    savingMappings.value = false;
  }
}

function openUnpairDialog() {
  unpairDialog.value?.showModal();
}

async function confirmUnpair() {
  if (!device.value) return;

  unpairDialog.value?.close();
  await deviceStore.unpairDevice(device.value.peer_device_id);
  await router.push("/settings");
}

function mappedSpaceEndLabel(spaceId: string): string | null {
  if (!mappedSelection.value.includes(spaceId)) return null;
  if (hasPendingSync.value) return "Syncing";
  const lastSynced = lastSyncedLabelBySpace.value[spaceId];
  return lastSynced ? `last synced: ${lastSynced}` : "Mapped";
}
</script>

<template>
  <div class="flex flex-col gap-4 pb-24">
    <header class="flex items-center justify-between rounded-xl bg-base-200 px-3 py-2">
      <router-link to="/settings" class="text-sm font-medium opacity-70">‹ Settings</router-link>
      <span class="text-sm font-semibold">Device</span>
      <span class="text-xs opacity-60">{{ device ? presenceLabel : "Unknown" }}</span>
    </header>

    <section v-if="device" class="rounded-xl bg-base-200 p-3">
      <SettingsListGroup>
        <SettingsListItem>
          <template #leading>
            <span class="h-2.5 w-2.5 rounded-full" :class="online ? 'bg-green-500' : 'bg-gray-400'" />
          </template>
          <template #start>
            <span class="block truncate font-semibold">{{ device.display_name }}</span>
          </template>
          <template #end>
            <span class="text-xs opacity-60">{{ presenceLabel }}</span>
          </template>
        </SettingsListItem>
        <SettingsListItem>
          <template #start>
            <span class="font-medium">Paired</span>
          </template>
          <template #end>
            <span class="text-xs opacity-60">{{ new Date(device.paired_at).toLocaleString() }}</span>
          </template>
        </SettingsListItem>
        <SettingsListItem v-if="device.last_seen_at">
          <template #start>
            <span class="font-medium">Last seen</span>
          </template>
          <template #end>
            <span class="text-xs opacity-60">{{ new Date(device.last_seen_at).toLocaleString() }}</span>
          </template>
        </SettingsListItem>
      </SettingsListGroup>
    </section>

    <section v-if="device" class="rounded-xl bg-base-200 p-3">
      <h2 class="mb-2 text-sm font-semibold uppercase tracking-wide opacity-70">Mapped spaces</h2>
      <div class="flex flex-col gap-2">
        <p class="text-xs opacity-60">
          Select spaces to sync with this device. Changes apply symmetrically for this pair.
        </p>
        <div v-if="mappingError" class="text-error text-xs">{{ mappingError }}</div>
        <SettingsListGroup>
          <SettingsListItem
            v-for="space in spaceStore.spaces"
            :key="space.id"
            data-testid="mapped-space-row"
            :data-space-id="space.id"
          >
            <template #leading>
              <input
                type="checkbox"
                class="checkbox checkbox-sm"
                data-testid="mapped-space-checkbox"
                :checked="mappedSelection.includes(space.id)"
                :disabled="!mappingsLoaded || savingMappings"
                @change="toggleMappedSpace(space.id)"
              />
            </template>
            <template #start>
              <span class="block truncate">{{ space.name }}</span>
            </template>
            <template #end>
              <span
                v-if="mappedSpaceEndLabel(space.id)"
                class="text-[11px] opacity-60"
                data-testid="mapped-space-last-synced"
              >
                {{ mappedSpaceEndLabel(space.id) }}
              </span>
            </template>
          </SettingsListItem>
          <SettingsListItem
            v-if="spaceStore.spaces.length === 0"
          >
            <span class="opacity-70">No spaces available.</span>
          </SettingsListItem>
        </SettingsListGroup>
        <div
          v-if="unresolvedCustomSpaces.length > 0"
          class="rounded-lg border border-warning/30 bg-base-100 p-3 text-xs"
        >
          <p class="mb-2 font-medium text-warning">Incoming custom spaces need resolution</p>
          <p class="mb-3 opacity-70">
            You have {{ unresolvedCustomSpaces.length }} incoming custom
            {{ unresolvedCustomSpaces.length > 1 ? "spaces" : "space" }} waiting in the global sync dialog.
          </p>
        </div>
        <div class="flex items-center gap-2">
          <button
            class="btn btn-sm btn-primary"
            data-testid="save-space-mappings"
            :disabled="!mappingsLoaded || savingMappings || !hasMappingChanges"
            @click="void saveMappings()"
          >
            {{ savingMappings ? "Saving..." : "Save mappings" }}
          </button>
          <button
            class="btn btn-sm btn-ghost"
            :disabled="savingMappings"
            @click="void loadMappings()"
          >
            Reload
          </button>
        </div>
        <SettingsListGroup v-if="syncStatus">
          <SettingsListItem>
            <template #start>
              <span class="font-medium">Sync status</span>
            </template>
            <template #end>
              <span class="text-xs opacity-70">
                pending {{ syncStatus.pending_event_count }} · outbox {{ syncStatus.outbox_event_count }}
                · acked {{ syncStatus.acked_event_count }}
              </span>
            </template>
          </SettingsListItem>
        </SettingsListGroup>
      </div>
    </section>

    <section v-if="device" class="rounded-xl bg-base-200 p-3">
      <h2 class="mb-2 text-sm font-semibold uppercase tracking-wide opacity-70">Actions</h2>
      <button class="btn btn-error btn-sm" @click="openUnpairDialog">Unpair</button>
    </section>

    <section v-else class="rounded-xl bg-base-200 p-3">
      <p class="text-sm opacity-70">Device not found.</p>
      <router-link to="/settings" class="btn btn-sm mt-2">Back to settings</router-link>
    </section>

    <dialog ref="unpairDialog" class="modal">
      <div class="modal-box">
        <h3 class="text-base font-semibold">Unpair device?</h3>
        <p class="mt-2 text-sm opacity-70">
          Existing local synced data will stay, but future sync with this device will stop.
        </p>
        <div class="modal-action">
          <form method="dialog">
            <button class="btn btn-ghost btn-sm">Cancel</button>
          </form>
          <button class="btn btn-error btn-sm" @click="void confirmUnpair()">Unpair</button>
        </div>
      </div>
      <form method="dialog" class="modal-backdrop">
        <button>close</button>
      </form>
    </dialog>
  </div>
</template>
