<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useModelDownload } from "../composables/useModelDownload";
import { useSpaceStore } from "../stores/space";
import { useDeviceStore } from "../stores/device";

const model = useModelDownload();
const spaceStore = useSpaceStore();
const deviceStore = useDeviceStore();

const newSpaceName = ref("");
const editingId = ref<string | null>(null);
const editingName = ref("");

onMounted(() => {
  model.checkDownloaded();
  spaceStore.fetchSpaces();
  void deviceStore.hydrate();
});

function progressLabel(): string {
  const p = model.progress.value;
  if (!p) return "";
  const pct = p.percent >= 0 ? `${p.percent}%` : "…";
  return `Downloading ${p.file} (${p.file_index + 1}/${p.file_count}) ${pct}`;
}

function progressValue(): number | undefined {
  const percent = model.progress.value?.percent;
  return typeof percent === "number" && percent >= 0 ? percent : undefined;
}

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
</script>

<template>
  <div class="flex flex-col gap-3 px-4 pt-4 pb-24">

    <section class="rounded-xl bg-base-200 p-3">
      <h2 class="mb-3 text-sm font-semibold uppercase tracking-wide opacity-70">Spaces</h2>
      <div class="flex flex-col gap-3">
        <div v-if="spaceStore.error" class="text-error text-sm">{{ spaceStore.error }}</div>
        <ul class="flex flex-col gap-1">
          <li
            v-for="space in spaceStore.spaces"
            :key="space.id"
            class="flex items-center gap-3 rounded-lg bg-base-100 px-3 py-2"
          >
            <template v-if="editingId === space.id">
              <input
                v-model="editingName"
                class="input input-bordered input-sm flex-1"
                @keyup.enter="confirmEdit(space.id)"
                @keyup.escape="cancelEdit"
                autofocus
              />
              <button class="btn btn-sm btn-primary" @click="confirmEdit(space.id)">Save</button>
              <button class="btn btn-sm btn-ghost" @click="cancelEdit">Cancel</button>
            </template>
            <template v-else>
              <span class="flex-1 text-sm font-medium">{{ space.name }}</span>
              <button class="btn btn-sm btn-ghost" @click="startEdit(space.id, space.name)">Edit</button>
              <button v-if="!['1', '2', '3'].includes(space.id)" class="btn btn-sm btn-error btn-outline" @click="spaceStore.deleteSpace(space.id)">Delete</button>
            </template>
          </li>
        </ul>
        <form class="flex gap-2" @submit.prevent="addSpace">
          <input v-model="newSpaceName" class="input input-bordered input-sm flex-1" placeholder="New space name" />
          <button type="submit" class="btn btn-sm btn-primary">Add</button>
        </form>
      </div>
    </section>

    <section class="rounded-xl bg-base-200 p-3">
      <h2 class="mb-3 text-sm font-semibold uppercase tracking-wide opacity-70">Devices</h2>
      <ul class="flex flex-col gap-1">
        <li v-for="device in deviceStore.pairedDevices" :key="device.id">
          <router-link
            :to="`/settings/device/${device.id}`"
            class="flex items-center gap-3 rounded-lg bg-base-100 px-3 py-2"
          >
            <span
              class="h-2.5 w-2.5 rounded-full"
              :class="deviceStore.isDeviceOnline(device) ? 'bg-green-500' : 'bg-gray-400'"
            />
            <span class="flex-1 text-sm font-medium">{{ device.display_name }}</span>
            <span class="text-xs opacity-60">{{ deviceStore.shortDeviceId(device.id) }}</span>
            <span class="text-sm opacity-50">›</span>
          </router-link>
        </li>
        <li
          v-if="deviceStore.pairedDevices.length === 0"
          class="rounded-lg bg-base-100 px-3 py-2 text-sm opacity-70"
        >
          No paired devices yet.
        </li>
        <li>
          <router-link
            to="/settings/add-device"
            class="flex items-center gap-3 rounded-lg bg-base-100 px-3 py-2"
          >
            <span class="text-base leading-none">+</span>
            <span class="flex-1 text-sm font-medium">Add device</span>
            <span class="text-sm opacity-50">›</span>
          </router-link>
        </li>
      </ul>
    </section>

    <section class="rounded-xl bg-base-200 p-3">
      <h2 class="mb-3 text-sm font-semibold uppercase tracking-wide opacity-70">Voice Model</h2>
      <div class="flex flex-col gap-3">
        <p class="text-sm opacity-70 leading-relaxed">
          On-device speech recognition via sherpa-onnx.<br />
          Model: <code class="font-mono text-xs">sherpa-onnx-streaming-zipformer-small-en</code> (~60 MB)
        </p>
        <div>
          <span v-if="model.downloaded.value" class="badge badge-success badge-sm font-semibold">Ready</span>
          <span v-else class="badge badge-warning badge-sm font-semibold">Not downloaded</span>
        </div>
        <div v-if="model.downloading.value" class="flex flex-col gap-1">
          <progress
            class="progress progress-primary w-full"
            :value="progressValue()"
            max="100"
          />
          <span class="text-xs opacity-60">{{ progressLabel() }}</span>
        </div>
        <p v-if="model.error.value" class="text-error text-sm">{{ model.error.value }}</p>
        <button
          class="btn btn-primary btn-sm self-start"
          :disabled="model.downloading.value || model.downloaded.value"
          @click="model.startDownload()"
        >
          {{ model.downloaded.value ? "Downloaded" : model.downloading.value ? "Downloading…" : "Download Model" }}
        </button>
      </div>
    </section>

  </div>
</template>
