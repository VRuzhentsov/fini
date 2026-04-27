<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useRouter } from "vue-router";
import { ADD_MODE_DISCOVERY_INTERVAL_MS, useDeviceStore } from "../stores/device";

const deviceStore = useDeviceStore();
const router = useRouter();
const codeInputByRequest = ref<Record<string, string>>({});
const acceptedIncomingByRequest = ref<Record<string, boolean>>({});

const nowMs = ref(Date.now());
let clockTimer: ReturnType<typeof setInterval> | null = null;

const outgoingSecondsLeft = computed(() => {
  const request = deviceStore.outgoingRequest;
  if (!request) return 0;
  const diff = Date.parse(request.expires_at) - nowMs.value;
  return diff > 0 ? Math.ceil(diff / 1000) : 0;
});

const outgoingPending = computed(() => {
  const request = deviceStore.outgoingRequest;
  return !!request && (request.status === "pending" || request.status === "awaiting_code");
});

const nextRefreshSecondsLeft = computed(() => {
  const lastRefresh = deviceStore.addModeLastRefreshAt;
  const fallback = Math.ceil(ADD_MODE_DISCOVERY_INTERVAL_MS / 1000);
  if (!lastRefresh) return fallback;

  const lastRefreshMs = Date.parse(lastRefresh);
  if (Number.isNaN(lastRefreshMs)) return fallback;

  const elapsedMs = Math.max(0, nowMs.value - lastRefreshMs);
  const remainingMs = ADD_MODE_DISCOVERY_INTERVAL_MS - (elapsedMs % ADD_MODE_DISCOVERY_INTERVAL_MS);
  return Math.ceil(remainingMs / 1000);
});

onMounted(() => {
  void deviceStore.hydrate();
  void deviceStore.enterAddMode();
  void deviceStore.refreshDebugStatus();

  clockTimer = setInterval(() => {
    nowMs.value = Date.now();
    void deviceStore.refreshDebugStatus();
  }, 1000);
});

onUnmounted(() => {
  if (clockTimer) {
    clearInterval(clockTimer);
    clockTimer = null;
  }
  void deviceStore.leaveAddMode();
});

watch(
  () => deviceStore.pairCompletedAt,
  (value, previous) => {
    if (!value || value === previous) return;
    void router.push("/settings");
  },
);

async function requestPair(deviceId: string) {
  const device = deviceStore.discoveredDevices.find((item) => item.device_id === deviceId);
  if (!device) return;
  await deviceStore.requestPair(device);
}

function incomingSecondsLeft(expiresAt: string): number {
  const diff = Date.parse(expiresAt) - nowMs.value;
  return diff > 0 ? Math.ceil(diff / 1000) : 0;
}

async function submitCode(requestId: string) {
  const code = (codeInputByRequest.value[requestId] ?? "")
    .replace(/\D/g, "")
    .slice(0, 6);
  if (code.length !== 6) return;
  const paired = await deviceStore.submitPairCode(requestId, code);
  if (paired) {
    codeInputByRequest.value[requestId] = "";
    delete acceptedIncomingByRequest.value[requestId];
  }
}

async function acceptRequest(requestId: string) {
  const accepted = await deviceStore.acceptIncomingRequest(requestId);
  if (accepted) {
    acceptedIncomingByRequest.value[requestId] = true;
  }
}

function rejectRequest(requestId: string) {
  delete acceptedIncomingByRequest.value[requestId];
  void deviceStore.rejectIncomingRequest(requestId);
}
</script>

