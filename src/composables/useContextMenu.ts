import { reactive } from "vue";

export interface MenuItem {
  label?: string;
  action?: () => void;
  disabled?: boolean;
  children?: MenuItem[];
  separator?: boolean;
}

interface ContextMenuState {
  visible: boolean;
  x: number;
  y: number;
  items: MenuItem[];
}

const state = reactive<ContextMenuState>({
  visible: false,
  x: 0,
  y: 0,
  items: [],
});

function open(event: MouseEvent, items: MenuItem[]) {
  event.preventDefault();
  event.stopPropagation();
  state.x = event.clientX;
  state.y = event.clientY;
  state.items = items;
  state.visible = true;
}

function close() {
  state.visible = false;
  state.items = [];
}

export function useContextMenu() {
  return { state, open, close };
}
