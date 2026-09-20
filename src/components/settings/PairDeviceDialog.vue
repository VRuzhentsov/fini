<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { XMarkIcon, CheckIcon, ClockIcon, MagnifyingGlassIcon } from "@heroicons/vue/24/outline";
import ChannelIcon from "./device/ChannelIcon.vue";
import { useDeviceStore, type DiscoveredDevice } from "../../stores/device";

// Pairing a device that is not paired yet.
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
const props = defineProps<{ open: boolean }>();
const emit = defineEmits<{ close: [] }>();

const deviceStore = useDeviceStore();

type Channel = "network" | "bluetooth";

const channel = ref<Channel | null>(null);
const codeInput = ref("");
const acceptedRequestId = ref<string | null>(null);
const codeError = ref<string | null>(null);
const nowMs = ref(Date.now());
let clockTimer: ReturnType<typeof setInterval> | null = null;

const outgoing = computed(() => deviceStore.outgoingRequest);

// Someone asking to pair with *this* device takes priority over whatever
// the user was doing: they are waiting on an answer, and a request that
// expires unseen is the worst outcome of the whole flow.
const incoming = computed(() => deviceStore.incomingRequests[0] ?? null);

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

// How long an empty list is ordinary before it is worth explaining.
// Bluetooth gets far longer than Network on purpose: scanning is
// duty-cycled, so tens of seconds of nothing is the expected shape of a
// working scan rather than a symptom (ADR-0006).
const EMPTY_LIST_PATIENCE_MS = { network: 20_000, bluetooth: 45_000 } as const;

// When the current channel's search started, so "nobody found" is reported
// after a wait rather than in the first second of one.
const lookingSince = ref(0);

const step = computed(() => {
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
    // Covers both "nobody answered" and "the peer vanished mid-ceremony":
    // the request expiring is the only signal this side genuinely has for
    // either, so claiming to tell them apart would be invention. What
    // changes is whether a code was already issued, which the copy reads
    // off `sender_code` rather than a separate step.
    if (request.status === "expired") return "timeout";
    if (request.sender_code) return "code";
    if (request.status === "pending" || request.status === "awaiting_code") return "sent";
  }

  if (!channel.value) return "channel";
  if (
    candidates.value.length === 0 &&
    lookingSince.value > 0 &&
    nowMs.value - lookingSince.value > EMPTY_LIST_PATIENCE_MS[channel.value]
  ) {
    return "none";
  }
  return "looking";
});

const CHANNEL_NAME: Record<Channel, string> = { network: "Network", bluetooth: "Bluetooth" };

const title = computed(() => (step.value === "channel" ? "Add device" : "Add device"));

function secondsLeft(iso: string): number {
  const diff = Date.parse(iso) - nowMs.value;
  return diff > 0 ? Math.ceil(diff / 1000) : 0;
}

const digits = computed(() => codeInput.value.replace(/\D/g, "").slice(0, 6));

// Entering add mode is what makes this device discoverable to the other
// one, so it has to happen exactly once per opening -- and it must survive
// being mounted already-open, which is what arriving straight at
// `/settings/add-device` does: the parent flips its flag during its own
// setup, before this component exists, so a plain `watch` on the prop would
// never see the transition and the nearby list would sit empty forever.
function startAddMode() {
  channel.value = null;
  codeInput.value = "";
  codeError.value = null;
  acceptedRequestId.value = null;
  void deviceStore.enterAddMode();
  stopClock();
  clockTimer = setInterval(() => {
    nowMs.value = Date.now();
  }, 1000);
}

function stopAddMode() {
  stopClock();
  void deviceStore.leaveAddMode();
}

watch(
  () => props.open,
  (open) => {
    if (open) startAddMode();
    else stopAddMode();
  },
);

// Deliberately not `{ immediate: true }` on the watcher above: that would
// call `leaveAddMode` on every mount that starts closed, which is the
// common case, tearing down discovery another surface may be using.
onMounted(() => {
  if (props.open) startAddMode();
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
  if (props.open) stopAddMode();
  else stopClock();
});

function pickChannel(kind: Channel) {
  channel.value = kind;
  lookingSince.value = Date.now();
}

