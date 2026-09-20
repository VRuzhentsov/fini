<script setup lang="ts">
import { computed, ref } from "vue";
import { StarIcon as StarSolid } from "@heroicons/vue/24/solid";
import { StarIcon as StarOutline, InformationCircleIcon, TrashIcon } from "@heroicons/vue/24/outline";
import ChannelIcon from "./ChannelIcon.vue";
import type { DeviceChannelStatus } from "../../../stores/device";
import { channelRowLabel, channelRowState, channelStatusText } from "../../../utils/channelStatusCodes";

const props = defineProps<{
  status: DeviceChannelStatus;
  // Whether this channel holds the star -- the user's stored choice of
  // which one carries the traffic. The parent resolves it across the rows
  // so exactly one can be starred.
  starred: boolean;
  peerName: string;
  busy: boolean;
}>();

const emit = defineEmits<{
  toggle: [enabled: boolean];
  pin: [];
  retry: [];
  unlink: [];
}>();

// "Off" is a fact about what the user chose; every other row state is a
// fact about the link. A channel that was never set up reads as off too --
// the difference between the two is what the page offers, not what the
// row says.
const enabled = computed(() => props.status.enabled);

const CHANNEL_NAME = { network: "Network", bluetooth: "Bluetooth" } as const;

// Starts open for the states the user has just caused and needs explaining
// -- switching a channel on while the radio is off being the one that
// replaced a toast. Everything else waits to be asked.
const reasonOpen = ref(false);

const rowState = computed(() => channelRowState(props.status.state, enabled.value));
const label = computed(() => channelRowLabel(rowState.value));

// The row's own reason, in the person's words. `off` says nothing: the
// user turned it off and does not need that explained back to them.
const reason = computed(() => {
  if (rowState.value === "off") return null;
  const code = props.status.state.code;
  return code ? channelStatusText(code, props.peerName) : null;
});

// Auto-opened rather than click-to-open for the one state that is both
// persistent and not the user's doing -- see `channelStatusCodes`'s note
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

// Unlinking is only offered for a channel that exists, and only once it is
// off. Shown disabled rather than hidden while it is on, so the control is
// where the person expects it and says what to do first -- hiding it would
// make the page look as though unlinking were unavailable.
const unlinkable = computed(() => props.status.configured);

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
    data-testid="channel-status-row"
    :data-channel-kind="status.kind"
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
          data-testid="channel-status-info"
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
        v-if="unlinkable"
        type="button"
        class="shrink-0 text-[var(--fg-4)] hover:text-error disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:text-[var(--fg-4)]"
        data-testid="unlink-channel"
        aria-label="Unlink channel"
        :title="enabled ? 'Turn the channel off first' : 'Unlink channel'"
        :disabled="busy || enabled"
        @click="emit('unlink')"
      >
        <TrashIcon class="size-3.5" />
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
      data-testid="channel-status-reason"
    >
      {{ reason }}
    </p>
  </li>
</template>
