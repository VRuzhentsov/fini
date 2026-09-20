<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { XMarkIcon, CheckIcon, MagnifyingGlassIcon, ExclamationCircleIcon } from "@heroicons/vue/24/outline";
import ChannelIcon from "./ChannelIcon.vue";
import { useDeviceStore } from "../../../stores/device";

// Setting up a second channel on a device that is *already paired*.
//
// The thing that makes this flow different from pairing is what it does not
// contain: no six-digit code, no acceptance screen, nobody on the other
// device to read anything out. Trust was established once, for the pair,
// not per channel -- so this is a search that reports its result, not a
// ceremony. The other device shows nothing at all while it runs.
const props = defineProps<{
  open: boolean;
  peerDeviceId: string;
  peerName: string;
}>();

const emit = defineEmits<{ close: [] }>();

const deviceStore = useDeviceStore();

type Step = "channel" | "searching" | "found" | "connected" | "notFound" | "failed";

const step = ref<Step>("channel");
const elapsed = ref(0);
const errorText = ref<string | null>(null);
let ticker: ReturnType<typeof setInterval> | null = null;
// Invalidates a scan whose dialog was closed (or restarted) while its
// up-to-60s `invoke` was still in flight -- without it, a stale resolve
// would drag the dialog back to "found" after the user had left.
let generation = 0;

const title = computed(() =>
  step.value === "channel" ? "Set up a channel" : "Connect over Bluetooth",
);

watch(
  () => props.open,
  (open) => {
    if (open) {
      step.value = "channel";
      errorText.value = null;
      elapsed.value = 0;
    } else {
      generation += 1;
      stopTicker();
    }
  },
);

function stopTicker() {
  if (ticker) {
    clearInterval(ticker);
    ticker = null;
  }
}

onUnmounted(() => {
  generation += 1;
  stopTicker();
});

async function searchBluetooth() {
  const mine = ++generation;
  step.value = "searching";
  errorText.value = null;
  elapsed.value = 0;
  stopTicker();
  ticker = setInterval(() => {
    elapsed.value += 1;
  }, 1000);

  try {
    const address = await deviceStore.findBluetoothAddress(props.peerDeviceId);
    if (mine !== generation) return;
    step.value = address ? "found" : "notFound";
  } catch (error) {
    if (mine !== generation) return;
    errorText.value = String(error);
    step.value = "failed";
  } finally {
    if (mine === generation) stopTicker();
  }
}

async function enableBluetooth() {
  const mine = ++generation;
  errorText.value = null;
  try {
    await deviceStore.setBluetoothChannel(props.peerDeviceId, true);
    await deviceStore.probeBluetoothAdapter();
    await deviceStore.refreshChannelStatuses(props.peerDeviceId);
    if (mine !== generation) return;
    step.value = "connected";
  } catch (error) {
    if (mine !== generation) return;
    errorText.value = String(error);
    step.value = "failed";
  }
}
</script>

