<script setup lang="ts">
import { computed, ref } from "vue";
import { StarIcon as StarSolid } from "@heroicons/vue/24/solid";
import { StarIcon as StarOutline, InformationCircleIcon } from "@heroicons/vue/24/outline";
import ChannelIcon from "./ChannelIcon.vue";
import type { DeviceTransportStatus } from "../../stores/device";
import { channelRowLabel, channelRowState, transportStatusText } from "../../utils/transportStatusCodes";

const props = defineProps<{
  status: DeviceTransportStatus;
  // The pair's own switch for this channel -- `network_enabled` /
  // `bluetooth_enabled`. Separate from `status`, because "off" is a fact
  // about what the user chose and every other row state is a fact about
  // the link.
  enabled: boolean;
  // Which channel the star sits on. A manual pin wins over the backend's
  // automatic choice; the parent resolves that and passes the answer.
  starred: boolean;
  peerName: string;
  busy: boolean;
}>();

const emit = defineEmits<{
  toggle: [enabled: boolean];
  pin: [];
  retry: [];
}>();

const CHANNEL_NAME = { network: "Network", bluetooth: "Bluetooth" } as const;

// Starts open for the states the user has just caused and needs explaining
// -- switching a channel on while the radio is off being the one that
// replaced a toast. Everything else waits to be asked.
const reasonOpen = ref(false);

const rowState = computed(() => channelRowState(props.status.state, props.enabled));
const label = computed(() => channelRowLabel(rowState.value));

// The row's own reason, in the person's words. `off` says nothing: the
// user turned it off and does not need that explained back to them.
const reason = computed(() => {
  if (rowState.value === "off") return null;
  const code = props.status.state.code;
  return code ? transportStatusText(code, props.peerName) : null;
});

// Auto-opened rather than click-to-open for the one state that is both
// persistent and not the user's doing -- see `transportStatusCodes`'s note
// on why this replaced the design's toast.
const reasonPinnedOpen = computed(
  () => props.status.state.code?.code === "bluetooth_adapter_off",
);
const showReason = computed(() => reason.value !== null && (reasonOpen.value || reasonPinnedOpen.value));

// Only a connected channel can carry traffic, so only a connected channel
// can hold the star. Offering it on a dead row would promise a switch that
// silently does nothing.
const canPin = computed(() => rowState.value === "connected" || rowState.value === "fading");

const retryable = computed(
  () =>
    props.status.state.state === "unconfigured" &&
    props.status.state.code.code === "bluetooth_dial_exhausted",
);

const dotClass = computed(() => {
  switch (rowState.value) {
    case "connected":
      return "bg-success";
    case "connecting":
    case "fading":
      return "bg-warning";
    default:
      // Gray covers off, waiting and down alike: none of them is an error,
      // and colouring "not connected" red would make an ordinary state look
      // like a fault.
      return "bg-[var(--fg-5)]";
  }
});
</script>

<template>
  <li
    class="flex flex-col gap-1 border-b border-base-200 bg-base-100 px-2 py-1.5 last:border-b-0"
    data-testid="transport-status-row"
    :data-transport-kind="status.kind"
    :data-channel-state="rowState"
  >
    <div class="flex flex-wrap items-center gap-x-2 gap-y-2">
      <component
        :is="canPin ? 'button' : 'div'"
        :type="canPin ? 'button' : undefined"
        class="-m-0.5 flex min-w-0 flex-1 items-center gap-2 rounded-md p-0.5 text-left"
        :class="canPin ? 'cursor-pointer hover:bg-base-200' : ''"
        :aria-pressed="canPin ? starred : undefined"
        :aria-label="canPin ? `Make ${CHANNEL_NAME[status.kind]} primary` : undefined"
        @click="canPin && emit('pin')"
      >
        <span class="size-2.5 shrink-0 rounded-full" :class="dotClass" />
        <ChannelIcon :kind="status.kind" class="size-3.5 shrink-0 opacity-70" />
        <span class="truncate text-sm font-semibold">{{ CHANNEL_NAME[status.kind] }}</span>
        <span class="truncate text-[11px] text-[var(--fg-3)]">{{ label }}</span>

        <button
          v-if="reason && !reasonPinnedOpen"
          type="button"
          class="shrink-0 text-[var(--fg-4)] hover:text-[var(--fg-2)]"
          data-testid="transport-status-info"
          :aria-label="reason"
          :aria-expanded="reasonOpen"
          @click.stop="reasonOpen = !reasonOpen"
        >
          <InformationCircleIcon class="size-3.5" />
        </button>

        <span
          v-if="canPin"
          class="ml-auto shrink-0"
          data-testid="channel-star"
          :data-starred="starred"
          :class="starred ? 'text-warning' : 'text-[var(--fg-5)]'"
        >
          <component :is="starred ? StarSolid : StarOutline" class="size-3.5" />
        </span>
      </component>

      <button
        v-if="retryable"
        type="button"
        class="btn btn-ghost btn-xs shrink-0"
        data-testid="retry-bluetooth-dial"
        :disabled="busy"
        @click="emit('retry')"
      >
        Try again
      </button>

      <button
        type="button"
        class="shrink-0"
        role="switch"
        data-testid="channel-switch"
        :aria-checked="enabled"
        :aria-label="`Turn ${CHANNEL_NAME[status.kind]} ${enabled ? 'off' : 'on'}`"
        :disabled="busy"
        @click="emit('toggle', !enabled)"
      >
        <!-- The knob sits in the on position whenever the switch is on, but
             the track stays gray until the channel can actually carry
             something. That is what "on, waiting" looks like: the user's
             choice is honoured and the state is not overstated. -->
        <span
          class="flex h-[22px] w-[38px] items-center rounded-full p-0.5 transition-colors"
          :class="enabled ? (rowState === 'waiting' ? 'bg-[var(--fg-5)]' : 'bg-success') : 'bg-base-300'"
        >
          <span
            class="size-[18px] rounded-full bg-base-100 shadow-sm transition-transform"
            :class="enabled ? 'translate-x-4' : ''"
          />
        </span>
      </button>
    </div>

    <p
      v-if="showReason"
      class="pl-[18px] text-[11px] leading-snug text-[var(--fg-2)]"
      data-testid="transport-status-reason"
    >
      {{ reason }}
    </p>
  </li>
</template>
