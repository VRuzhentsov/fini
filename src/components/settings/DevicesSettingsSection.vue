<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { PlusIcon, InformationCircleIcon } from "@heroicons/vue/24/outline";
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

// Which channel, if any, actually has a live session with this device.
//
// Not `isDeviceOnline`: that is presence -- a beacon heard on the LAN --
// which says a machine exists, not that we are talking to it. A desktop
// sitting discoverable with no session was showing a green dot here while
// nothing was connected.
function connectedChannel(device: PairedDevice): "network" | "bluetooth" | null {
  const live = deviceStore
    .getChannelStatuses(device.peer_device_id)
    .find((status) => channelRowState(status.state, status.enabled) === "connected");
  return live?.kind ?? null;
}

// What this device's state amounts to, in one sentence. Never rendered on
// its own line: the row is a circle, a name and an info button, and this is
// what the button reveals. A list of devices is not the place to explain
// each one's silence to someone who did not ask.
function deviceDetail(device: PairedDevice): string | null {
  const statuses = deviceStore.getChannelStatuses(device.peer_device_id);
  const kind = connectedChannel(device);
  if (kind) return `${kind === "network" ? "Network" : "Bluetooth"} · connected`;

  // Nothing is connected, so say why -- preferring whichever channel is
  // actually switched on, since a channel the user turned off explains
  // nothing about why the device is unreachable.
  const candidate = statuses.find((status) => status.enabled && status.state.code) ?? statuses[0];
  return candidate?.state.code
    ? channelStatusText(candidate.state.code, device.display_name)
    : null;
}

// Which row has its detail open. One at a time: these are one-line answers
// to "what about this one", not a panel to leave hanging open.
const detailOpen = ref<string | null>(null);

function toggleDetail(peerDeviceId: string) {
  detailOpen.value = detailOpen.value === peerDeviceId ? null : peerDeviceId;
}

// Each row shaped once, rather than asking the store the same question from
// several places in the template.
const renderLists = computed(() => ({
  devices: deviceStore.pairedDevices.map((device) => ({
    device,
    connected: connectedChannel(device) !== null,
    detail: deviceDetail(device),
    detailShown: detailOpen.value === device.peer_device_id,
  })),
}));

const renderFlags = computed(() => ({
  emptyState: renderLists.value.devices.length === 0,
}));
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
        v-for="row in renderLists.devices"
        :key="row.device.peer_device_id"
        :to="`/settings/device/${row.device.peer_device_id}`"
        data-testid="paired-device-row"
        :data-peer-device-id="row.device.peer_device_id"
      >
        <template #leading>
          <!-- `data-connected` carries what the colour means, so a test can
               assert the state rather than a Tailwind class name. -->
          <span
            class="size-2.5 rounded-full"
            data-testid="paired-device-dot"
            :data-connected="row.connected"
            :class="row.connected ? 'bg-success' : 'bg-[var(--fg-5)]'"
          />
        </template>
        <template #start>
          <span class="block truncate font-medium" data-testid="paired-device-name">
            {{ row.device.display_name }}
          </span>
          <!-- Below the name rather than beside it: the detail is a sentence,
               and a sentence in a row this narrow would push the name out. -->
          <span
            v-if="row.detailShown"
            class="block text-[11px] leading-snug text-[var(--fg-2)]"
            data-testid="paired-device-detail"
          >
            {{ row.detail }}
          </span>
        </template>
        <template #end>
          <!-- Inside a RouterLink, so the click has to be stopped from
               navigating: asking what a row means is not asking to open it. -->
          <button
            v-if="row.detail"
            type="button"
            class="shrink-0 text-[var(--fg-4)] hover:text-[var(--fg-2)]"
            data-testid="paired-device-info"
            :aria-label="row.detail"
            :aria-expanded="row.detailShown"
            @click.stop.prevent="toggleDetail(row.device.peer_device_id)"
          >
            <InformationCircleIcon class="size-4" />
          </button>
        </template>
        <template #trailing><span class="text-sm opacity-50">›</span></template>
      </SettingsListItem>

      <SettingsListItem v-if="renderFlags.emptyState">
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