<template>
  <!-- Same shape as PairDeviceDialog: a teleported overlay rendered only
       while open, not a native <dialog>. See the note there. -->
  <Teleport to="body">
    <div
      v-if="open"
      class="fixed inset-0 z-[1200] flex items-end justify-center p-3 sm:items-center sm:p-4"
      data-testid="channel-setup-dialog"
    >
      <button
        type="button"
        class="absolute inset-0 bg-black/45"
        aria-label="Close channel setup"
        data-testid="channel-setup-backdrop"
        @click="emit('close')"
      ></button>
      <div class="relative w-full max-w-[428px] overflow-hidden rounded-xl bg-base-100 shadow-2xl">
        <div class="flex items-center gap-2 p-3">
        <h3 class="flex-1 text-sm font-semibold">{{ title }}</h3>
        <button class="btn btn-ghost btn-xs px-1" aria-label="Close" @click="emit('close')">
          <XMarkIcon class="size-4" />
        </button>
      </div>

      <div class="flex flex-col gap-3 px-3 pb-3">
        <template v-if="step === 'channel'">
          <!-- Network is listed and disabled rather than hidden: seeing that
               it is already on is what explains why there is only one thing
               to press. -->
          <div class="flex w-full items-start gap-3 rounded-[10px] border border-base-300 p-3 opacity-65">
            <span class="grid size-[34px] shrink-0 place-items-center rounded-[9px] bg-base-200">
              <ChannelIcon kind="network" class="size-5 opacity-70" />
            </span>
            <span class="flex min-w-0 flex-1 flex-col gap-0.5 text-left">
              <b class="text-[13px] font-semibold">Network</b>
              <span class="text-[11.5px] text-[var(--fg-3)]">Same network</span>
            </span>
            <span class="shrink-0 pt-0.5 text-[9.5px] font-bold uppercase tracking-wider text-success">On</span>
          </div>

          <button
            class="flex w-full items-start gap-3 rounded-[10px] border border-base-300 p-3 text-left hover:border-primary hover:bg-base-200"
            data-testid="setup-bluetooth"
            @click="void searchBluetooth()"
          >
            <span class="grid size-[34px] shrink-0 place-items-center rounded-[9px] bg-base-200">
              <ChannelIcon kind="bluetooth" class="size-5 opacity-70" />
            </span>
            <span class="flex min-w-0 flex-1 flex-col gap-0.5">
              <b class="text-[13px] font-semibold">Bluetooth</b>
              <span class="text-[11.5px] text-[var(--fg-3)]">Nearby, no network</span>
            </span>
          </button>
        </template>

        <template v-else-if="step === 'searching'">
          <div class="flex flex-col items-center gap-3 py-2 text-center">
            <span class="grid size-16 place-items-center rounded-full bg-base-200">
              <ChannelIcon kind="bluetooth" class="size-6 opacity-70" />
            </span>
            <h4 class="text-[15px] font-semibold">Looking for {{ peerName }}</h4>
            <span class="font-mono text-[11px] text-[var(--fg-4)]">searching · {{ elapsed }}s</span>
          </div>
          <p class="text-center text-[11.5px] text-[var(--fg-3)]">
            {{ peerName }} doesn't need to do anything — you're already paired.
          </p>
        </template>

        <template v-else-if="step === 'found'">
          <div class="flex flex-col items-center gap-3 py-2 text-center">
            <span class="grid size-13 place-items-center rounded-full bg-success p-3 text-white">
              <CheckIcon class="size-6" />
            </span>
            <h4 class="text-[15px] font-semibold">Found {{ peerName }}</h4>
          </div>
        </template>

        <template v-else-if="step === 'connected'">
          <div class="flex flex-col items-center gap-3 py-2 text-center">
            <span class="grid size-13 place-items-center rounded-full bg-success p-3 text-white">
              <CheckIcon class="size-6" />
            </span>
            <h4 class="text-[15px] font-semibold">Bluetooth is on</h4>
          </div>
        </template>

        <template v-else-if="step === 'notFound'">
          <div class="flex flex-col items-center gap-3 py-2 text-center">
            <span class="grid size-13 place-items-center rounded-full bg-base-200 p-3 text-error">
              <MagnifyingGlassIcon class="size-6" />
            </span>
            <h4 class="text-[15px] font-semibold">Couldn't find {{ peerName }}</h4>
          </div>
          <!-- Physical causes, in the order they actually happen. Never
               "no devices found" on its own. -->
          <ul class="flex list-none flex-col gap-1.5 text-[12px] leading-snug text-[var(--fg-2)]">
            <li>Is {{ peerName }} awake, and is its Bluetooth on?</li>
            <li>Bluetooth reaches a few metres, through one wall at best.</li>
            <li>Some phones stop answering when the screen has been off a while.</li>
          </ul>
        </template>

        <template v-else>
          <div class="flex flex-col items-center gap-3 py-2 text-center">
            <span class="grid size-13 place-items-center rounded-full bg-base-200 p-3 text-error">
              <ExclamationCircleIcon class="size-6" />
            </span>
            <h4 class="text-[15px] font-semibold">Found it, but couldn't connect</h4>
          </div>
          <p v-if="errorText" class="text-center text-[11.5px] text-[var(--fg-3)]">{{ errorText }}</p>
        </template>
      </div>

      <div class="flex items-center gap-2 border-t border-base-200 p-3">
        <template v-if="step === 'searching'">
          <button class="btn btn-ghost btn-sm" @click="emit('close')">Cancel</button>
        </template>
        <template v-else-if="step === 'found'">
          <button class="btn btn-ghost btn-sm" @click="emit('close')">Cancel</button>
          <span class="flex-1" />
          <button class="btn btn-primary btn-sm" data-testid="turn-on-bluetooth" @click="void enableBluetooth()">
            Turn on Bluetooth
          </button>
        </template>
        <template v-else-if="step === 'connected'">
          <span class="flex-1" />
          <button class="btn btn-primary btn-sm" @click="emit('close')">Done</button>
        </template>
          <template v-else-if="step === 'notFound' || step === 'failed'">
            <button class="btn btn-ghost btn-sm" @click="emit('close')">Close</button>
            <span class="flex-1" />
            <button class="btn btn-primary btn-sm" @click="void searchBluetooth()">Search again</button>
          </template>
        </div>
      </div>
    </div>
  </Teleport>
</template>
