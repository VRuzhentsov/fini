<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import {
  XMarkIcon,
  CheckIcon,
  ClockIcon,
  MagnifyingGlassIcon,
  ExclamationCircleIcon,
  InformationCircleIcon,
} from "@heroicons/vue/24/outline";
import ChannelIcon from "./device/ChannelIcon.vue";
import {
  useDeviceStore,
  type ChannelSetupStatus,
  type DiscoveredDevice,
} from "../../stores/device";
import { ChannelKind } from "../../utils/channel";
import { channelName } from "../../utils/channelStatusCodes";

// The one setup dialog (ADR-0008 D20), for every way two devices come to
// share a channel:
//
// - a new device (never paired, or paired again after unpairing): choose a
//   channel, pick the device, confirm with the code ceremony;
// - a known device getting a channel (added, or re-added after unlink):
//   both devices search and say hello to each other; once both hellos are
//   acknowledged the channel's init is complete and OK switches it on.
//
// Explanations live behind ⓘ, never as text in the dialog, and there is no
// "nobody found" dead end: the search simply keeps going while it is open.
//
// A modal rather than a page, because it is a short two-person ceremony
// with a beginning and an end: one person acts while the other waits, and
// both screens have to keep saying whose turn it is. The old inline page
// could not do that -- it looked like a form, and a form does not explain
// why nothing is happening.
//
// The first question is which channel, asked out loud. The previous screen
// inferred it from whichever radio happened to find something and captioned
// the result "via network" / "via Bluetooth" after the fact.
const props = defineProps<{
  open: boolean;
  // Which incoming request to show, when the caller opened the dialog from
  // a specific row. Without it the dialog always showed the newest, so with
  // two devices asking at once, "Open request" on the older row answered
  // the wrong person -- and the older one could not be accepted at all.
  requestId?: string | null;
  // Set for a known device: the channel being set up with it.
  peerDeviceId?: string | null;
  peerName?: string;
  kind?: ChannelKind | null;
}>();
const emit = defineEmits<{ close: [] }>();

const deviceStore = useDeviceStore();

const channel = ref<ChannelKind | null>(null);
const codeInput = ref("");
const acceptedRequestId = ref<string | null>(null);
const codeError = ref<string | null>(null);
const nowMs = ref(Date.now());
let clockTimer: ReturnType<typeof setInterval> | null = null;

const outgoing = computed(() => deviceStore.outgoingRequest);

// Someone asking to pair with *this* device takes priority over whatever
// the user was doing: they are waiting on an answer, and a request that
// expires unseen is the worst outcome of the whole flow.
const incoming = computed(() => {
  const requests = deviceStore.incomingRequests;
  if (props.requestId) {
    const chosen = requests.find((request) => request.request_id === props.requestId);
    // Falls back rather than showing nothing: the chosen request can expire
    // while the dialog is open, and an empty dialog would say less than the
    // next person waiting.
    if (chosen) return chosen;
  }
  return requests[0] ?? null;
});

// Read from the per-channel lists, not from `discoveredDevices`.
//
// That array is deduplicated with Network preferred, so a peer visible over
// both channels appears in it only as a Network entry -- and filtering it by
// the chosen channel would make that peer vanish the moment the user picks
// Bluetooth. Two devices on one LAN is the common case, so the dialog would
// have reported "nobody found" while the BLE scan was looking straight at
// the peer.
const candidates = computed<DiscoveredDevice[]>(() =>
  channel.value ? deviceStore.discoveredByChannel[channel.value] : [],
);

const knownDevice = computed(() => Boolean(props.peerDeviceId && props.kind));

