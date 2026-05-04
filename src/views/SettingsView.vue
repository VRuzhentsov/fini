<script setup lang="ts">
import { ref, onMounted } from "vue";
import packageJson from "../../package.json";
import AboutCard from "../components/SettingsView/AboutCard.vue";
import SettingsListGroup from "../components/SettingsView/SettingsListGroup.vue";
import SettingsListItem from "../components/SettingsView/SettingsListItem.vue";
import ThemeSelector from "../components/SettingsView/ThemeSelector.vue";
import { useSpaceStore } from "../stores/space";
import { useDeviceStore, type PairedDevice } from "../stores/device";

const spaceStore = useSpaceStore();
const deviceStore = useDeviceStore();

const newSpaceName = ref("");
const editingId = ref<string | null>(null);
const editingName = ref("");
const appVersion = packageJson.version;
const sourceUrl = "https://github.com/VRuzhentsov/fini";

onMounted(() => {
  spaceStore.fetchSpaces();
  void deviceStore.hydrate();
});

async function addSpace() {
  const name = newSpaceName.value.trim();
  if (!name) return;
  await spaceStore.createSpace(name);
  newSpaceName.value = "";
}

function startEdit(id: string, name: string) {
  editingId.value = id;
  editingName.value = name;
}

async function confirmEdit(id: string) {
  const name = editingName.value.trim();
  if (name) await spaceStore.updateSpace(id, { name });
  editingId.value = null;
}

function cancelEdit() {
  editingId.value = null;
}

function devicePresenceLabel(device: PairedDevice) {
  return deviceStore.isDeviceOnline(device) ? "Online" : "Offline";
}
</script>

<template>
  <div class="flex flex-col gap-3 pb-24">

    <section class="rounded-xl bg-base-200 p-3">
      <h2 class="mb-3 text-sm font-semibold uppercase tracking-wide opacity-70">Spaces</h2>
      <div class="flex flex-col gap-3">
        <div v-if="spaceStore.error" class="text-error text-sm">{{ spaceStore.error }}</div>
        <SettingsListGroup>
          <template v-for="space in spaceStore.spaces" :key="space.id">
            <SettingsListItem v-if="editingId === space.id">
              <template #start>
                <input
                  v-model="editingName"
                  class="input input-bordered input-sm w-full"
                  @keyup.enter="confirmEdit(space.id)"
                  @keyup.escape="cancelEdit"
                  autofocus
                />
              </template>
              <template #end>
                <div class="flex items-center justify-end gap-2">
                  <button class="btn btn-sm btn-primary" @click="confirmEdit(space.id)">Save</button>
                  <button class="btn btn-sm btn-ghost" @click="cancelEdit">Cancel</button>
                </div>
              </template>
            </SettingsListItem>
            <SettingsListItem v-else>
              <template #start>
                <span class="block truncate font-medium">{{ space.name }}</span>
              </template>
              <template #end>
                <div class="dropdown dropdown-end">
                  <button tabindex="0" class="btn btn-xs btn-ghost">Actions</button>
                  <ul tabindex="0" class="dropdown-content menu rounded-box bg-base-100 z-10 mt-2 w-36 p-2 shadow">
                    <li><button @click="startEdit(space.id, space.name)">Edit</button></li>
                    <li v-if="!['1', '2', '3'].includes(space.id)">
                      <button class="text-error" @click="spaceStore.deleteSpace(space.id)">Delete</button>
                    </li>
                  </ul>
                </div>
              </template>
            </SettingsListItem>
          </template>
        </SettingsListGroup>
        <form @submit.prevent="addSpace">
          <SettingsListGroup>
            <SettingsListItem>
              <template #start>
                <input v-model="newSpaceName" class="input input-bordered input-sm w-full" placeholder="New space name" />
              </template>
              <template #end>
                <button type="submit" class="btn btn-sm btn-primary">Add</button>
              </template>
            </SettingsListItem>
          </SettingsListGroup>
        </form>
      </div>
    </section>

    <section class="rounded-xl bg-base-200 p-3" data-testid="settings-devices">
      <h2 class="mb-3 text-sm font-semibold uppercase tracking-wide opacity-70">Devices</h2>
      <SettingsListGroup>
        <SettingsListItem
          v-for="device in deviceStore.pairedDevices"
          :key="device.peer_device_id"
          :to="`/settings/device/${device.peer_device_id}`"
          data-testid="paired-device-row"
          :data-peer-device-id="device.peer_device_id"
        >
          <template #leading>
            <span
              class="h-2.5 w-2.5 rounded-full"
              :class="deviceStore.isDeviceOnline(device) ? 'bg-green-500' : 'bg-gray-400'"
            />
          </template>
          <template #start>
            <span class="block truncate font-medium" data-testid="paired-device-name">{{ device.display_name }}</span>
          </template>
          <template #end>
            <span class="text-xs opacity-60">{{ devicePresenceLabel(device) }}</span>
          </template>
          <template #trailing>
            <span class="text-sm opacity-50">›</span>
          </template>
        </SettingsListItem>
        <SettingsListItem
          v-if="deviceStore.pairedDevices.length === 0"
        >
          <span class="opacity-70">No paired devices yet.</span>
        </SettingsListItem>
        <SettingsListItem to="/settings/add-device" data-testid="add-device-link">
          <template #leading>
            <span class="text-base leading-none">+</span>
          </template>
          <template #start>
            <span class="font-medium">Add device</span>
          </template>
          <template #trailing>
            <span class="text-sm opacity-50">›</span>
          </template>
        </SettingsListItem>
      </SettingsListGroup>
    </section>

    <ThemeSelector />

    <AboutCard :version="appVersion" :source-url="sourceUrl" />
  </div>
</template>
