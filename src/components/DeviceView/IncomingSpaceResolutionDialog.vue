<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useDeviceStore } from "../../stores/device";
import { useSpaceStore } from "../../stores/space";
import { shortUuid } from "../../utils/shortUuid";
import MapToExistingDialog from "../MapToExistingDialog.vue";

const EMBEDDED_SPACE_IDS = new Set(["1", "2", "3"]);

function isEmbeddedSpaceId(spaceId: string): boolean {
  return EMBEDDED_SPACE_IDS.has(spaceId);
}

type ResolutionMode = "create_new" | "use_existing";

const deviceStore = useDeviceStore();
const spaceStore = useSpaceStore();
const saving = ref(false);
const error = ref<string | null>(null);
const deferredKey = ref<string | null>(null);
const showMapPicker = ref(false);
const resolutionModeByRemoteId = ref<Record<string, ResolutionMode>>({});
const resolutionExistingByRemoteId = ref<Record<string, string>>({});

const pendingRequests = computed(() => deviceStore.listPendingSpaceSyncRequests());
const unresolvedEntries = computed(() => {
  return deviceStore.pairedDevices.flatMap((device) => {
    return deviceStore.getUnresolvedCustomSpaces(device.peer_device_id).map((space) => ({
      peerDeviceId: device.peer_device_id,
      peerName: device.display_name,
      space,
    }));
  });
});

const activeDialog = computed(() => {
  for (const request of pendingRequests.value) {
    const key = `approve:${request.peer_device_id}:${request.sent_at}`;
    if (deferredKey.value === key) continue;

    const peer = deviceStore.findPairedDevice(request.peer_device_id);
    const peerName = peer?.display_name ?? shortUuid(request.peer_device_id);
    const requestedSpaceId = request.mapped_space_ids[0];
    if (!requestedSpaceId) continue;
    const customSpace = request.custom_spaces.find((space) => space.space_id === requestedSpaceId);
    const requestedSpaceName = customSpace?.name ?? spaceStore.spaces.find((space) => space.id === requestedSpaceId)?.name ?? shortUuid(requestedSpaceId);
    const customMessage = customSpace ? " You will choose how to map this custom space after approval." : "";

    return {
      kind: "approve" as const,
      key,
      peerDeviceId: request.peer_device_id,
      spaceId: requestedSpaceId,
      title: "Incoming space sync request",
      message: `${peerName} wants to sync ${requestedSpaceName} with this device.${customMessage}`,
      incomingSpace: null,
    };
  }

  for (const entry of unresolvedEntries.value) {
    const key = `resolve:${entry.peerDeviceId}:${entry.space.space_id}`;
    if (deferredKey.value === key) continue;

    return {
      kind: "resolve" as const,
      key,
      peerDeviceId: entry.peerDeviceId,
      spaceId: entry.space.space_id,
      title: "Resolve incoming space",
      message: `${entry.peerName} shared a custom space that needs a local mapping before sync can continue.`,
      incomingSpace: entry.space,
    };
  }

  return null;
});

const activeIncomingSpace = computed(() => {
  return activeDialog.value?.kind === "resolve" ? activeDialog.value.incomingSpace : null;
});

const activeApproveSpaceCount = computed(() => {
  const dialog = activeDialog.value;
  if (!dialog || dialog.kind !== "approve") return 0;
  return 1;
});

const activeResolutionMode = computed<ResolutionMode>(() => {
  const incomingSpace = activeIncomingSpace.value;
  if (!incomingSpace) return "create_new";
  return resolutionModeByRemoteId.value[incomingSpace.space_id] ?? "create_new";
});

const activeExistingSpaceId = computed(() => {
  const incomingSpace = activeIncomingSpace.value;
  if (!incomingSpace) return "";
  return resolutionExistingByRemoteId.value[incomingSpace.space_id] ?? "";
});

const selectableSpaces = computed(() => {
  const incomingSpace = activeIncomingSpace.value;
  if (!incomingSpace) return [];

  return spaceStore.spaces.filter((space) => {
    return !isEmbeddedSpaceId(space.id) || space.id === incomingSpace.space_id;
  });
});

const canConfirm = computed(() => {
  if (!activeDialog.value) return false;
  if (activeDialog.value.kind === "approve") return true;
  if (activeResolutionMode.value !== "use_existing") return true;
  return Boolean(activeExistingSpaceId.value);
});

watch(
  unresolvedEntries,
  (next) => {
    for (const entry of next) {
      if (!resolutionModeByRemoteId.value[entry.space.space_id]) {
        resolutionModeByRemoteId.value[entry.space.space_id] = "create_new";
      }

      if (!resolutionExistingByRemoteId.value[entry.space.space_id]) {
        const firstSelectable = spaceStore.spaces.find((space) => {
          return !isEmbeddedSpaceId(space.id) || space.id === entry.space.space_id;
        });
        if (firstSelectable) {
          resolutionExistingByRemoteId.value[entry.space.space_id] = firstSelectable.id;
        }
      }
    }
  },
  { immediate: true },
);

watch(activeDialog, (next) => {
  if (next) {
    error.value = null;
  }
});