// The known-device list (ADR-0008 D20 steps 2-3): the known peer is marked
// and preselected at the top, and every other device in range can still be
// picked. Picking one -- or the known peer showing up as a device to pair
// with, because automatic confirmation cannot work with it -- goes through
// the code ceremony.
const renderLists = computed(() => ({
  otherCandidates: knownDevice.value
    ? candidates.value.filter((device) => device.device_id !== props.peerDeviceId)
    : candidates.value,
  // Read from the list that keeps paired devices: the known peer is paired,
  // so the ordinary candidate list never contains it.
  knownPeerAsCandidate:
    knownDevice.value && props.kind
      ? deviceStore.discoveredWithPairedByChannel[props.kind].filter(
          (device) => device.device_id === props.peerDeviceId,
        )
      : [],
}));

// The known-device branch's own progress, polled while the dialog is open.
const setupStatus = ref<ChannelSetupStatus | null>(null);
const setupError = ref<string | null>(null);
let setupPoll: ReturnType<typeof setInterval> | null = null;
const SETUP_POLL_MS = 1_000;
// Set once OK or close ended the setup, so unmounting does not end it twice.
let setupFinished = false;

const step = computed(() => {
  // A code ceremony under way -- the known peer's fallback, or another
  // device picked from the list -- keeps the screen until it ends, even if
  // the automatic path completes meanwhile: OK would otherwise set up the
  // original peer and abandon the ceremony the person is in.
  if (deviceStore.pairCompletedAt) return "paired";
  if (incoming.value) {
    return acceptedRequestId.value === incoming.value.request_id ? "entercode" : "incoming";
  }

  const request = outgoing.value;
  if (request) {
    // A decline is a legitimate answer, not an error -- and worth saying
    // plainly, because the person who asked is otherwise left guessing what
    // the other one saw.
    if (request.status === "rejected") return "declined";
    // Not a decline: this side never got the request out. Saying "they said
    // no" here would be a claim about someone who never saw it.
    if (request.status === "send_failed") return "sendFailed";
    // Covers both "nobody answered" and "the peer vanished mid-ceremony":
    // the request expiring is the only signal this side genuinely has for
    // either, so claiming to tell them apart would be invention. What
    // changes is whether a code was already issued, which the copy reads
    // off `sender_code` rather than a separate step.
    if (request.status === "expired") return "timeout";
    if (request.sender_code) return "code";
    if (request.status === "pending" || request.status === "awaiting_code") return "sent";
  }

  // Known device, automatic path (ADR-0008 D20 step 4): the hello exchange.
  if (knownDevice.value) {
    if (setupError.value) return "setupFailed";
    if (setupStatus.value?.initialized) return "setupReady";
    if (setupStatus.value?.helloAckedByPeer || setupStatus.value?.ackedPeerHello) return "setupFound";
    return "setupSearching";
  }
  if (!channel.value) return "channel";
  return "looking";
});

// The first step's choices, and what each one is for.
const CHANNEL_CHOICES = Object.values(ChannelKind);

function channelChoiceHint(kind: ChannelKind): string {
  return kind === ChannelKind.Network ? "Same network" : "Nearby, no network";
}

const title = computed(() =>
  knownDevice.value && props.kind ? `Add ${channelName(props.kind)}` : "Add device",
);

// What ⓘ in the header explains for the step on screen, or null when there
// is nothing to add. The dialog itself carries no explanatory text.
const hint = computed<string | null>(() => {
  switch (step.value) {
    case "setupSearching":
    case "setupFound":
      return `Open Add ${props.kind ? channelName(props.kind) : ""} on ${props.peerName ?? "the other device"} too — OK becomes available once both devices have found each other.`;
    case "looking":
      return "Open Add device on the other device too — it appears here once it is also looking.";
    case "declined":
      return "Nothing was shared, and no code was created.";
    case "sendFailed":
      return "The request never got out. The other device may be asleep or out of range.";
    case "timeout":
      return outgoing.value?.sender_code
        ? "That code no longer works. The other device may be asleep, locked, or out of range."
        : "The other device may be asleep, locked, or out of range.";
    default:
      return null;
  }
});

