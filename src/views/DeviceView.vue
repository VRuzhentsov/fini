<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useDeviceStore } from "../stores/device";

const route = useRoute();
const router = useRouter();
const deviceStore = useDeviceStore();
const unpairDialog = ref<HTMLDialogElement | null>(null);

const deviceId = computed(() => String(route.params.id ?? ""));
const device = computed(() => deviceStore.findPairedDevice(deviceId.value));
const online = computed(() => (device.value ? deviceStore.isDeviceOnline(device.value) : false));

onMounted(() => {
  void deviceStore.hydrate();
});

function openUnpairDialog() {
  unpairDialog.value?.showModal();
}

async function confirmUnpair() {
  if (!device.value) return;

  unpairDialog.value?.close();
  deviceStore.unpairDevice(device.value.id);
  await router.push("/settings");
}
</script>

<template>
  <div class="flex flex-col gap-4 px-4 pt-4 pb-24">
    <header class="flex items-center justify-between rounded-xl bg-base-200 px-3 py-2">
      <router-link to="/settings" class="text-sm font-medium opacity-70">‹ Settings</router-link>
      <span class="text-sm font-semibold">Device</span>
      <span class="text-xs opacity-60" v-if="device">{{ deviceStore.shortDeviceId(device.id) }}</span>
      <span class="text-xs opacity-60" v-else>Unknown</span>
    </header>

    <section v-if="device" class="rounded-xl bg-base-200 p-3">
      <div class="flex items-center gap-3">
        <span class="h-2.5 w-2.5 rounded-full" :class="online ? 'bg-green-500' : 'bg-gray-400'" />
        <h1 class="text-base font-semibold">{{ device.display_name }}</h1>
      </div>
      <p class="mt-2 text-xs opacity-60">paired at {{ new Date(device.paired_at).toLocaleString() }}</p>
      <p class="text-xs opacity-60">
        {{ online ? "online" : "offline" }}
        <template v-if="device.last_seen_at">
          · last seen {{ new Date(device.last_seen_at).toLocaleString() }}
        </template>
      </p>
    </section>

    <section v-if="device" class="rounded-xl bg-base-200 p-3">
      <h2 class="mb-2 text-sm font-semibold uppercase tracking-wide opacity-70">Mapped spaces</h2>
      <div class="rounded-lg bg-base-100 px-3 py-2 text-sm opacity-70">TBD</div>
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
