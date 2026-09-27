<script setup lang="ts">
import { computed } from "vue";
import { InformationCircleIcon, TrashIcon } from "@heroicons/vue/24/outline";
import ChannelIcon from "./ChannelIcon.vue";
import type { ChannelColor, DeviceChannelStatus } from "../../../stores/device";
import { channelName, channelProblemText } from "../../../utils/channelStatusCodes";

// One channel of a paired device (ADR-0008 D19). The backend decides the
// colour; this row only draws it:
//
// | colour       | state | controls        |
// |--------------|-------|-----------------|
// | green        | on    | switch          |
// | grey         | on    | switch          |
// | orange       | on    | switch, ⓘ popup |
// | empty circle | off   | switch, unlink  |
// | (no dot)     | none  | Add             |
const props = defineProps<{
  status: DeviceChannelStatus;
  busy: boolean;
}>();

const emit = defineEmits<{
  toggle: [enabled: boolean];
  add: [];
  unlink: [];
}>();

const name = computed(() => channelName(props.status.kind));
const switchedOn = computed(() => props.status.state === "on");
const problem = computed(() =>
  props.status.problem ? channelProblemText(props.status.problem) : null,
);

const DOT_CLASS: Record<ChannelColor, string> = {
  green: "bg-success",
  grey: "bg-[var(--fg-5)]",
  orange: "bg-warning",
  off: "border border-[var(--fg-4)] bg-transparent",
  none: "",
};

// The template's render contract, per fini-frontend.
const renderFlags = computed(() => ({
  dot: props.status.color !== "none",
  problemInfo: problem.value !== null,
  addButton: props.status.state === "none",
  unlinkButton: props.status.state === "off",
  channelSwitch: props.status.state !== "none",
}));

function handleToggle() {
  emit("toggle", !switchedOn.value);
}
</script>

<template>
  <li
    class="flex items-center gap-2 border-b border-base-200 bg-base-100 px-2 py-1.5 last:border-b-0"
    data-testid="channel-status-row"
    :data-channel-kind="status.kind"
    :data-channel-color="status.color"
  >
    <span
      v-if="renderFlags.dot"
      class="size-2.5 shrink-0 rounded-full"
      :class="DOT_CLASS[status.color]"
      data-testid="channel-dot"
    />
    <ChannelIcon :kind="status.kind" class="size-3.5 shrink-0 opacity-70" />
    <span class="min-w-0 flex-1 truncate text-sm font-semibold">{{ name }}</span>

    <!-- ⓘ opens a popup; nothing explanatory is ever rendered inline. -->
    <div v-if="renderFlags.problemInfo" class="dropdown dropdown-end shrink-0">
      <button
        type="button"
        tabindex="0"
        class="text-warning"
        data-testid="channel-problem-info"
        :aria-label="`About ${name}`"
      >
        <InformationCircleIcon class="size-4" />
      </button>
      <p
        tabindex="0"
        class="dropdown-content z-10 w-60 rounded-box bg-base-200 p-2 text-xs shadow"
        data-testid="channel-problem-popup"
      >
        {{ problem }}
      </p>
    </div>

    <button
      v-if="renderFlags.unlinkButton"
      type="button"
      class="shrink-0 text-[var(--fg-4)] hover:text-error disabled:cursor-not-allowed disabled:opacity-40"
      data-testid="unlink-channel"
      :aria-label="`Unlink ${name}`"
      :disabled="busy"
      @click="emit('unlink')"
    >
      <TrashIcon class="size-3.5" />
    </button>

    <button
      v-if="renderFlags.addButton"
      type="button"
      class="btn btn-ghost btn-xs shrink-0"
      data-testid="add-channel"
      :disabled="busy"
      @click="emit('add')"
    >
      Add
    </button>

    <input
      v-if="renderFlags.channelSwitch"
      type="checkbox"
      class="toggle toggle-sm toggle-success shrink-0"
      data-testid="channel-switch"
      :aria-label="`Turn ${name} ${switchedOn ? 'off' : 'on'}`"
      :checked="switchedOn"
      :disabled="busy"
      @change="handleToggle"
    />
  </li>
</template>