// The known-device steps' render contract, per fini-frontend.
const renderFlags = computed(() => ({
  hintInfo: hint.value !== null,
  setupSearch: step.value === "setupSearching" || step.value === "setupFound",
  setupReady: step.value === "setupReady",
  setupFailed: step.value === "setupFailed",
  setupFooter: ["setupSearching", "setupFound", "setupReady", "setupFailed"].includes(step.value),
  otherCandidates: renderLists.value.otherCandidates.length > 0,
}));

function secondsLeft(iso: string): number {
  const diff = Date.parse(iso) - nowMs.value;
  return diff > 0 ? Math.ceil(diff / 1000) : 0;
}

const digits = computed(() => codeInput.value.replace(/\D/g, "").slice(0, 6));

// Entering add mode is what makes this device discoverable to the other
// one, so it has to happen exactly once per opening -- and it must survive
// being mounted already-open. The parent can flip its flag during its own
// setup, before this component exists, and a plain `watch` on the prop
// would never see that transition: the nearby list would sit empty forever.
// Adding a known device's Network channel has no use for the Bluetooth
// half of add mode.
const addModeUsesBluetooth = computed(
  () => !(knownDevice.value && props.kind === ChannelKind.Network),
);

function startAddMode() {
  channel.value = null;
  codeInput.value = "";
  codeError.value = null;
  acceptedRequestId.value = null;
  void deviceStore.enterAddMode({ bluetooth: addModeUsesBluetooth.value });
  stopClock();
  clockTimer = setInterval(() => {
    nowMs.value = Date.now();
  }, 1000);
}

function stopAddMode() {
  stopClock();
  void deviceStore.leaveAddMode();
}

// A known device needs no add mode: both devices run a setup search for
// each other instead (ADR-0008 D2), and this polls how far it got.
async function startChannelSetup() {
  setupFinished = false;
  setupStatus.value = null;
  setupError.value = null;
  if (!props.peerDeviceId || !props.kind) return;
  const peerDeviceId = props.peerDeviceId;
  const kind = props.kind;
  try {
    await deviceStore.beginChannelSetup(peerDeviceId, kind);
  } catch (error) {
    if (!setupFinished) setupError.value = String(error);
    return;
  }
  // Closed while the begin was still waiting on the radio or a permission
  // prompt: the end already ran, before the backend search existed, so end
  // the search that has just started instead of leaving it running.
  if (setupFinished) {
    await deviceStore.endChannelSetup(peerDeviceId, kind, false);
    return;
  }
  stopSetupPoll();
  setupPoll = setInterval(() => void pollChannelSetup(), SETUP_POLL_MS);
}

async function pollChannelSetup() {
  if (!props.peerDeviceId || !props.kind) return;
  setupStatus.value = await deviceStore.channelSetupStatus(props.peerDeviceId, props.kind);
}

function stopSetupPoll() {
  if (setupPoll) {
    clearInterval(setupPoll);
    setupPoll = null;
  }
}

// OK switches the channel on; closing leaves it Off if the init completed
// and writes nothing otherwise (ADR-0008 D15) -- the backend decides.
// `false` when writing the channel failed: the backend keeps the completed
// init, so OK can be pressed again.
async function finishChannelSetup(switchOn: boolean): Promise<boolean> {
  stopSetupPoll();
  if (setupFinished) return true;
  setupFinished = true;
  if (!props.peerDeviceId || !props.kind || setupError.value) return true;
  try {
    await deviceStore.endChannelSetup(props.peerDeviceId, props.kind, switchOn);
    return true;
  } catch (error) {
    console.warn("[device-setup] finishing the channel setup failed", error);
    setupFinished = false;
    return false;
  }
}

// A known device runs both: the setup search for the automatic path, and
// add mode so the list and the code fallback work (ADR-0008 D20).
function begin() {
  startAddMode();
  if (knownDevice.value) {
    channel.value = props.kind ?? null;
    void startChannelSetup();
  }
}

