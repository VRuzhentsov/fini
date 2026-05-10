import { reactive } from "vue";

export interface MenuItem {
  label?: string;
  action?: () => void;
  disabled?: boolean;
  children?: MenuItem[];
  separator?: boolean;
}

export type ContextMenuTrigger =
  | { kind: "pointer"; x: number; y: number }
  | { kind: "element"; rect: { left: number; top: number; right: number; bottom: number; width: number; height: number } };

interface ContextMenuState {
  visible: boolean;
  trigger: ContextMenuTrigger | null;
  items: MenuItem[];
}

const state = reactive<ContextMenuState>({
  visible: false,
  trigger: null,
  items: [],
});

function deriveTrigger(event: Event): ContextMenuTrigger {
  if (event.type === "contextmenu") {
    const me = event as MouseEvent;
    return { kind: "pointer", x: me.clientX, y: me.clientY };
  }
  const target = (event.currentTarget ?? event.target) as Element | null;
  if (target && typeof (target as HTMLElement).getBoundingClientRect === "function") {
    const r = (target as HTMLElement).getBoundingClientRect();
    return {
      kind: "element",
      rect: { left: r.left, top: r.top, right: r.right, bottom: r.bottom, width: r.width, height: r.height },
    };
  }
  const me = event as MouseEvent;
  return { kind: "pointer", x: me.clientX ?? 0, y: me.clientY ?? 0 };
}

function open(event: Event, items: MenuItem[]) {
  event.preventDefault?.();
  event.stopPropagation?.();
  state.trigger = deriveTrigger(event);
  state.items = items;
  state.visible = true;
}

function openFromRect(rect: DOMRect | DOMRectReadOnly, items: MenuItem[]) {
  state.trigger = {
    kind: "element",
    rect: { left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom, width: rect.width, height: rect.height },
  };
  state.items = items;
  state.visible = true;
}

function close() {
  state.visible = false;
  state.items = [];
  state.trigger = null;
}

export function useContextMenu() {
  return { state, open, openFromRect, close };
}
