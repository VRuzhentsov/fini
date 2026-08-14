<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import SettingsListGroup from "../components/SettingsView/SettingsListGroup.vue";
import SettingsListItem from "../components/SettingsView/SettingsListItem.vue";
import { useDeviceStore, type DeviceTransportRowState } from "../stores/device";
import { useSpaceStore, isBuiltinSpace } from "../stores/space";
import { shortUuid } from "../utils/shortUuid";
import { formatMacAddress, macAddressHexDigits } from "../utils/macAddress";

const route = useRoute();
const router = useRouter();
const deviceStore = useDeviceStore();
const spaceStore = useSpaceStore();
const unpairDialog = ref<HTMLDialogElement | null>(null);
const mappedSelection = ref<string[]>([]);
// Stores just the raw hex digits (no colons); the input mask formats them
// with colons on display and strips any pasted/typed separators back out
// on write, so the user never has to type ":" themselves.
const bluetoothAddressHexDigits = ref("");
const bluetoothAddressInput = computed({
  get: () => formatMacAddress(bluetoothAddressHexDigits.value),
  set: (value: string) => {
    bluetoothAddressHexDigits.value = macAddressHexDigits(value);
  },
});
const mappingsLoaded = ref(false);
const savingMappings = ref(false);
const savingBluetoothTransport = ref(false);
const findingBluetoothAddress = ref(false);
const bluetoothFindResult = ref<"found" | "not_found" | null>(null);
let findBluetoothGeneration = 0;
const mappingError = ref<string | null>(null);
const bluetoothTransportError = ref<string | null>(null);
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
const transportStatuses = computed(() => {
  if (!deviceId.value) return [];
  return deviceStore.getTransportStatuses(deviceId.value);
});
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

const TRANSPORT_STATUS_POLL_INTERVAL_MS = 5_000;
let transportStatusTimer: ReturnType<typeof setInterval> | null = null;

onMounted(() => {
  void deviceStore.hydrate();
  void spaceStore.fetchSpaces();
  void deviceStore.runSpaceSyncTick();
  void loadMappings();

  // `loadMappings` only refreshes transport status once, on mount/route
  // change -- the store's periodic presence loop only touches paired
  // devices that show up in the *network* presence snapshot, which a
  // Bluetooth-only fallback session never does. Without an independent
  // poll here, "connected now" goes stale the moment a Bluetooth-only
  // session connects or disconnects while this page stays open.
  //
  // Uses the lightweight `refreshLiveConnectedState`, not the full
  // `refreshTransportStatuses` `loadMappings` already ran once above: the
  // full check re-verifies OS Bluetooth bonding via a `bluetoothctl`
  // subprocess on Linux, which this view has no reason to rerun every 5s
  // for as long as it stays open -- only the live "connected" state
  // actually needs to be fresh here.
  transportStatusTimer = setInterval(() => {
    if (deviceId.value) void deviceStore.refreshLiveConnectedState(deviceId.value);
  }, TRANSPORT_STATUS_POLL_INTERVAL_MS);
});

onUnmounted(() => {
  if (transportStatusTimer) {
    clearInterval(transportStatusTimer);
    transportStatusTimer = null;
  }
});

