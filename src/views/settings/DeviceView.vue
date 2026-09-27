<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { ChevronLeftIcon } from "@heroicons/vue/24/outline";
import SettingsListGroup from "../../components/settings/SettingsListGroup.vue";
import SettingsListItem from "../../components/settings/SettingsListItem.vue";
import ChannelRow from "../../components/settings/device/ChannelRow.vue";
import SyncQueueSection from "../../components/settings/device/SyncQueueSection.vue";
import DeviceSetupDialog from "../../components/settings/DeviceSetupDialog.vue";
import { useDeviceStore } from "../../stores/device";
import { ChannelColor, type ChannelKind } from "../../utils/channel";
import { useSpaceStore, isBuiltinSpace } from "../../stores/space";
import { shortUuid } from "../../utils/shortUuid";

const route = useRoute();
const router = useRouter();
const deviceStore = useDeviceStore();
const spaceStore = useSpaceStore();

const unpairDialog = ref<HTMLDialogElement | null>(null);
// The channel being added with the setup dialog, or null when it is closed.
const channelBeingAdded = ref<ChannelKind | null>(null);
const mappedSelection = ref<string[]>([]);
const mappingsLoaded = ref(false);
const savingMappings = ref(false);
const mappingsDirty = ref(false);
const mappingError = ref<string | null>(null);
const channelError = ref<string | null>(null);
const busyChannel = ref<ChannelKind | null>(null);

const deviceId = computed(() => String(route.params.id ?? ""));
const device = computed(() => deviceStore.findPairedDevice(deviceId.value));
const peerName = computed(() => device.value?.display_name ?? "That device");

const channelStatuses = computed(() =>
  deviceId.value ? deviceStore.getChannelStatuses(deviceId.value) : [],
);
const syncQueue = computed(() => (deviceId.value ? deviceStore.getSyncQueue(deviceId.value) : null));

// The template's render contract, per fini-frontend.
const renderFlags = computed(() => ({
  channelSetupDialog: device.value !== null && device.value !== undefined && channelBeingAdded.value !== null,
}));

// Drives the sync queue's "sending now" vs "nothing can reach it" line:
// a green channel is one data can go over (ADR-0008 D9).
const anyChannelConnected = computed(() =>
  channelStatuses.value.some((status) => status.color === ChannelColor.Green),
);

const lastSyncedAtBySpace = computed<Record<string, string | null>>(() =>
  deviceId.value ? deviceStore.getLastSyncedAtBySpace(deviceId.value) : {},
);

const savedMappedSelection = computed(() =>
  deviceId.value ? deviceStore.getMappedSpaceIds(deviceId.value) : [],
);
const unresolvedCustomSpaces = computed(() =>
  deviceId.value ? deviceStore.getUnresolvedCustomSpaces(deviceId.value) : [],
);
const hasMappingChanges = computed(() => {
  if (!deviceId.value) return false;
  const saved = [...deviceStore.getMappedSpaceIds(deviceId.value)].sort();
  const current = [...mappedSelection.value].sort();
  return saved.join(",") !== current.join(",");
});

// Which spaces the unlink confirmation names. The design's rule is that a
// destructive confirmation states what actually stops, not a generic
// warning -- so it lists the spaces by name.
// Read from what is saved, not from the checkbox draft: unlinking removes
// the persisted mappings, so those are what stop. Someone who ticks a box
// and then unlinks without saving would otherwise be told a space stops
// syncing that was never mapped, or not told about one that is.
const mappedSpaceNames = computed(() =>
  savedMappedSelection.value.map(
    (id) =>
      // An id whose name has not arrived yet is still a space that stops
      // syncing. Dropping it made the count fall to zero while mappings
      // existed, and the confirmation then told the person that nothing is
      // shared with this device -- "I do not know its name" rendered as
      // "there is nothing here", on the one screen where being wrong costs
      // them a decision.
      spaceStore.spaces.find((space) => space.id === id)?.name ?? "a space",
  ),
);

// While this page is open the backend searches for the peer so green is
// current (ADR-0008 D12), and the rows are re-read from it; neither runs in
// the background.
const LIVE_POLL_INTERVAL_MS = 5_000;
let livePollTimer: ReturnType<typeof setInterval> | null = null;

