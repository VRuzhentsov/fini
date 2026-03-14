<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useModelDownload } from "../composables/useModelDownload";
import { useSpaceStore } from "../stores/space";

const model = useModelDownload();
const spaceStore = useSpaceStore();

const newSpaceName = ref("");
const editingId = ref<number | null>(null);
const editingName = ref("");

onMounted(() => {
  model.checkDownloaded();
  spaceStore.fetchSpaces();
});

function progressLabel(): string {
  const p = model.progress.value;
  if (!p) return "";
  const pct = p.percent >= 0 ? `${p.percent}%` : "…";
  return `Downloading ${p.file} (${p.file_index + 1}/${p.file_count}) ${pct}`;
}

async function addSpace() {
  const name = newSpaceName.value.trim();
  if (!name) return;
  await spaceStore.createSpace(name);
  newSpaceName.value = "";
}

function startEdit(id: number, name: string) {
  editingId.value = id;
  editingName.value = name;
}

async function confirmEdit(id: number) {
  const name = editingName.value.trim();
  if (name) await spaceStore.updateSpace(id, { name });
  editingId.value = null;
}

function cancelEdit() {
  editingId.value = null;
}
</script>

<template>
  <div class="flex flex-col px-4 pt-4 pb-24">

    <!-- Spaces -->
    <div class="collapse collapse-arrow bg-base-200 rounded-xl mb-2">
      <input type="checkbox" />
      <div class="collapse-title font-semibold">Spaces</div>
      <div class="collapse-content flex flex-col gap-3">
        <div v-if="spaceStore.error" class="text-error text-sm">{{ spaceStore.error }}</div>
        <ul class="flex flex-col gap-1">
          <li v-for="space in spaceStore.spaces" :key="space.id" class="flex items-center gap-3 px-3 py-2 rounded-lg bg-base-100">
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
              <button v-if="space.id !== 1" class="btn btn-sm btn-error btn-outline" @click="spaceStore.deleteSpace(space.id)">Delete</button>
            </template>
          </li>
        </ul>
        <form class="flex gap-2" @submit.prevent="addSpace">
          <input v-model="newSpaceName" class="input input-bordered input-sm flex-1" placeholder="New space name" />
          <button type="submit" class="btn btn-sm btn-primary">Add</button>
        </form>
      </div>
    </div>

    <!-- Voice Model -->
    <div class="collapse collapse-arrow bg-base-200 rounded-xl mb-2">
      <input type="checkbox" />
      <div class="collapse-title font-semibold">Voice Model</div>
      <div class="collapse-content flex flex-col gap-3">
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
            :value="model.progress.value?.percent >= 0 ? model.progress.value?.percent : undefined"
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
    </div>

  </div>
</template>