watch(deviceId, () => {
  mappingsDirty.value = false;
  void loadMappings();

  // Invalidate any in-flight findViaBluetooth scan for the device we just
  // navigated away from, and reset this route's own find UI immediately.
  // findingBluetoothAddress is page-level, not per-device; without this,
  // it would stay "Scanning..." (and the button disabled) on the new
  // device's view until the old device's up-to-60s scan happens to settle.
  findBluetoothGeneration += 1;
  findingBluetoothAddress.value = false;
  bluetoothFindResult.value = null;
  bluetoothTransportError.value = null;
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
    await deviceStore.refreshTransportStatuses(deviceId.value);
    bluetoothAddressInput.value = device.value?.bluetooth_address ?? "";
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

async function saveBluetoothTransport(enabled: boolean) {
  if (!deviceId.value) return;
  savingBluetoothTransport.value = true;
  bluetoothTransportError.value = null;

  try {
    await deviceStore.setBluetoothTransport(
      deviceId.value,
      enabled,
      enabled ? bluetoothAddressInput.value : null,
    );
  } catch (error) {
    bluetoothTransportError.value = String(error);
  } finally {
    savingBluetoothTransport.value = false;
  }
}

// Phase 1 of ADR 0002: scans for up to 60s and, on a match, the backend has
// already persisted (and possibly enabled) Bluetooth for this pair -- the
// store call re-loads paired-device/transport-status state, so the address
// input just needs to pick up whatever landed there.
async function findViaBluetooth() {
  if (!deviceId.value) return;
  const requestedDeviceId = deviceId.value;
  const generation = ++findBluetoothGeneration;
  findingBluetoothAddress.value = true;
  bluetoothFindResult.value = null;
  bluetoothTransportError.value = null;

  try {
    const address = await deviceStore.findBluetoothAddress(requestedDeviceId);
    if (generation !== findBluetoothGeneration) return;
    if (address) {
      bluetoothAddressInput.value = address;
      bluetoothFindResult.value = "found";
    } else {
      bluetoothFindResult.value = "not_found";
    }
  } catch (error) {
    if (generation === findBluetoothGeneration) {
      bluetoothTransportError.value = String(error);
    }
  } finally {
    // The route-change watcher already reset `findingBluetoothAddress` for
    // the device we navigated to (if any) -- a stale generation clearing
    // it here would incorrectly cancel a newer scan's own "Scanning..." state.
    if (generation === findBluetoothGeneration) {
      findingBluetoothAddress.value = false;
    }
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

// ADR 0003 Phase 2: shared four-state model for both transport rows.
// `live` is the only state that gets a border (ring) -- the sole "this is
// the transport actually carrying the session right now" signal, replacing
// the old separate "connected now" text badge. Amber now only means
// "recently unreliable" (Configured, reliable: false), not "network is
// just doing the job instead" -- that case is Configured/reliable: true,
// green with no border.
function rowDotClass(state: DeviceTransportRowState): string {
  switch (state.state) {
    case "live":
      return "bg-green-500 ring-2 ring-green-600 ring-offset-2 ring-offset-base-200";
    case "configured":
      return state.reliable ? "bg-green-500" : "bg-amber-400";
    case "unconfigured":
      return "bg-gray-400";
  }
}

function rowDetailText(state: DeviceTransportRowState): string {
  switch (state.state) {
    case "unconfigured":
      return state.reason;
    case "configured":
      return state.reliable ? "Ready" : "Recently unreliable";
    case "live":
      return "Connected now";
  }
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
      <h2 class="mb-2 text-sm font-semibold uppercase tracking-wide opacity-70">Transports</h2>
      <SettingsListGroup>
        <SettingsListItem
          v-for="status in transportStatuses"
          :key="status.kind"
          data-testid="transport-status-row"
          :data-transport-kind="status.kind"
        >
          <template #leading>
            <span class="h-2.5 w-2.5 rounded-full" :class="rowDotClass(status.state)" />
          </template>
          <template #start>
            <span class="font-medium">{{ status.kind === "network" ? "Network" : "Bluetooth" }}</span>
            <span v-if="status.preferred" class="ml-2 text-[11px] opacity-60">preferred</span>
          </template>
          <template #end>
            <span class="text-xs opacity-60">{{ rowDetailText(status.state) }}</span>
          </template>
        </SettingsListItem>
      </SettingsListGroup>
      <div class="mt-3 flex flex-col gap-2">
        <button
          class="btn btn-sm btn-outline w-fit"
          data-testid="find-bluetooth-address"
          :disabled="findingBluetoothAddress"
          @click="void findViaBluetooth()"
        >{{ findingBluetoothAddress ? "Scanning… (up to 60s)" : "Find via Bluetooth" }}</button>
        <p v-if="bluetoothFindResult === 'found'" class="text-xs text-success">
          Found and confirmed nearby.
        </p>
        <p v-else-if="bluetoothFindResult === 'not_found'" class="text-xs opacity-60">
          Not found within 60s — make sure the other device is nearby and its Bluetooth is on, or
          enter its address manually below.
        </p>
        <label class="text-xs font-medium opacity-70" for="bluetooth-address">Bluetooth address</label>
        <input
          id="bluetooth-address"
          v-model="bluetoothAddressInput"
          class="input input-sm input-bordered"
          data-testid="bluetooth-address-input"
          placeholder="AA:BB:CC:DD:EE:FF"
          maxlength="17"
          :disabled="savingBluetoothTransport"
        />
        <p class="text-xs opacity-60">
          Enable only after this device is already paired in the OS Bluetooth settings.
        </p>
        <div v-if="bluetoothTransportError" class="text-error text-xs">{{ bluetoothTransportError }}</div>
        <div class="flex items-center gap-2">
          <button
            class="btn btn-sm btn-primary"
            data-testid="enable-bluetooth-transport"
            :disabled="savingBluetoothTransport"
            @click="void saveBluetoothTransport(true)"
          >Enable Bluetooth</button>
          <button
            class="btn btn-sm btn-ghost"
            data-testid="disable-bluetooth-transport"
            :disabled="savingBluetoothTransport || !device.bluetooth_enabled"
            @click="void saveBluetoothTransport(false)"
          >Disable</button>
        </div>
      </div>
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
              <span
                v-if="!isBuiltinSpace(space.id)"
                class="text-xs opacity-60"
                :title="space.id"
              >{{ shortUuid(space.id) }}</span>
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