onMounted(() => {
  void deviceStore.hydrate();
  void spaceStore.fetchSpaces();
  void deviceStore.runSpaceSyncTick();
  void loadDeviceState();
  void deviceStore.watchPresence(true);

  livePollTimer = setInterval(() => {
    if (!deviceId.value) return;
    void deviceStore.refreshChannelStatuses(deviceId.value);
    void deviceStore.refreshSyncQueue(deviceId.value);
  }, LIVE_POLL_INTERVAL_MS);
});

onUnmounted(() => {
  if (livePollTimer) {
    clearInterval(livePollTimer);
    livePollTimer = null;
  }
  void deviceStore.watchPresence(false);
});

watch(deviceId, () => {
  mappingsDirty.value = false;
  channelError.value = null;
  channelBeingAdded.value = null;
  void loadDeviceState();
});

watch(savedMappedSelection, (next) => {
  if (savingMappings.value || mappingsDirty.value) return;
  mappedSelection.value = [...next];
});

async function loadDeviceState() {
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
    await deviceStore.refreshChannelStatuses(deviceId.value);
    await deviceStore.refreshSyncQueue(deviceId.value);
    mappingsDirty.value = false;
  } catch (error) {
    mappingError.value = String(error);
  } finally {
    mappingsLoaded.value = true;
  }
}

function toggleMappedSpace(spaceId: string) {
  mappingsDirty.value = true;
  mappedSelection.value = mappedSelection.value.includes(spaceId)
    ? mappedSelection.value.filter((id) => id !== spaceId)
    : [...mappedSelection.value, spaceId];
}

async function saveMappings() {
  if (!deviceId.value) return;
  savingMappings.value = true;
  mappingError.value = null;
  try {
    mappedSelection.value = await deviceStore.saveMappedSpaces(deviceId.value, [
      ...new Set(mappedSelection.value),
    ]);
    mappingsDirty.value = false;
  } catch (error) {
    mappingError.value = String(error);
  } finally {
    savingMappings.value = false;
  }
}

// The switch on an existing channel (ADR-0008 D15). Refused by the backend
// when the channel cannot work on this device (D6); the error says why.
async function toggleChannel(kind: ChannelKind, enabled: boolean) {
  if (!deviceId.value || busyChannel.value) return;
  busyChannel.value = kind;
  channelError.value = null;
  try {
    await deviceStore.setChannelEnabled(deviceId.value, kind, enabled);
  } catch (error) {
    channelError.value = String(error);
    await deviceStore.refreshChannelStatuses(deviceId.value);
  } finally {
    busyChannel.value = null;
  }
}

// "Add" on a channel that does not exist yet opens the setup dialog.
function addChannel(kind: ChannelKind) {
  channelBeingAdded.value = kind;
}

async function closeChannelSetup() {
  channelBeingAdded.value = null;
  if (deviceId.value) await deviceStore.refreshChannelStatuses(deviceId.value);
}