function end() {
  if (knownDevice.value) void finishChannelSetup(false);
  stopAddMode();
}

watch(
  () => props.open,
  (open) => {
    if (open) begin();
    else end();
  },
);

// Deliberately not `{ immediate: true }` on the watcher above: that would
// call `leaveAddMode` on every mount that starts closed, which is the
// common case, tearing down discovery another surface may be using.
onMounted(() => {
  if (props.open) begin();
});

function stopClock() {
  if (clockTimer) {
    clearInterval(clockTimer);
    clockTimer = null;
  }
}

// Unmounting while open (navigating away mid-flow) has to release add mode
// too, or this device keeps advertising itself as looking for a pair with
// no screen left to show it.
onUnmounted(() => {
  if (props.open) end();
  else stopClock();
  stopSetupPoll();
});

function pickChannel(kind: ChannelKind) {
  channel.value = kind;
}

// Back to the list. A known device keeps its channel: the row's Add set it.
function startOver() {
  deviceStore.cancelOutgoingRequest();
  channel.value = knownDevice.value ? (props.kind ?? null) : null;
}

async function requestPair(device: DiscoveredDevice) {
  await deviceStore.requestPair(device);
}

async function acceptIncoming(requestId: string) {
  const accepted = await deviceStore.acceptIncomingRequest(requestId);
  if (accepted) acceptedRequestId.value = requestId;
}

function declineIncoming(requestId: string) {
  acceptedRequestId.value = null;
  void deviceStore.rejectIncomingRequest(requestId);
}

async function submitCode(requestId: string) {
  codeError.value = null;
  if (digits.value.length !== 6) return;
  const paired = await deviceStore.submitPairCode(requestId, digits.value);
  if (paired) {
    codeInput.value = "";
    acceptedRequestId.value = null;
  } else {
    codeError.value = "That code doesn't match";
    codeInput.value = "";
  }
}

function close() {
  deviceStore.cancelOutgoingRequest();
  emit("close");
}

async function confirmChannelSetup() {
  if (await finishChannelSetup(true)) emit("close");
}
</script>

