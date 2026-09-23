<script setup lang="ts">
import { ref } from "vue";
import { PencilIcon, TrashIcon } from "@heroicons/vue/24/outline";
import ActionsBtn from "../ActionsBtn.vue";
import SettingsListGroup from "./SettingsListGroup.vue";
import SettingsListItem from "./SettingsListItem.vue";
import { useSpaceStore, isBuiltinSpace } from "../../stores/space";
import { useContextMenu, type MenuItem } from "../../composables/useContextMenu";

const spaceStore = useSpaceStore();
const contextMenu = useContextMenu();
const newSpaceName = ref("");
const editingId = ref<string | null>(null);
const editingName = ref("");

function openSpaceMenu(event: MouseEvent, spaceId: string, spaceName: string) {
  const items: MenuItem[] = [{ label: "Edit", icon: PencilIcon, action: () => startEdit(spaceId, spaceName) }];
  if (!isBuiltinSpace(spaceId)) {
    items.push({ separator: true });
    items.push({ label: "Delete", icon: TrashIcon, danger: true, action: () => spaceStore.deleteSpace(spaceId) });
  }
  contextMenu.open(event, items);
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
  <section class="rounded-xl bg-base-200 p-3" data-testid="settings-spaces">
    <h2 class="mb-3 text-sm font-semibold uppercase tracking-wide opacity-70">Spaces</h2>
    <div class="flex flex-col gap-3">
      <div v-if="spaceStore.error" class="text-error text-sm">{{ spaceStore.error }}</div>
    <SettingsListGroup v-if="spaceStore.spaces.length > 0">
      <template v-for="space in spaceStore.spaces" :key="space.id">
        <SettingsListItem v-if="editingId === space.id">
          <template #start>
            <input v-model="editingName" class="input input-bordered input-sm w-full" @keyup.enter="confirmEdit(space.id)" @keyup.escape="cancelEdit" autofocus />
          </template>
          <template #end>
            <div class="flex items-center justify-end gap-2">
              <button class="btn btn-sm btn-primary" @click="confirmEdit(space.id)">Save</button>
              <button class="btn btn-sm btn-ghost" @click="cancelEdit">Cancel</button>
            </div>
          </template>
        </SettingsListItem>
        <SettingsListItem v-else>
          <template #start><span class="block truncate font-medium">{{ space.name }}</span></template>
          <template #end><ActionsBtn :aria-label="`Actions for ${space.name}`" @click="openSpaceMenu($event, space.id, space.name)" /></template>
        </SettingsListItem>
      </template>
    </SettingsListGroup>
    <form @submit.prevent="addSpace">
      <SettingsListGroup>
        <SettingsListItem>
          <template #start><input v-model="newSpaceName" class="input input-bordered input-sm w-full" placeholder="New space name" /></template>
          <template #end><button type="submit" class="btn btn-sm btn-primary">Add</button></template>
        </SettingsListItem>
      </SettingsListGroup>
    </form>
    </div>
  </section>
</template>
