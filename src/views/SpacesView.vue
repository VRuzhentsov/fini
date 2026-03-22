<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useSpaceStore } from "../stores/space";

const store = useSpaceStore();
const newName = ref("");
const editingId = ref<string | null>(null);
const editingName = ref("");

onMounted(() => store.fetchSpaces());

async function add() {
  const name = newName.value.trim();
  if (!name) return;
  await store.createSpace(name);
  newName.value = "";
}

function startEdit(id: string, name: string) {
  editingId.value = id;
  editingName.value = name;
}

async function confirmEdit(id: string) {
  const name = editingName.value.trim();
  if (name) await store.updateSpace(id, { name });
  editingId.value = null;
}

function cancelEdit() {
  editingId.value = null;
}
</script>

<template>
  <div class="flex flex-col gap-4 px-4 pt-4">
    <h2 class="text-xl font-semibold">Spaces</h2>

    <div v-if="store.error" class="text-error text-sm">{{ store.error }}</div>

    <ul class="flex flex-col gap-1">
      <li v-for="space in store.spaces" :key="space.id" class="flex items-center gap-3 px-4 py-3 rounded-xl bg-base-200">
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
          <span class="flex-1 font-medium text-sm">{{ space.name }}</span>
          <button class="btn btn-sm btn-ghost" @click="startEdit(space.id, space.name)">Edit</button>
          <button v-if="!['1', '2', '3'].includes(space.id)" class="btn btn-sm btn-error btn-outline" @click="store.deleteSpace(space.id)">Delete</button>
        </template>
      </li>
    </ul>

    <form class="flex gap-3" @submit.prevent="add">
      <input v-model="newName" class="input input-bordered flex-1" placeholder="New space name" />
      <button type="submit" class="btn btn-primary">Add</button>
    </form>
  </div>
</template>
