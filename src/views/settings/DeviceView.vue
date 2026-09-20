<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { ChevronLeftIcon } from "@heroicons/vue/24/outline";
import SettingsListGroup from "../../components/settings/SettingsListGroup.vue";
import SettingsListItem from "../../components/settings/SettingsListItem.vue";
import ChannelRow from "../../components/settings/device/ChannelRow.vue";
import SyncQueueSection from "../../components/settings/device/SyncQueueSection.vue";
import ChannelSetupDialog from "../../components/settings/device/ChannelSetupDialog.vue";
import { useDeviceStore, type ChannelKind } from "../../stores/device";
import { useSpaceStore, isBuiltinSpace } from "../../stores/space";
import { shortUuid } from "../../utils/shortUuid";
import { channelRowState } from "../../utils/channelStatusCodes";

const route = useRoute();
const router = useRouter();
const deviceStore = useDeviceStore();
const spaceStore = useSpaceStore();

const unpairDialog = ref<HTMLDialogElement | null>(null);
const channelSetupOpen = ref(false);
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

// The star is the user's stored choice, not whichever channel happens to
// be carrying traffic at this moment -- so it stays put across a
// disconnect, and says what will govern the next reconnect. Null when they
// have not chosen and selection is automatic.
const starredChannel = computed<ChannelKind | null>(
  () => channelStatuses.value.find((status) => status.primary)?.kind ?? null,
);

// Drives the sync queue's "sending now" vs "nothing can reach it" line.
// Anything actually carrying a proven link counts, primary or not.
const anyChannelConnected = computed(() =>
  channelStatuses.value.some(
    (status) => channelRowState(status.state, status.enabled) === "connected",
  ),
);

// fini-frontend: template render decisions belong in a named renderFlags
// key, not an ad hoc expression inline in `v-if`.
const renderFlags = computed(() => ({
  bluetoothSetupOffered: !channelStatuses.value.some(
    (status) => status.kind === "bluetooth" && status.configured,
  ),
}));

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
const mappedSpaceNames = computed(() =>
  mappedSelection.value
    .map((id) => spaceStore.spaces.find((space) => space.id === id)?.name)
    .filter((name): name is string => Boolean(name)),
);

const LIVE_POLL_INTERVAL_MS = 5_000;
let livePollTimer: ReturnType<typeof setInterval> | null = null;

onMounted(() => {
  void deviceStore.hydrate();
  void spaceStore.fetchSpaces();
  void deviceStore.runSpaceSyncTick();
  void loadDeviceState();

  // The store's periodic presence loop only touches paired devices that
  // appear in the *network* presence snapshot, which a Bluetooth-only
  // session never does -- without an independent poll here, a row goes
  // stale the moment such a session connects or drops while this page is
  // open. Uses the lightweight liveness call, not the full status reload:
  // only the live connected state needs to be this fresh.
  livePollTimer = setInterval(() => {
    if (!deviceId.value) return;
    void deviceStore.refreshLiveConnectedState(deviceId.value);
    void deviceStore.refreshSyncQueue(deviceId.value);
  }, LIVE_POLL_INTERVAL_MS);
});

onUnmounted(() => {
  if (livePollTimer) {
    clearInterval(livePollTimer);
    livePollTimer = null;
  }
});

watch(deviceId, () => {
  mappingsDirty.value = false;
  channelError.value = null;
  channelSetupOpen.value = false;
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

// One handler for both channels. Turning a channel on never fails and never
// snaps the switch back: if the condition it needs isn't met the channel
// stays on and starts by itself, and the row says why. The only thing the
// probe changes is *when* the row can say it -- immediately, rather than
// whenever the background dial loop next tries.
async function toggleChannel(kind: ChannelKind, enabled: boolean) {
  if (!deviceId.value || busyChannel.value) return;
  busyChannel.value = kind;
  channelError.value = null;
  try {
    await deviceStore.setChannelEnabled(deviceId.value, kind, enabled);
    if (kind === "bluetooth" && enabled) {
      await deviceStore.probeBluetoothAdapter();
      await deviceStore.refreshChannelStatuses(deviceId.value);
    }
  } catch (error) {
    channelError.value = String(error);
  } finally {
    busyChannel.value = null;
  }
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

async function pinChannel(kind: ChannelKind) {
  if (!deviceId.value || busyChannel.value) return;
  busyChannel.value = kind;
  channelError.value = null;
  try {
    await deviceStore.setPrimaryChannel(deviceId.value, kind);
  } catch (error) {
    channelError.value = String(error);
  } finally {
    busyChannel.value = null;
  }
}

async function retryChannel() {
  if (!deviceId.value || busyChannel.value) return;
  busyChannel.value = "bluetooth";
  try {
    await deviceStore.retryBluetoothDial(deviceId.value);
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
            :starred="starredChannel === status.kind"
            :peer-name="peerName"
            :busy="busyChannel === status.kind"
            @toggle="(next) => toggleChannel(status.kind, next)"
            @pin="pinChannel(status.kind)"
            @retry="retryChannel()"
            @unlink="unlinkChannel(status.kind)"
          />
        </ul>
        <p v-if="channelError" class="mt-2 text-xs text-error">{{ channelError }}</p>
        <button
          v-if="renderFlags.bluetoothSetupOffered"
          class="btn btn-ghost btn-sm mt-2 w-fit"
          data-testid="add-channel"
          @click="channelSetupOpen = true"
        >
          Set up Bluetooth
        </button>
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

    <ChannelSetupDialog
      v-if="device"
      :open="channelSetupOpen"
      :peer-device-id="device.peer_device_id"
      :peer-name="peerName"
      @close="channelSetupOpen = false"
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