// "Try Bluetooth instead" / "Try Network instead" from the nobody-found
// screen: a dead end on one channel is a channel choice, not a failure, so
// the way out is the other one rather than a retry of the same nothing.
function switchChannel() {
  pickChannel(channel.value === "network" ? "bluetooth" : "network");
}

function keepLooking() {
  lookingSince.value = Date.now();
}

function startOver() {
  deviceStore.cancelOutgoingRequest();
  channel.value = null;
  lookingSince.value = 0;
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
        <button class="btn btn-ghost btn-xs px-1" aria-label="Close" @click="close()">
          <XMarkIcon class="size-4" />
        </button>
      </div>

      <div class="flex flex-col gap-3 px-3 pb-3">
        <!-- 1. The channel, chosen rather than inferred. -->
        <template v-if="step === 'channel'">
          <button
            v-for="kind in (['network', 'bluetooth'] as Channel[])"
            :key="kind"
            class="flex w-full items-start gap-3 rounded-[10px] border border-base-300 p-3 text-left hover:border-primary hover:bg-base-200"
            :data-testid="`pair-channel-${kind}`"
            @click="pickChannel(kind)"
          >
            <span class="grid size-[34px] shrink-0 place-items-center rounded-[9px] bg-base-200">
              <ChannelIcon :kind="kind" class="size-5 opacity-70" />
            </span>
            <span class="flex min-w-0 flex-1 flex-col gap-0.5">
              <b class="text-[13px] font-semibold">{{ CHANNEL_NAME[kind] }}</b>
              <span class="text-[11.5px] text-[var(--fg-3)]">
                {{ kind === "network" ? "Same network" : "Nearby, no network" }}
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
              Looking for devices on {{ CHANNEL_NAME[channel!] }}
            </h4>
            <p class="text-[12px] leading-snug text-[var(--fg-2)]">
              Open <b>Add device</b> on the other device too — it only appears here once it is also asking.
            </p>
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

        <!-- 2a. Nobody found. Network fails quietly for reasons the person
             can act on, so name them in the order they are likely rather
             than reporting "no devices found" and stopping. -->
        <template v-else-if="step === 'none'">
          <div class="flex flex-col items-center gap-3 py-2 text-center">
            <span class="grid size-13 place-items-center rounded-full bg-base-200 p-3 text-error">
              <MagnifyingGlassIcon class="size-6" />
            </span>
            <h4 class="text-[15px] font-semibold">Nobody found</h4>
          </div>
          <ul class="flex list-none flex-col gap-1.5 text-[12px] leading-snug text-[var(--fg-2)]">
            <li>Is <b>Add device</b> open on the other device?</li>
            <li v-if="channel === 'network'">Are both devices on the same network?</li>
            <li v-else>Is Bluetooth on there, and the device within a few metres?</li>
            <li v-if="channel === 'network'">
              Guest and work networks often stop devices from seeing each other.
            </li>
            <li v-else>Some phones stop answering when the screen has been off a while.</li>
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
            <p class="text-[12.5px] text-[var(--fg-2)]">Nothing was shared, and no code was created.</p>
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
            <p class="text-[12.5px] text-[var(--fg-2)]">
              <template v-if="outgoing?.sender_code">
                {{ outgoing.to_hostname }} may be asleep or have walked off. That code no longer works.
              </template>
              <template v-else>
                {{ outgoing?.to_hostname }} may be asleep, locked, or out of range.
              </template>
            </p>
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
        <template v-if="step === 'looking'">
          <button class="btn btn-ghost btn-sm" @click="channel = null">Back</button>
          <span class="flex-1" />
          <span class="inline-flex items-center gap-1.5 font-mono text-[11px] text-[var(--fg-4)]">
            <MagnifyingGlassIcon class="size-3.5" />
            looking
          </span>
        </template>
        <template v-else-if="step === 'none'">
          <button class="btn btn-ghost btn-sm" data-testid="pair-switch-channel" @click="switchChannel()">
            Try {{ channel === "network" ? "Bluetooth" : "Network" }} instead
          </button>
          <span class="flex-1" />
          <button class="btn btn-primary btn-sm" @click="keepLooking()">Keep looking</button>
        </template>
        <template v-else-if="step === 'declined' || step === 'timeout'">
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
