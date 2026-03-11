<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useSpaceStore } from "../stores/space";

const store = useSpaceStore();
const newName = ref("");
const editingId = ref<number | null>(null);
const editingName = ref("");

onMounted(() => store.fetchSpaces());

async function add() {
  const name = newName.value.trim();
  if (!name) return;
  await store.createSpace(name);
  newName.value = "";
}

function startEdit(id: number, name: string) {
  editingId.value = id;
  editingName.value = name;
}

async function confirmEdit(id: number) {
  const name = editingName.value.trim();
  if (name) await store.updateSpace(id, { name });
  editingId.value = null;
}

function cancelEdit() {
  editingId.value = null;
}
</script>

<template>
  <div class="spaces-view">
    <h2>Spaces</h2>

    <div v-if="store.error" class="error">{{ store.error }}</div>

    <ul class="space-list">
      <li v-for="space in store.spaces" :key="space.id" class="space-item">
        <template v-if="editingId === space.id">
          <input
            v-model="editingName"
            class="edit-input"
            @keyup.enter="confirmEdit(space.id)"
            @keyup.escape="cancelEdit"
            autofocus
          />
          <button @click="confirmEdit(space.id)">Save</button>
          <button @click="cancelEdit">Cancel</button>
        </template>
        <template v-else>
          <span class="space-name">{{ space.name }}</span>
          <button @click="startEdit(space.id, space.name)">Edit</button>
          <button v-if="space.id !== 1" class="danger" @click="store.deleteSpace(space.id)">Delete</button>
        </template>
      </li>
    </ul>

    <form class="add-form" @submit.prevent="add">
      <input v-model="newName" placeholder="New space name" />
      <button type="submit">Add</button>
    </form>
  </div>
</template>

<style scoped>
.spaces-view {
  padding: 1.5rem;
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
}

h2 {
  font-size: 1.25rem;
  font-weight: 600;
}

.error {
  color: red;
  font-size: 0.875rem;
}

.space-list {
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.space-item {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0.75rem 1rem;
  border-radius: 0.5rem;
  background: rgba(128, 128, 128, 0.08);
}

.space-name {
  flex: 1;
  font-weight: 500;
}

.edit-input {
  flex: 1;
  padding: 0.25rem 0.5rem;
  border: 1px solid rgba(128, 128, 128, 0.4);
  border-radius: 0.25rem;
  background: transparent;
  color: inherit;
  font: inherit;
}

.add-form {
  display: flex;
  gap: 0.75rem;
}

.add-form input {
  flex: 1;
  padding: 0.5rem 0.75rem;
  border: 1px solid rgba(128, 128, 128, 0.4);
  border-radius: 0.5rem;
  background: transparent;
  color: inherit;
  font: inherit;
}

button {
  padding: 0.375rem 0.75rem;
  border-radius: 0.375rem;
  border: 1px solid rgba(128, 128, 128, 0.3);
  background: transparent;
  color: inherit;
  cursor: pointer;
  font: inherit;
  font-size: 0.875rem;
}

button.danger {
  color: #e55;
  border-color: #e55;
}
</style>