<template>
  <div class="flex flex-col gap-4 pb-24">
    <header class="flex items-center justify-between rounded-xl bg-base-200 px-3 py-2">
      <router-link to="/settings" class="text-sm font-medium opacity-70">‹ Settings</router-link>
      <span class="text-sm font-semibold">Add Device</span>
      <span class="text-xs opacity-60">{{ deviceStore.shortDeviceId(deviceStore.identity.device_id) }}</span>
    </header>

    <section class="rounded-xl bg-base-200 p-3">
      <p class="text-sm opacity-70">
        Add Device mode is active. Discovery beacons run every 5s while this view is open.
      </p>
      <p class="mt-1 text-xs opacity-60">Next refresh in {{ nextRefreshSecondsLeft }}s</p>
    </section>

    <section v-if="deviceStore.debugStatus" class="rounded-xl bg-base-200 p-3">
      <h2 class="mb-2 text-sm font-semibold uppercase tracking-wide opacity-70">Debug</h2>
      <p class="text-xs opacity-70">
        worker {{ deviceStore.debugStatus.worker_started ? 'ready' : 'not ready' }} ·
        add-mode {{ deviceStore.debugStatus.add_mode_enabled ? 'on' : 'off' }} ·
        port {{ deviceStore.debugStatus.discovery_port }}
      </p>
      <p class="text-xs opacity-70">
        tx {{ deviceStore.debugStatus.tx_count }} · rx {{ deviceStore.debugStatus.rx_count }} ·
        seen {{ deviceStore.debugStatus.discovered_count }} ·
        sessions {{ deviceStore.debugStatus.peer_session_count }} ·
        incoming {{ deviceStore.debugStatus.incoming_request_count }} ·
        mapping {{ deviceStore.debugStatus.incoming_space_mapping_update_count }} ·
        outgoing {{ deviceStore.debugStatus.outgoing_code_count }}
      </p>
      <p class="text-xs text-error" v-if="deviceStore.debugStatus.last_error">
        {{ deviceStore.debugStatus.last_error }}
      </p>
    </section>

    <section v-if="deviceStore.incomingRequests.length" class="rounded-xl bg-base-200 p-3" data-testid="incoming-requests">
      <h2 class="mb-2 text-sm font-semibold uppercase tracking-wide opacity-70">Incoming requests</h2>
      <ul class="flex flex-col gap-2">
        <li
          v-for="request in deviceStore.incomingRequests"
          :key="request.request_id"
          data-testid="incoming-request-row"
          :data-from-hostname="request.from_hostname"
          class="rounded-lg bg-base-100 p-3"
        >
          <p class="text-sm font-medium">{{ request.from_hostname }}</p>
          <p class="text-xs opacity-60">
            {{ deviceStore.shortDeviceId(request.from_device_id) }} · expires in {{ incomingSecondsLeft(request.expires_at) }}s
          </p>
          <div
            v-if="!acceptedIncomingByRequest[request.request_id]"
            class="mt-2 flex flex-wrap gap-2"
          >
            <button class="btn btn-sm btn-primary" data-testid="accept-incoming-request" @click="void acceptRequest(request.request_id)">Accept</button>
            <button class="btn btn-sm btn-ghost" @click="rejectRequest(request.request_id)">Reject</button>
          </div>
          <div v-else class="mt-2 flex gap-2">
            <input
              v-model="codeInputByRequest[request.request_id]"
              maxlength="6"
              type="tel"
              inputmode="numeric"
              pattern="[0-9]*"
              class="input input-bordered input-sm w-28"
              data-testid="pair-code-input"
              placeholder="6-digit"
            />
            <button class="btn btn-sm" data-testid="pair-code-submit" @click="void submitCode(request.request_id)">Submit code</button>
          </div>
          <p v-if="request.cooldown_until && incomingSecondsLeft(request.cooldown_until) > 0" class="mt-2 text-xs text-warning">
            Too many wrong codes. Try again in {{ incomingSecondsLeft(request.cooldown_until) }}s.
          </p>
        </li>
      </ul>
    </section>

    <section class="rounded-xl bg-base-200 p-3" data-testid="nearby-devices">
      <h2 class="mb-2 text-sm font-semibold uppercase tracking-wide opacity-70">Nearby devices</h2>
      <ul class="flex flex-col gap-1">
        <li v-for="device in deviceStore.discoveredDevices" :key="device.device_id" data-testid="nearby-device-row" :data-device-hostname="device.hostname" :data-device-id="device.device_id">
          <div class="flex items-center gap-3 rounded-lg bg-base-100 px-3 py-2">
            <span class="h-2.5 w-2.5 rounded-full bg-green-500" />
            <span class="flex-1 text-sm font-medium">{{ device.hostname }}</span>
            <span class="text-xs opacity-60">{{ deviceStore.shortDeviceId(device.device_id) }} · {{ device.addr }}</span>
            <button
              class="btn btn-sm btn-primary"
              data-testid="request-pair"
              :disabled="outgoingPending"
              @click="void requestPair(device.device_id)"
            >
              Pair
            </button>
          </div>
        </li>
        <li v-if="!deviceStore.discoveredDevices.length" class="rounded-lg bg-base-100 px-3 py-2 text-sm opacity-70">
          No devices discovered in Add Device mode yet.
        </li>
      </ul>
      <p v-if="outgoingPending" class="mt-2 text-xs opacity-60">
        Pair request in progress. Complete or cancel it before sending another.
      </p>
    </section>

    <section v-if="deviceStore.outgoingRequest" class="rounded-xl bg-base-200 p-3" data-testid="outgoing-pair-request">
      <h2 class="mb-2 text-sm font-semibold uppercase tracking-wide opacity-70">Pair request</h2>
      <p class="text-sm font-medium">{{ deviceStore.outgoingRequest.to_hostname }}</p>
      <p class="text-xs opacity-60">
        {{ deviceStore.shortDeviceId(deviceStore.outgoingRequest.to_device_id) }} · {{ deviceStore.outgoingRequest.status }}
      </p>
      <p class="text-xs opacity-60" v-if="deviceStore.outgoingRequest.status === 'pending'">
        Expires in {{ outgoingSecondsLeft }}s
      </p>
      <div v-if="deviceStore.outgoingRequest.sender_code" class="mt-2 rounded-lg bg-base-100 px-3 py-2">
        <p class="text-xs opacity-60">Share this code with the receiving device</p>
        <p class="font-mono text-lg tracking-wider" data-testid="pair-code">{{ deviceStore.outgoingRequest.sender_code }}</p>
      </div>
      <button class="btn btn-sm btn-ghost mt-2" @click="deviceStore.cancelOutgoingRequest()">Cancel request</button>
    </section>
  </div>
</template>
