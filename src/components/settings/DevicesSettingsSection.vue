<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { PlusIcon } from "@heroicons/vue/24/outline";
import SettingsListGroup from "./SettingsListGroup.vue";
import SettingsListItem from "./SettingsListItem.vue";
import PairDeviceDialog from "./PairDeviceDialog.vue";
import { useDeviceStore, type PairedDevice } from "../../stores/device";
import { channelRowState, channelStatusText } from "../../utils/channelStatusCodes";

const props = defineProps<{
  // Bumped by the Settings search when someone picks "Add device". A counter
  // rather than a boolean because the same request can be made twice: open
  // the dialog, close it, search again. A flag would already be true the
  // second time and nothing would happen.
  pairRequests?: number;
}>();

const deviceStore = useDeviceStore();
const pairDialogOpen = ref(false);

watch(
  () => props.pairRequests ?? 0,
  (requests, previous) => {
    if (requests > (previous ?? 0)) pairDialogOpen.value = true;
  },
);

function closePairDialog() {
  pairDialogOpen.value = false;
}

// Someone asking to pair surfaces here, on the devices page, rather than
// only inside the add-device flow: a request that expires unseen because
// the user happened not to be on the right screen is the worst outcome of
// the whole ceremony.
const incoming = computed(() => deviceStore.incomingRequests);

// One line under each device name, in the same plain language the device
// page uses. Built from whatever channel state is already cached -- the
// presence loop refreshes it for every presenced peer, so this costs no
// extra calls, and degrades to a bare "Not connected" for a peer nothing
// has looked at yet rather than inventing a reason.
function deviceSummary(device: PairedDevice): string {
  const statuses = deviceStore.getChannelStatuses(device.peer_device_id);
  if (statuses.length === 0) {
    return deviceStore.isDeviceOnline(device) ? "Connected" : "Not connected";
  }

  const live = statuses.find(
    (status) => channelRowState(status.state, status.enabled) === "connected",
  );
  if (live) {
    return `${live.kind === "network" ? "Network" : "Bluetooth"} · connected`;
  }

  // Nothing is connected, so say why -- preferring whichever channel is
  // actually switched on, since a channel the user turned off explains
  // nothing about why the device is unreachable.
  const candidate = statuses.find((status) => status.enabled && status.state.code) ?? statuses[0];
  const reason = candidate?.state.code
    ? channelStatusText(candidate.state.code, device.display_name)
    : null;
  return reason ? `Not connected · ${reason.toLowerCase()}` : "Not connected";
}
</script>

<template>
  <section class="rounded-xl bg-base-200 p-3" data-testid="settings-devices">
    <h2 class="mb-3 text-sm font-semibold uppercase tracking-wide opacity-70">Devices</h2>

    <div
      v-for="request in incoming"
      :key="request.request_id"
      class="mb-2 overflow-hidden rounded-lg bg-base-100 ring-[1.5px] ring-primary"
      data-testid="incoming-request-row"
      :data-from-hostname="request.from_hostname"
    >
      <div class="flex items-center gap-2.5 px-2 py-2.5">
        <span class="size-2.5 shrink-0 rounded-full bg-warning" />
        <span class="min-w-0 flex-1 text-sm font-medium">
          {{ request.from_hostname }} wants to pair
          <small class="mt-0.5 block text-[11px] font-normal text-[var(--fg-3)]">Asking now</small>
        </span>
      </div>
      <div class="flex gap-2 px-3 pb-2.5">
        <button class="btn btn-primary btn-xs" data-testid="open-pair-request" @click="pairDialogOpen = true">
          Open request
        </button>
        <button
          class="btn btn-ghost btn-xs"
          @click="void deviceStore.rejectIncomingRequest(request.request_id)"
        >Decline</button>
      </div>
    </div>

    <SettingsListGroup>
      <SettingsListItem
        v-for="device in deviceStore.pairedDevices"
        :key="device.peer_device_id"
        :to="`/settings/device/${device.peer_device_id}`"
        data-testid="paired-device-row"
        :data-peer-device-id="device.peer_device_id"
      >
        <template #leading>
          <span
            class="size-2.5 rounded-full"
            :class="deviceStore.isDeviceOnline(device) ? 'bg-success' : 'bg-[var(--fg-5)]'"
          />
        </template>
        <template #start>
          <span class="block truncate font-medium" data-testid="paired-device-name">
            {{ device.display_name }}
          </span>
          <span class="block truncate text-[11px] text-[var(--fg-3)]" data-testid="paired-device-summary">
            {{ deviceSummary(device) }}
          </span>
        </template>
        <template #trailing><span class="text-sm opacity-50">›</span></template>
      </SettingsListItem>

      <SettingsListItem v-if="deviceStore.pairedDevices.length === 0">
        <span class="opacity-70">No paired devices yet</span>
      </SettingsListItem>

      <SettingsListItem button testid="add-device-link" @click="pairDialogOpen = true">
        <template #leading><PlusIcon class="size-4" /></template>
        <template #start><span class="font-medium">Add device</span></template>
        <template #trailing><span class="text-sm opacity-50">›</span></template>
      </SettingsListItem>
    </SettingsListGroup>

    <PairDeviceDialog :open="pairDialogOpen" @close="closePairDialog()" />
  </section>
</template>
