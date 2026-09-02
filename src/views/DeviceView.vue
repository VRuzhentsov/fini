<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { StarIcon } from "@heroicons/vue/24/solid";
import { InformationCircleIcon } from "@heroicons/vue/24/outline";
import SettingsListGroup from "../components/SettingsView/SettingsListGroup.vue";
import SettingsListItem from "../components/SettingsView/SettingsListItem.vue";
import { useDeviceStore, type DeviceTransportStatus } from "../stores/device";
import { useSpaceStore, isBuiltinSpace } from "../stores/space";
import { shortUuid } from "../utils/shortUuid";
import { formatMacAddress, macAddressHexDigits } from "../utils/macAddress";
import { transportStatusText } from "../utils/transportStatusCodes";

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
// FOUND_ENABLED: the backend's bond check confirmed OS pairing and enabled
// the transport as part of the find itself -- pressing "Enable Bluetooth"
// below is then a no-op, and the UI should say so plainly instead of
// implying another step is still needed (see `findViaBluetooth`'s doc
// comment and `persist_bluetooth_address_and_maybe_enable` backend-side).
// FOUND_NOT_ENABLED: the address was found but the bond check didn't
// confirm OS pairing, so it genuinely does still need "Enable Bluetooth"
// pressed once OS pairing is done.
const BLUETOOTH_FIND_RESULT = {
  FOUND_ENABLED: "found_enabled",
  FOUND_NOT_ENABLED: "found_not_enabled",
  NOT_FOUND: "not_found",
} as const;
type BluetoothFindResult = (typeof BLUETOOTH_FIND_RESULT)[keyof typeof BLUETOOTH_FIND_RESULT] | null;
const bluetoothFindResult = ref<BluetoothFindResult>(null);
let findBluetoothGeneration = 0;
const mappingError = ref<string | null>(null);
const bluetoothTransportError = ref<string | null>(null);
const mappingsDirty = ref(false);
const settingPreferredTransport = ref<"network" | "bluetooth" | null>(null);
const preferredTransportError = ref<string | null>(null);
const retryingBluetoothDial = ref(false);

const deviceId = computed(() => String(route.params.id ?? ""));
const device = computed(() => deviceStore.findPairedDevice(deviceId.value));
const online = computed(() => (device.value ? deviceStore.isDeviceOnline(device.value) : false));
const syncStatus = computed(() => {
  if (!deviceId.value) return null;
  return deviceStore.getSpaceSyncStatus(deviceId.value);
});
const hasPendingSync = computed(() => (syncStatus.value?.pending_event_count ?? 0) > 0);

