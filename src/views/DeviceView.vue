<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import IncomingSpaceResolutionDialog from "../components/DeviceView/IncomingSpaceResolutionDialog.vue";
import { useDeviceStore } from "../stores/device";
import { useSpaceStore } from "../stores/space";
import { shortUuid } from "../utils/shortUuid";

const EMBEDDED_SPACE_IDS = new Set(["1", "2", "3"]);

function isEmbeddedSpaceId(spaceId: string): boolean {
  return EMBEDDED_SPACE_IDS.has(spaceId);
}

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
const resolvingRemoteSpaceId = ref<string | null>(null);
const resolveDialogOpen = ref(false);
const resolveDialogRemoteSpaceId = ref<string | null>(null);
const resolveDialogDeferred = ref(false);
const previousUnresolvedIds = ref<string[]>([]);
const resolutionModeByRemoteId = ref<Record<string, "create_new" | "use_existing">>({});
const resolutionExistingByRemoteId = ref<Record<string, string>>({});

const deviceId = computed(() => String(route.params.id ?? ""));
const device = computed(() => deviceStore.findPairedDevice(deviceId.value));
const online = computed(() => (device.value ? deviceStore.isDeviceOnline(device.value) : false));
const syncStatus = computed(() => {
  if (!deviceId.value) return null;
  return deviceStore.getSpaceSyncStatus(deviceId.value);
});
const hasPendingSync = computed(() => (syncStatus.value?.pending_event_count ?? 0) > 0);
const lastSyncedAtBySpace = computed<Record<string, string | null>>(() => {
  if (!deviceId.value) return {};
  return deviceStore.getLastSyncedAtBySpace(deviceId.value);
});
const lastSyncedLabelBySpace = computed<Record<string, string>>(() => {
  const labels: Record<string, string> = {};
  for (const [spaceId, syncedAt] of Object.entries(lastSyncedAtBySpace.value)) {
    if (!syncedAt) continue;
    labels[spaceId] = new Date(syncedAt).toLocaleTimeString();
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
const activeIncomingCustomSpace = computed(() => {
  if (unresolvedCustomSpaces.value.length === 0) return null;
  const currentId = resolveDialogRemoteSpaceId.value;
  if (!currentId) return unresolvedCustomSpaces.value[0];
  return (
    unresolvedCustomSpaces.value.find((item) => item.space_id === currentId) ??
    unresolvedCustomSpaces.value[0]
  );
});
const activeIncomingMode = computed<"create_new" | "use_existing">(() => {
  const active = activeIncomingCustomSpace.value;
  if (!active) return "create_new";
  return resolutionModeByRemoteId.value[active.space_id] ?? "create_new";
});
const activeIncomingExistingSpaceId = computed(() => {
  const active = activeIncomingCustomSpace.value;
  if (!active) return "";
  return resolutionExistingByRemoteId.value[active.space_id] ?? "";
});
const selectableLocalSpacesForActiveIncoming = computed(() => {
  const active = activeIncomingCustomSpace.value;
  if (!active) return [];
  return spaceStore.spaces.filter(
    (item) => !isEmbeddedSpaceId(item.id) || item.id === active.space_id,
  );
});
const canResolveActiveIncoming = computed(() => {
  const active = activeIncomingCustomSpace.value;
  if (!active) return false;
  if (activeIncomingMode.value !== "use_existing") return true;
  return Boolean(activeIncomingExistingSpaceId.value);
});
const resolveDialogVisible = computed(() => {
  if (resolveDialogDeferred.value) return false;
  if (!activeIncomingCustomSpace.value) return false;
  if (resolveDialogOpen.value) return true;
  return unresolvedCustomSpaces.value.length > 0;
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
  previousUnresolvedIds.value = [];
  resolveDialogRemoteSpaceId.value = null;
  resolveDialogDeferred.value = false;
  resolveDialogOpen.value = false;
  void loadMappings();
});

watch(savedMappedSelection, (next) => {
  if (savingMappings.value || mappingsDirty.value) return;
  mappedSelection.value = [...next];
});

watch(unresolvedCustomSpaces, (next) => {
  const nextIds = next.map((item) => item.space_id);
  const hasNewIncoming = nextIds.some((id) => !previousUnresolvedIds.value.includes(id));

  for (const item of next) {
    if (!resolutionModeByRemoteId.value[item.space_id]) {
      resolutionModeByRemoteId.value[item.space_id] = "create_new";
    }

    if (!resolutionExistingByRemoteId.value[item.space_id]) {
      const firstSelectable = spaceStore.spaces.find(
        (space) => !isEmbeddedSpaceId(space.id) || space.id === item.space_id,
      );
      if (firstSelectable) {
        resolutionExistingByRemoteId.value[item.space_id] = firstSelectable.id;
      }
    }
  }

  if (next.length === 0) {
    previousUnresolvedIds.value = [];
    resolveDialogRemoteSpaceId.value = null;
    resolveDialogDeferred.value = false;
    resolveDialogOpen.value = false;
    return;
  }

  if (hasNewIncoming) {
    resolveDialogDeferred.value = false;
  }

  const currentId = resolveDialogRemoteSpaceId.value;
  if (!currentId || !nextIds.includes(currentId)) {
    resolveDialogRemoteSpaceId.value = next[0].space_id;
  }

  if (!resolveDialogDeferred.value && !resolveDialogOpen.value) {
    resolveDialogOpen.value = true;
  }

  previousUnresolvedIds.value = nextIds;
}, { immediate: true });

function setResolutionMode(spaceId: string, mode: "create_new" | "use_existing") {
  resolutionModeByRemoteId.value[spaceId] = mode;
  if (mode !== "use_existing") return;
  if (resolutionExistingByRemoteId.value[spaceId]) return;

  const firstSelectable = spaceStore.spaces.find(
    (space) => !isEmbeddedSpaceId(space.id) || space.id === spaceId,
  );
  if (firstSelectable) {
    resolutionExistingByRemoteId.value[spaceId] = firstSelectable.id;
  }
}

function openResolveSpaceDialog() {
  const active = activeIncomingCustomSpace.value;
  if (!active) return;
  resolveDialogDeferred.value = false;
  resolveDialogRemoteSpaceId.value = active.space_id;
  resolveDialogOpen.value = true;
}

function cancelResolveSpaceDialog() {
  resolveDialogDeferred.value = true;
  resolveDialogOpen.value = false;
}

function setActiveIncomingMode(mode: "create_new" | "use_existing") {
  const active = activeIncomingCustomSpace.value;
  if (!active) return;
  setResolutionMode(active.space_id, mode);
}

function setActiveIncomingExistingSpaceId(spaceId: string) {
  const active = activeIncomingCustomSpace.value;
  if (!active) return;
  resolutionExistingByRemoteId.value[active.space_id] = spaceId;
}

function resolveActiveIncomingCustomSpace() {
  const active = activeIncomingCustomSpace.value;
  if (!active) return;
  void resolveCustomSpace(active.space_id, active.name);
}

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

async function resolveCustomSpace(spaceId: string, name: string) {
  if (!deviceId.value) return;

  const mode = resolutionModeByRemoteId.value[spaceId] ?? "create_new";
  const existingSpaceId = resolutionExistingByRemoteId.value[spaceId];
  if (mode === "use_existing" && !existingSpaceId) {
    mappingError.value = "Select existing local space to link.";
    return;
  }

  resolvingRemoteSpaceId.value = spaceId;
  resolveDialogDeferred.value = false;
  mappingError.value = null;
  try {
    await deviceStore.resolveCustomSpaceMapping(
      deviceId.value,
      spaceId,
      mode,
      mode === "use_existing" ? existingSpaceId : undefined,
      name,
    );
    await spaceStore.fetchSpaces();
    await loadMappings();

    if (unresolvedCustomSpaces.value.length === 0) {
      resolveDialogRemoteSpaceId.value = null;
      resolveDialogOpen.value = false;
      return;
    }

    resolveDialogRemoteSpaceId.value = unresolvedCustomSpaces.value[0].space_id;
    resolveDialogOpen.value = true;
  } catch (error) {
    mappingError.value = String(error);
  } finally {
    resolvingRemoteSpaceId.value = null;
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
</script>

<template>
  <div class="flex flex-col gap-4 pb-24">
    <header class="flex items-center justify-between rounded-xl bg-base-200 px-3 py-2">
      <router-link to="/settings" class="text-sm font-medium opacity-70">‹ Settings</router-link>
      <span class="text-sm font-semibold">Device</span>
      <span class="text-xs opacity-60" v-if="device">{{ deviceStore.shortDeviceId(device.peer_device_id) }}</span>
      <span class="text-xs opacity-60" v-else>Unknown</span>
    </header>

    <section v-if="device" class="rounded-xl bg-base-200 p-3">
      <div class="flex items-center gap-3">
        <span class="h-2.5 w-2.5 rounded-full" :class="online ? 'bg-green-500' : 'bg-gray-400'" />
        <h1 class="text-base font-semibold">{{ device.display_name }}</h1>
      </div>
      <p class="mt-2 text-xs opacity-60">paired at {{ new Date(device.paired_at).toLocaleString() }}</p>
      <p class="text-xs opacity-60">
        {{ online ? "online" : "offline" }}
        <template v-if="device.last_seen_at">
          · last seen {{ new Date(device.last_seen_at).toLocaleString() }}
        </template>
      </p>
    </section>

    <section v-if="device" class="rounded-xl bg-base-200 p-3">
      <h2 class="mb-2 text-sm font-semibold uppercase tracking-wide opacity-70">Mapped spaces</h2>
      <div class="flex flex-col gap-2">
        <p class="text-xs opacity-60">
          Select spaces to sync with this device. Changes apply symmetrically for this pair.
        </p>
        <div v-if="mappingError" class="text-error text-xs">{{ mappingError }}</div>
        <ul class="flex flex-col gap-1">
          <li
            v-for="space in spaceStore.spaces"
            :key="space.id"
            class="flex items-center gap-3 rounded-lg bg-base-100 px-3 py-2"
          >
            <input
              type="checkbox"
              class="checkbox checkbox-sm"
              :checked="mappedSelection.includes(space.id)"
              :disabled="!mappingsLoaded || savingMappings"
              @change="toggleMappedSpace(space.id)"
            />
            <span class="flex-1 text-sm">{{ space.name }}</span>
            <span
              v-if="mappedSelection.includes(space.id) && hasPendingSync"
              class="loading loading-spinner loading-xs text-primary"
              title="Syncing"
            />
            <span
              v-else-if="mappedSelection.includes(space.id) && !hasPendingSync && lastSyncedLabelBySpace[space.id]"
              class="text-[11px] opacity-60"
            >
              last synced: {{ lastSyncedLabelBySpace[space.id] }}
            </span>
            <span v-if="!isEmbeddedSpaceId(space.id)" class="text-xs opacity-60" :title="space.id">{{ shortUuid(space.id) }}</span>
          </li>
          <li
            v-if="spaceStore.spaces.length === 0"
            class="rounded-lg bg-base-100 px-3 py-2 text-sm opacity-70"
          >
            No spaces available.
          </li>
        </ul>
        <div
          v-if="unresolvedCustomSpaces.length > 0"
          class="rounded-lg border border-warning/30 bg-base-100 p-3 text-xs"
        >
          <p class="mb-2 font-medium text-warning">Incoming custom spaces need resolution</p>
          <p class="mb-3 opacity-70">
            You have {{ unresolvedCustomSpaces.length }} incoming custom
            {{ unresolvedCustomSpaces.length > 1 ? "spaces" : "space" }}.
          </p>
          <div class="flex items-center gap-2">
            <button class="btn btn-xs btn-warning" @click="openResolveSpaceDialog">
              Resolve invite
            </button>
          </div>
        </div>
        <div class="flex items-center gap-2">
          <button
            class="btn btn-sm btn-primary"
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
        <div v-if="syncStatus" class="rounded-lg bg-base-100 px-3 py-2 text-xs opacity-70">
          pending {{ syncStatus.pending_event_count }} · outbox {{ syncStatus.outbox_event_count }}
          · acked {{ syncStatus.acked_event_count }}
        </div>
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

    <IncomingSpaceResolutionDialog
      :open="resolveDialogVisible"
      :incoming-space="activeIncomingCustomSpace"
      :resolution-mode="activeIncomingMode"
      :existing-space-id="activeIncomingExistingSpaceId"
      :selectable-spaces="selectableLocalSpacesForActiveIncoming"
      :resolving="Boolean(activeIncomingCustomSpace && resolvingRemoteSpaceId === activeIncomingCustomSpace.space_id)"
      :can-resolve="canResolveActiveIncoming"
      :error="mappingError"
      @cancel="cancelResolveSpaceDialog"
      @set-mode="setActiveIncomingMode"
      @set-existing-space-id="setActiveIncomingExistingSpaceId"
      @confirm="resolveActiveIncomingCustomSpace"
    />
  </div>
</template>