<template>
  <!-- A teleported overlay rendered only while open, matching
       ExportSpacesDialog/MergeConflictDialog. Not a native <dialog>: bound
       `open` leaves DaisyUI's `.modal` at `visibility: hidden`, so the node
       exists but nothing can see or click it -- including the e2e lane. -->
  <Teleport to="body">
    <div
      v-if="open"
      class="fixed inset-0 z-[1200] flex items-end justify-center p-3 sm:items-center sm:p-4"
      data-testid="pair-device-dialog"
    >
      <button
        type="button"
        class="absolute inset-0 bg-black/45"
        aria-label="Close pairing dialog"
        data-testid="pair-dialog-backdrop"
        @click="close()"
      ></button>
      <div class="relative w-full max-w-[428px] overflow-hidden rounded-xl bg-base-100 shadow-2xl">
        <div class="flex items-center gap-2 p-3">
        <h3 class="flex-1 text-sm font-semibold">{{ title }}</h3>
        <div v-if="renderFlags.hintInfo" class="dropdown dropdown-end">
          <button
            type="button"
            tabindex="0"
            class="btn btn-ghost btn-xs px-1"
            data-testid="setup-hint-info"
            aria-label="About this step"
          >
            <InformationCircleIcon class="size-4" />
          </button>
          <p
            tabindex="0"
            class="dropdown-content z-10 w-64 rounded-box bg-base-200 p-2 text-xs shadow"
            data-testid="setup-hint-popup"
          >
            {{ hint }}
          </p>
        </div>
        <button class="btn btn-ghost btn-xs px-1" aria-label="Close" @click="close()">
          <XMarkIcon class="size-4" />
        </button>
      </div>

      <div class="flex flex-col gap-3 px-3 pb-3">
        <!-- Known device: both devices search for each other (ADR-0008 D1). -->
        <template v-if="renderFlags.setupSearch || renderFlags.setupReady">
          <div
            class="flex items-center gap-2.5 rounded-lg bg-base-200 px-2 py-2"
            data-testid="setup-peer-row"
            :data-setup-step="step"
          >
            <!-- Searching pulses; found (one direction done) turns amber; ready
                 is green. The search takes tens of seconds on a real radio, and
                 a still grey dot reads as a frozen screen. -->
            <span
              class="size-2.5 shrink-0 rounded-full"
              :class="
                renderFlags.setupReady
                  ? 'bg-success'
                  : step === 'setupFound'
                    ? 'animate-pulse bg-warning'
                    : 'animate-pulse bg-[var(--fg-5)]'
              "
            />
            <span class="min-w-0 flex-1 truncate text-sm font-medium">{{ peerName }}</span>
            <span v-if="renderFlags.setupSearch" class="loading loading-dots loading-xs opacity-60" />
            <CheckIcon v-if="renderFlags.setupReady" class="size-4 text-success" />
            <button
              v-for="device in renderLists.knownPeerAsCandidate"
              :key="device.device_id"
              class="btn btn-ghost btn-xs"
              data-testid="setup-pair-with-code"
              @click="void requestPair(device)"
            >Pair with code</button>
          </div>
          <ul v-if="renderFlags.otherCandidates" class="flex list-none flex-col overflow-hidden rounded-lg">
            <li
              v-for="device in renderLists.otherCandidates"
              :key="device.device_id"
              class="flex items-center gap-2.5 border-b border-base-200 bg-base-100 px-2 py-2 last:border-b-0"
              data-testid="nearby-device-row"
              :data-device-id="device.device_id"
            >
              <span class="size-2.5 shrink-0 rounded-full bg-success" />
              <span class="min-w-0 flex-1 truncate text-sm font-medium">{{ device.hostname }}</span>
              <button
                class="btn btn-primary btn-xs"
                data-testid="request-pair"
                @click="void requestPair(device)"
              >Pair</button>
            </li>
          </ul>
        </template>

        <template v-else-if="renderFlags.setupFailed">
          <div class="flex flex-col items-center gap-3 py-2 text-center" data-testid="setup-failed">
            <span class="grid size-13 place-items-center rounded-full bg-base-200 p-3 text-error">
              <ExclamationCircleIcon class="size-6" />
            </span>
            <h4 class="text-[15px] font-semibold">{{ setupError }}</h4>
          </div>
        </template>

        <!-- 1. The channel, chosen rather than inferred. -->
        <template v-else-if="step === 'channel'">
          <button
            v-for="kind in CHANNEL_CHOICES"
            :key="kind"
            class="flex w-full items-start gap-3 rounded-[10px] border border-base-300 p-3 text-left hover:border-primary hover:bg-base-200"
            :data-testid="`pair-channel-${kind}`"
            @click="pickChannel(kind)"
          >
            <span class="grid size-[34px] shrink-0 place-items-center rounded-[9px] bg-base-200">
              <ChannelIcon :kind="kind" class="size-5 opacity-70" />
            </span>
            <span class="flex min-w-0 flex-1 flex-col gap-0.5">
              <b class="text-[13px] font-semibold">{{ channelName(kind) }}</b>
              <span class="text-[11.5px] text-[var(--fg-3)]">
                {{ channelChoiceHint(kind) }}
              </span>
            </span>
          </button>
        </template>

        <!-- 2. Looking. An empty list is the consent rule made visible:
             there is nobody to pair with who has not also asked. -->
        <template v-else-if="step === 'looking'">
          <div v-if="candidates.length === 0" class="flex flex-col items-center gap-3 py-2 text-center">
            <span class="grid size-16 place-items-center rounded-full bg-base-200">
              <ChannelIcon :kind="channel!" class="size-6 opacity-70" />
            </span>
            <h4 class="text-[15px] font-semibold">
              Looking for devices on {{ channelName(channel!) }}
            </h4>
          </div>

          <ul v-else class="flex list-none flex-col overflow-hidden rounded-lg">
            <li
              v-for="device in candidates"
              :key="device.device_id"
              class="flex items-center gap-2.5 border-b border-base-200 bg-base-100 px-2 py-2 last:border-b-0"
              data-testid="nearby-device-row"
              :data-device-hostname="device.hostname"
              :data-device-id="device.device_id"
              :data-channel-kind="device.channel_kind"
            >
              <span class="size-2.5 shrink-0 rounded-full bg-success" />
              <span class="min-w-0 flex-1 truncate text-sm font-medium">{{ device.hostname }}</span>
              <button
                class="btn btn-primary btn-xs"
                data-testid="request-pair"
                @click="void requestPair(device)"
              >Pair</button>
            </li>
          </ul>
        </template>

        <!-- 2b. Declined. Nothing was shared and no code was ever generated
             -- worth saying, or the asker is left guessing. -->
        <template v-else-if="step === 'declined'">
          <div class="flex flex-col items-center gap-3 py-2 text-center">
            <span class="grid size-13 place-items-center rounded-full bg-base-200 p-3 text-error">
              <XMarkIcon class="size-6" />
            </span>
            <h4 class="text-[15px] font-semibold">{{ outgoing?.to_hostname }} said no</h4>
          </div>
        </template>

        <!-- 2b-ii. The request never left this device. Distinct from a
             decline: naming the peer as having refused would be a claim
             about a device that never received the question. -->
        <template v-else-if="step === 'sendFailed'">
          <div class="flex flex-col items-center gap-3 py-2 text-center">
            <span class="grid size-13 place-items-center rounded-full bg-base-200 p-3 text-error">
              <ExclamationCircleIcon class="size-6" />
            </span>
            <h4 class="text-[15px] font-semibold">Couldn't reach {{ outgoing?.to_hostname }}</h4>
          </div>
        </template>

        <!-- 2c. Ran out. The causes are physical -- asleep, locked, walked
             off -- so name those rather than "request timed out". When a
             code had already been issued, say it is dead: otherwise someone
             keeps typing it into a device that no longer accepts it. -->
        <template v-else-if="step === 'timeout'">
          <div class="flex flex-col items-center gap-3 py-2 text-center">
            <span class="grid size-13 place-items-center rounded-full bg-base-200 p-3 text-error">
              <ClockIcon class="size-6" />
            </span>
            <h4 class="text-[15px] font-semibold">The request ran out</h4>
          </div>
        </template>

        <!-- 3. Asked, and only able to wait. Saying so is the point. -->
        <template v-else-if="step === 'sent'">
          <div class="flex flex-col items-center gap-3 py-2 text-center">
            <span class="loading loading-spinner loading-md text-primary" />
            <h4 class="text-[15px] font-semibold">Waiting for {{ outgoing?.to_hostname }}</h4>
            <span class="font-mono text-[11px] text-[var(--fg-4)]">
              expires in {{ outgoing ? secondsLeft(outgoing.expires_at) : 0 }}s
            </span>
          </div>
        </template>

        <!-- 4. Only now does a code exist: before acceptance there was
             nothing to intercept and nothing to shoulder-surf. -->
        <template v-else-if="step === 'code'">
          <h4 class="text-[15px] font-semibold">Read this to {{ outgoing?.to_hostname }}</h4>
          <div class="flex justify-center gap-[7px] py-1">
            <b
              v-for="(character, index) in (outgoing?.sender_code ?? '').split('')"
              :key="index"
              class="grid h-[54px] w-[42px] place-items-center rounded-[9px] bg-base-200 font-mono text-2xl font-medium"
              data-testid="pair-code"
            >{{ character }}</b>
          </div>
          <p class="text-center text-[11.5px] text-[var(--fg-3)]">
            Waiting for {{ outgoing?.to_hostname }} to type it
          </p>
        </template>

        <!-- 5. The other side. Accepting costs nothing to refuse: it only
             lets the other device ask for a code. -->
        <template v-else-if="step === 'incoming'">
          <h4 class="text-[15px] font-semibold">{{ incoming?.from_hostname }} wants to pair</h4>
          <p class="font-mono text-[11px] text-[var(--fg-4)]">
            expires in {{ incoming ? secondsLeft(incoming.expires_at) : 0 }}s
          </p>
        </template>

        <template v-else-if="step === 'entercode'">
          <h4 class="text-[15px] font-semibold">Type the code from {{ incoming?.from_hostname }}</h4>
          <input
            v-model="codeInput"
            maxlength="6"
            type="tel"
            inputmode="numeric"
            pattern="[0-9]*"
            class="input input-bordered w-full text-center font-mono text-2xl tracking-[0.4em]"
            data-testid="pair-code-input"
            placeholder="······"
          />
          <p v-if="codeError" class="text-center text-[12px] text-error">{{ codeError }}</p>
        </template>

        <template v-else>
          <div class="flex flex-col items-center gap-3 py-2 text-center">
            <span class="grid size-13 place-items-center rounded-full bg-success p-3 text-white">
              <CheckIcon class="size-6" />
            </span>
            <h4 class="text-[15px] font-semibold">Paired</h4>
          </div>
        </template>
      </div>

      <div class="flex items-center gap-2 border-t border-base-200 p-3">
        <template v-if="renderFlags.setupFooter">
          <button class="btn btn-ghost btn-sm" data-testid="setup-close" @click="close()">Close</button>
          <span class="flex-1" />
          <button
            class="btn btn-primary btn-sm"
            data-testid="setup-ok"
            :disabled="!renderFlags.setupReady"
            @click="void confirmChannelSetup()"
          >OK</button>
        </template>
        <template v-else-if="step === 'looking'">
          <button class="btn btn-ghost btn-sm" @click="channel = null">Back</button>
          <span class="flex-1" />
          <span class="inline-flex items-center gap-1.5 font-mono text-[11px] text-[var(--fg-4)]">
            <MagnifyingGlassIcon class="size-3.5" />
            looking
          </span>
        </template>
        <template v-else-if="step === 'declined' || step === 'timeout' || step === 'sendFailed'">
          <button class="btn btn-ghost btn-sm" @click="emit('close')">Close</button>
          <span class="flex-1" />
          <button class="btn btn-primary btn-sm" data-testid="pair-ask-again" @click="startOver()">
            Ask again
          </button>
        </template>
        <template v-else-if="step === 'sent' || step === 'code'">
          <button class="btn btn-ghost btn-sm" @click="deviceStore.cancelOutgoingRequest()">
            Cancel request
          </button>
        </template>
        <template v-else-if="step === 'incoming'">
          <button class="btn btn-ghost btn-sm" @click="declineIncoming(incoming!.request_id)">Decline</button>
          <span class="flex-1" />
          <button
            class="btn btn-primary btn-sm"
            data-testid="accept-incoming-request"
            @click="void acceptIncoming(incoming!.request_id)"
          >Accept</button>
        </template>
        <template v-else-if="step === 'entercode'">
          <button class="btn btn-ghost btn-sm" @click="acceptedRequestId = null">Cancel</button>
          <span class="flex-1" />
          <button
            class="btn btn-primary btn-sm"
            data-testid="pair-code-submit"
            :disabled="digits.length < 6"
            @click="void submitCode(incoming!.request_id)"
          >Pair</button>
        </template>
          <template v-else-if="step === 'paired'">
            <span class="flex-1" />
            <button class="btn btn-primary btn-sm" @click="emit('close')">Done</button>
          </template>
        </div>
      </div>
    </div>
  </Teleport>
</template>