// fini-frontend: template render decisions belong in a named renderFlags
// key, not an ad hoc expression (or function call) inline in `v-if`.
const renderFlags = computed(() => ({
  bluetoothFindResult: bluetoothFindResult.value !== null,
}));
const presenceLabel = computed(() => (online.value ? "Online" : "Offline"));
const transportStatuses = computed(() => {
  if (!deviceId.value) return [];
  return deviceStore.getTransportStatuses(deviceId.value);
});
// A manual pin (device.preferred_transport) always wins over the backend's
// own primary-selection signal (status.primary) for display purposes --
// once the user has pinned a transport, that's the one actually governing
// which becomes primary once connected, and the star should track it, not
// whichever transport happens to already be primary right now.
const starredTransportKind = computed<"network" | "bluetooth" | null>(() => {
  const pinned = device.value?.preferred_transport;
  if (pinned === "network" || pinned === "bluetooth") return pinned;
  return transportStatuses.value.find((status) => status.primary)?.kind ?? null;
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

// Drives the "stuck connecting" timeout below. Updated on the same 5s tick
// that already polls live state -- no need for a finer-grained clock than
// that to notice a 30s threshold has passed.
const now = ref(Date.now());
const CONNECTING_TIMEOUT_MS = 30_000;
// First-seen timestamp per row, keyed by transport kind -- this view only
// ever shows the two rows for one device, so kind alone is enough. Reset on
// device navigation (see the `deviceId` watcher below) so a slow row on one
// device doesn't report as instantly "stuck" on the next.
const connectingSince = ref<Partial<Record<DeviceTransportStatus["kind"], number>>>({});

watch(
  transportStatuses,
  (statuses) => {
    for (const status of statuses) {
      const connecting = status.state.state === "configured" && status.state.code?.code === "connecting";
      if (connecting) {
        if (!(status.kind in connectingSince.value)) connectingSince.value[status.kind] = Date.now();
      } else {
        delete connectingSince.value[status.kind];
      }
    }
  },
  { immediate: true },
);

function isStuckConnecting(status: DeviceTransportStatus): boolean {
  const since = connectingSince.value[status.kind];
  return status.state.code?.code === "connecting" && since !== undefined && now.value - since > CONNECTING_TIMEOUT_MS;
}

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
    now.value = Date.now();
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
  connectingSince.value = {};
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
      // `findBluetoothAddress` already reloaded paired-device state above,
      // so `device` reflects whatever the backend's bond check just
      // decided -- see `bluetoothFindResult`'s doc comment.
      bluetoothFindResult.value = device.value?.bluetooth_enabled
        ? BLUETOOTH_FIND_RESULT.FOUND_ENABLED
        : BLUETOOTH_FIND_RESULT.FOUND_NOT_ENABLED;
    } else {
      bluetoothFindResult.value = BLUETOOTH_FIND_RESULT.NOT_FOUND;
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

// Single source of truth for the Find-via-Bluetooth result message and its
// styling -- keeps `bluetoothFindResult`'s three-way branch out of the
// template. FOUND_ENABLED reads as done (success/green): the backend
// already enabled the transport as part of the find, so pressing "Enable
// Bluetooth" below would be a redundant no-op -- the previous copy ("Found
// and confirmed nearby.") implied more work was needed when there wasn't
// any, which was the actual source of user confusion this fixes.
function bluetoothFindResultText(): string | null {
  switch (bluetoothFindResult.value) {
    case BLUETOOTH_FIND_RESULT.FOUND_ENABLED:
      return "Found and enabled — Bluetooth is on for this pair. No further action needed.";
    case BLUETOOTH_FIND_RESULT.FOUND_NOT_ENABLED:
      return "Address found, but OS Bluetooth pairing wasn't confirmed yet. Pair in system Bluetooth settings, then press Enable Bluetooth below.";
    case BLUETOOTH_FIND_RESULT.NOT_FOUND:
      return "Not found within 60s — make sure the other device is nearby and its Bluetooth is on, or enter its address manually below.";
    case null:
      return null;
  }
}

function bluetoothFindResultClass(): string {
  return bluetoothFindResult.value === BLUETOOTH_FIND_RESULT.FOUND_ENABLED ? "text-success" : "opacity-60";
}

// ADR-0003 Phase 3: click-to-pin. Disabled for Unconfigured (Gray) rows --
// there's nothing to switch to yet -- but otherwise available regardless of
// whether the row is already live, matching the grill-me answer that this
// should "force an immediate switch right now" even when re-clicking the
// transport that's already preferred/live, rather than being a no-op toggle.
async function pinTransport(kind: "network" | "bluetooth") {
  if (!deviceId.value) return;
  settingPreferredTransport.value = kind;
  preferredTransportError.value = null;

  try {
    await deviceStore.setPreferredTransport(deviceId.value, kind);
  } catch (error) {
    preferredTransportError.value = String(error);
  } finally {
    settingPreferredTransport.value = null;
  }
}

// Real-device evidence (2026-09-01): a flaky Bluetooth link kept retrying
// silently in the background for minutes with nothing but an unchanging
// "Still connecting..." to show for it -- the backend now gives up after
// one minute of no successful auth and reports `bluetooth_dial_exhausted`
// (gray, "Unavailable") instead of retrying forever. This is the row's
// "tap to try again" side of that: only meaningful for that one code, on
// the bluetooth row.
function isRetryableBluetoothDial(status: DeviceTransportStatus): boolean {
  return (
    status.kind === "bluetooth" &&
    status.state.state === "unconfigured" &&
    status.state.code.code === "bluetooth_dial_exhausted"
  );
}

async function retryBluetoothDial() {
  if (!deviceId.value || retryingBluetoothDial.value) return;
  retryingBluetoothDial.value = true;
  try {
    await deviceStore.retryBluetoothDial(deviceId.value);
  } finally {
    retryingBluetoothDial.value = false;
  }
}

// Single source of truth for whether a transport row is interactive at
// all, so the template's `:button` binding never has to branch itself.
function isTransportRowClickable(status: DeviceTransportStatus): boolean {
  return status.state.state !== "unconfigured" || isRetryableBluetoothDial(status);
}

// Single click handler for every transport row -- keeps the branching (retry
// vs. pin-as-preferred vs. no-op) out of the template, which only ever calls
// this one function.
function handleTransportRowClick(status: DeviceTransportStatus) {
  if (isRetryableBluetoothDial(status)) {
    void retryBluetoothDial();
    return;
  }
  if (status.state.state !== "unconfigured" && settingPreferredTransport.value === null) {
    void pinTransport(status.kind);
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

// ADR-0003 revision: gray -> amber -> green per row, independent of which
// transport is primary. The ring (border) is `primary`'s own signal --
// "this is the transport actually carrying application traffic right
// now" -- so a row can be green and unbordered (connected, healthy, just
// not the one in use) or bordered while still amber (chosen as primary
// the moment it connected, before its first ping/ack cycle completed).
function rowDotClass(status: DeviceTransportStatus): string {
  if (status.state.state === "unconfigured") return "bg-gray-400";
  const isGreen = status.state.code === null;
  const color = isGreen ? "bg-green-500" : "bg-amber-400";
  if (!status.primary) return color;
  const ring = isGreen ? "ring-green-600" : "ring-amber-500";
  return `${color} ring-2 ${ring} ring-offset-2 ring-offset-base-200`;
}

function rowDetailText(status: DeviceTransportStatus): string {
  if (status.state.state === "unconfigured") return "Unavailable";
  if (status.state.code !== null) return isStuckConnecting(status) ? "Still connecting…" : "Connecting…";
  return status.primary ? "Connected now" : "Ready";
}

// Single source of truth for the row's trailing label -- folds in the two
// transient in-flight states (retrying, switching preferred transport) on
// top of `rowDetailText`'s steady-state text, so the template just renders
// this one call.
function rowActionLabel(status: DeviceTransportStatus): string {
  if (retryingBluetoothDial.value && isRetryableBluetoothDial(status)) return "Retrying…";
  if (settingPreferredTransport.value === status.kind) return "Switching…";
  return rowDetailText(status);
}

// The "i" tooltip only has something to say when there's a code to explain
// -- a fully green, primary-or-not row has no error to surface. Past the
// 30s timeout, "connecting" specifically swaps its usual (accurate but
// static) explanation for something actionable, since by then the plain
// "no session established yet" text has stopped telling the user anything
// new -- see `CONNECTING_TIMEOUT_MS`/`isStuckConnecting`.
function rowStatusTooltip(status: DeviceTransportStatus): string | null {
  const code = status.state.code;
  if (!code) return null;
  if (isStuckConnecting(status)) {
    const peerHint = status.kind === "bluetooth" ? "Bluetooth is on for both devices" : "both devices are on the same network";
    return `Still trying after 30+ seconds. Check that the peer device is powered on, nearby, and that ${peerHint}.`;
  }
  return transportStatusText(code);
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
          :button="isTransportRowClickable(status)"
          @click="handleTransportRowClick(status)"
        >
          <template #leading>
            <span class="h-2.5 w-2.5 rounded-full" :class="rowDotClass(status)" />
          </template>
          <template #start>
            <span class="font-medium">{{ status.kind === "network" ? "Network" : "Bluetooth" }}</span>
            <StarIcon
              v-if="starredTransportKind === status.kind"
              class="ml-1.5 inline-block h-3 w-3 align-text-top opacity-60"
              :aria-label="device?.preferred_transport ? 'Pinned' : 'Automatically preferred'"
            />
            <div
              v-if="rowStatusTooltip(status)"
              class="tooltip tooltip-bottom ml-1 inline-block align-text-top"
              :data-tip="rowStatusTooltip(status)"
            >
              <InformationCircleIcon
                class="size-4 opacity-60"
                :aria-label="rowStatusTooltip(status) ?? undefined"
                data-testid="transport-status-info"
              />
            </div>
          </template>
          <template #end>
            <span class="text-xs opacity-60">{{ rowActionLabel(status) }}</span>
          </template>
        </SettingsListItem>
      </SettingsListGroup>
      <p v-if="preferredTransportError" class="mt-2 text-xs text-error">{{ preferredTransportError }}</p>
      <div class="mt-3 flex flex-col gap-2">
        <button
          class="btn btn-sm btn-outline w-fit"
          data-testid="find-bluetooth-address"
          :disabled="findingBluetoothAddress"
          @click="void findViaBluetooth()"
        >{{ findingBluetoothAddress ? "Scanning… (up to 60s)" : "Find via Bluetooth" }}</button>
        <p v-if="renderFlags.bluetoothFindResult" class="text-xs" :class="bluetoothFindResultClass()">
          {{ bluetoothFindResultText() }}
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
