<script setup lang="ts">
import SettingsListGroup from "./SettingsListGroup.vue";
import SettingsListItem from "./SettingsListItem.vue";
import { useDeviceStore, type PairedDevice } from "../../stores/device";

const deviceStore = useDeviceStore();
function devicePresenceLabel(device: PairedDevice) {
  return deviceStore.isDeviceOnline(device) ? "Online" : "Offline";
}
</script>

<template>
  <section class="rounded-xl bg-base-200 p-3" data-testid="settings-devices">
    <h2 class="mb-3 text-sm font-semibold uppercase tracking-wide opacity-70">Devices</h2>
    <SettingsListGroup>
      <SettingsListItem v-for="device in deviceStore.pairedDevices" :key="device.peer_device_id" :to="`/settings/device/${device.peer_device_id}`" data-testid="paired-device-row" :data-peer-device-id="device.peer_device_id">
        <template #leading><span class="h-2.5 w-2.5 rounded-full" :class="deviceStore.isDeviceOnline(device) ? 'bg-green-500' : 'bg-gray-400'" /></template>
        <template #start><span class="block truncate font-medium" data-testid="paired-device-name">{{ device.display_name }}</span></template>
        <template #end><span class="text-xs opacity-60">{{ devicePresenceLabel(device) }}</span></template>
        <template #trailing><span class="text-sm opacity-50">›</span></template>
      </SettingsListItem>
      <SettingsListItem v-if="deviceStore.pairedDevices.length === 0"><span class="opacity-70">No paired devices yet.</span></SettingsListItem>
      <SettingsListItem to="/settings/add-device" data-testid="add-device-link"><template #leading><span class="text-base leading-none">+</span></template><template #start><span class="font-medium">Add device</span></template><template #trailing><span class="text-sm opacity-50">›</span></template></SettingsListItem>
    </SettingsListGroup>
  </section>
</template>
