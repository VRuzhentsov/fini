import { defineStore } from "pinia";
import { invoke } from "@tauri-apps/api/core";
import { ref } from "vue";

export interface Space {
  id: string;
  name: string;
  item_order: number;
  created_at: string;
}

export const SPACE_COLOR_CLASS: Record<string, string> = {
  "1": "space-color-personal",
  "2": "space-color-family",
  "3": "space-color-work",
};

export const useSpaceStore = defineStore("space", () => {
  const spaces = ref<Space[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);
  const selectedSpaceId = ref<string | null>(null);

  function selectSpace(id: string | null) {
    selectedSpaceId.value = id;
  }

  async function fetchSpaces() {
    loading.value = true;
    error.value = null;
    try {
      spaces.value = await invoke<Space[]>("get_spaces");
    } catch (e) {
      error.value = String(e);
    } finally {
      loading.value = false;
    }
  }

  async function createSpace(name: string) {
    const item_order = spaces.value.length;
    const space = await invoke<Space>("create_space", {
      input: { name, item_order },
    });
    spaces.value.push(space);
    return space;
  }

  async function updateSpace(id: string, patch: { name?: string; item_order?: number }) {
    const space = await invoke<Space>("update_space", { id, input: patch });
    const idx = spaces.value.findIndex((s) => s.id === id);
    if (idx !== -1) spaces.value[idx] = space;
    return space;
  }

  async function deleteSpace(id: string) {
    await invoke("delete_space", { id });
    spaces.value = spaces.value.filter((s) => s.id !== id);
  }

  return { spaces, loading, error, selectedSpaceId, selectSpace, fetchSpaces, createSpace, updateSpace, deleteSpace };
});