watch(
  [pendingRequests, unresolvedEntries],
  ([nextPending, nextUnresolved]) => {
    if (!deferredKey.value) return;

    const activeKeys = new Set<string>();
    for (const request of nextPending) {
      activeKeys.add(`approve:${request.peer_device_id}:${request.sent_at}`);
    }
    for (const entry of nextUnresolved) {
      activeKeys.add(`resolve:${entry.peerDeviceId}:${entry.space.space_id}`);
    }

    if (!activeKeys.has(deferredKey.value)) {
      deferredKey.value = null;
    }
  },
  { immediate: true },
);

onMounted(() => {
  void spaceStore.fetchSpaces();
});

function dismissDialog() {
  if (!activeDialog.value) return;
  deferredKey.value = activeDialog.value.key;
}

function setResolutionMode(mode: ResolutionMode) {
  const incomingSpace = activeIncomingSpace.value;
  if (!incomingSpace) return;

  resolutionModeByRemoteId.value[incomingSpace.space_id] = mode;
  if (mode !== "use_existing") return;
  if (resolutionExistingByRemoteId.value[incomingSpace.space_id]) return;

  const firstSelectable = spaceStore.spaces.find((space) => {
    return !isEmbeddedSpaceId(space.id) || space.id === incomingSpace.space_id;
  });
  if (firstSelectable) {
    resolutionExistingByRemoteId.value[incomingSpace.space_id] = firstSelectable.id;
  }
}

async function createAndConfirm() {
  setResolutionMode("create_new");
  await confirmDialog();
}

async function mapToExistingAndConfirm(spaceId: string) {
  showMapPicker.value = false;
  const incomingSpace = activeIncomingSpace.value;
  if (!incomingSpace) return;
  resolutionModeByRemoteId.value[incomingSpace.space_id] = "use_existing";
  resolutionExistingByRemoteId.value[incomingSpace.space_id] = spaceId;
  await confirmDialog();
}

async function confirmDialog() {
  const dialog = activeDialog.value;
  if (!dialog) return;

  saving.value = true;
  error.value = null;

  try {
    if (dialog.kind === "approve") {
      await deviceStore.approvePendingSpaceSyncRequest(dialog.peerDeviceId, dialog.spaceId);
      await spaceStore.fetchSpaces();
      deferredKey.value = null;
      return;
    }

    const incomingSpace = dialog.incomingSpace;
    const existingSpaceId = activeExistingSpaceId.value;
    if (activeResolutionMode.value === "use_existing" && !existingSpaceId) {
      error.value = "Select existing local space to link.";
      return;
    }

    await deviceStore.resolveCustomSpaceMapping(
      dialog.peerDeviceId,
      incomingSpace.space_id,
      activeResolutionMode.value,
      activeResolutionMode.value === "use_existing" ? existingSpaceId : undefined,
      incomingSpace.name,
    );
    await spaceStore.fetchSpaces();
    deferredKey.value = null;
  } catch (err) {
    error.value = String(err);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <Teleport to="body">
    <div
      v-if="activeDialog"
      class="fixed inset-0 z-[1200] flex items-end justify-center p-3 sm:items-center sm:p-4"
      data-testid="incoming-space-sync-dialog"
      :data-dialog-kind="activeDialog.kind"
      :data-peer-device-id="activeDialog.peerDeviceId"
      :data-space-id="activeDialog.spaceId"
      :data-space-count="activeApproveSpaceCount"
    >
      <button
        type="button"
        class="absolute inset-0 bg-black/45"
        aria-label="Close dialog"
        @click="dismissDialog"
      ></button>

      <div class="relative w-full max-w-md rounded-xl bg-base-100 p-4 shadow-2xl">
        <h3 class="text-base font-semibold">{{ activeDialog.title }}</h3>
        <p class="mt-2 text-sm opacity-70">{{ activeDialog.message }}</p>

        <template v-if="activeIncomingSpace">
          <p class="mt-3 text-sm opacity-70">
            Incoming space: <span class="font-medium">{{ activeIncomingSpace.name }}</span>
          </p>
          <p class="font-mono text-xs opacity-60" :title="activeIncomingSpace.space_id">
            {{ shortUuid(activeIncomingSpace.space_id) }}
          </p>
        </template>

        <div v-if="error" class="mt-3 text-xs text-error">{{ error }}</div>

        <div class="mt-4 flex justify-end gap-2">
          <button class="btn btn-ghost btn-sm" @click="dismissDialog">Not now</button>
          <template v-if="activeDialog.kind === 'approve'">
            <button
              class="btn btn-primary btn-sm"
              data-testid="approve-space-sync"
              :disabled="!canConfirm || saving"
              @click="void confirmDialog()"
            >
              <template v-if="saving">Saving…</template>
              <template v-else>Approve sync</template>
            </button>
          </template>
          <template v-else-if="activeIncomingSpace">
            <button
              class="btn btn-sm"
              @click="showMapPicker = true"
            >Map to existing</button>
            <button
              class="btn btn-primary btn-sm"
              data-testid="approve-space-sync"
              :disabled="saving"
              @click="void createAndConfirm()"
            >
              <template v-if="saving">Saving…</template>
              <template v-else>Create</template>
            </button>
          </template>
        </div>
      </div>
    </div>

    <MapToExistingDialog
      v-if="showMapPicker && activeIncomingSpace"
      :context="{ kind: 'peer-space', name: activeIncomingSpace.name }"
      :spaces="selectableSpaces"
      @cancel="showMapPicker = false"
      @confirm="(id) => void mapToExistingAndConfirm(id)"
    />
  </Teleport>
</template>