// Forgetting a channel, as opposed to switching it off. The backend
// refuses while it is still on, and the row's own control is disabled
// until then -- this catch is for the race, not the ordinary path.
async function unlinkChannel(kind: ChannelKind) {
  if (!deviceId.value || busyChannel.value) return;
  busyChannel.value = kind;
  channelError.value = null;
  try {
    await deviceStore.unlinkChannel(deviceId.value, kind);
  } catch (error) {
    channelError.value = String(error);
  } finally {
    busyChannel.value = null;
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
  const lastSynced = lastSyncedAtBySpace.value[spaceId];
  return lastSynced ? `last synced: ${new Date(lastSynced).toLocaleString()}` : "Mapped";
}
</script>

<template>
  <div class="flex flex-col gap-4 pb-24">
    <header class="flex items-center gap-2">
      <router-link to="/settings" class="btn btn-ghost btn-sm gap-1 pl-1 pr-2 font-medium">
        <ChevronLeftIcon class="size-4" />
        Settings
      </router-link>
    </header>

    <template v-if="device">
      <!-- The name is the header. Inline renaming is designed but not built
           yet -- see issue #117, which owns the editable affordance. -->
      <h1 class="truncate px-2 text-base font-semibold tracking-tight">{{ device.display_name }}</h1>

      <section class="rounded-xl bg-base-200 p-3">
        <h2 class="mb-2 text-sm font-semibold uppercase tracking-wide opacity-70">Channels</h2>
        <ul class="flex list-none flex-col overflow-hidden rounded-lg">
          <ChannelRow
            v-for="status in channelStatuses"
            :key="status.kind"
            :status="status"
            :busy="busyChannel === status.kind"
            @toggle="(next) => toggleChannel(status.kind, next)"
            @add="addChannel(status.kind)"
            @unlink="unlinkChannel(status.kind)"
          />
        </ul>
        <p v-if="channelError" class="mt-2 text-xs text-error">{{ channelError }}</p>
      </section>

      <section class="rounded-xl bg-base-200 p-3">
        <h2 class="mb-2 text-sm font-semibold uppercase tracking-wide opacity-70">Shared spaces</h2>
        <div class="flex flex-col gap-2">
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
                  class="checkbox checkbox-sm checkbox-success"
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
                >{{ mappedSpaceEndLabel(space.id) }}</span>
                <span
                  v-if="!isBuiltinSpace(space.id)"
                  class="text-xs opacity-60"
                  :title="space.id"
                >{{ shortUuid(space.id) }}</span>
              </template>
            </SettingsListItem>
            <SettingsListItem v-if="spaceStore.spaces.length === 0">
              <span class="opacity-70">No spaces available.</span>
            </SettingsListItem>
          </SettingsListGroup>
          <div
            v-if="unresolvedCustomSpaces.length > 0"
            class="rounded-lg border border-warning/30 bg-base-100 p-3 text-xs"
          >
            <p class="mb-2 font-medium text-warning">Incoming custom spaces need resolution</p>
            <p class="opacity-70">
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
            >{{ savingMappings ? "Saving..." : "Save mappings" }}</button>
            <button
              class="btn btn-sm btn-ghost"
              :disabled="savingMappings"
              @click="void loadDeviceState()"
            >Reload</button>
          </div>
        </div>
      </section>

      <SyncQueueSection
        :summary="syncQueue"
        :peer-name="peerName"
        :any-channel-connected="anyChannelConnected"
      />

      <!-- Last, on its own, as a text button: a red block here would
           compete with the channel switches above it for attention it
           doesn't deserve. -->
      <button
        class="w-fit px-2 text-left text-[12.5px] font-medium text-error"
        data-testid="unlink-device"
        @click="openUnpairDialog"
      >
        Unlink {{ device.display_name }}
      </button>
    </template>

    <section v-else class="rounded-xl bg-base-200 p-3">
      <p class="text-sm opacity-70">Device not found.</p>
      <router-link to="/settings" class="btn btn-sm mt-2">Back to settings</router-link>
    </section>

    <DeviceSetupDialog
      v-if="renderFlags.channelSetupDialog"
      :open="true"
      :peer-device-id="device?.peer_device_id ?? null"
      :peer-name="peerName"
      :kind="channelBeingAdded"
      @close="closeChannelSetup"
    />

    <dialog ref="unpairDialog" class="modal" data-testid="unlink-dialog">
      <div class="modal-box">
        <h3 class="text-base font-semibold">Unlink {{ device?.display_name }}?</h3>
        <!-- Two facts, in this order: what stops, and that nothing is lost.
             The second is what makes this decision safe to make. -->
        <p class="mt-2 text-sm opacity-70">
          <template v-if="mappedSpaceNames.length > 0">
            <b>{{ mappedSpaceNames.join(", ") }}</b>
            {{ mappedSpaceNames.length > 1 ? "stop" : "stops" }} syncing. Nothing is deleted.
          </template>
          <template v-else-if="!mappingsLoaded">
            Checking what is shared with this device… Nothing is deleted.
          </template>
          <template v-else>
            No spaces are shared with this device, so nothing stops syncing. Nothing is deleted.
          </template>
        </p>
        <div class="modal-action">
          <form method="dialog">
            <button class="btn btn-ghost btn-sm">Cancel</button>
          </form>
          <button class="btn btn-error btn-sm" @click="void confirmUnpair()">Unlink</button>
        </div>
      </div>
      <form method="dialog" class="modal-backdrop">
        <button>close</button>
      </form>
    </dialog>
  </div>
</template>
