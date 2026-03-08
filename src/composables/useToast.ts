import { ref } from "vue";

export type ToastType = "error" | "info" | "success";

interface Toast {
  id: number;
  message: string;
  type: ToastType;
}

// Module-level singleton so all callers share the same list
const toasts = ref<Toast[]>([]);
let nextId = 0;

export function useToast() {
  function show(message: string, type: ToastType = "info", duration = 1000) {
    const id = nextId++;
    toasts.value.push({ id, message, type });
    setTimeout(() => {
      toasts.value = toasts.value.filter((t) => t.id !== id);
    }, duration);
  }

  function error(message: string) { show(message, "error"); }
  function info(message: string)  { show(message, "info"); }

  return { toasts, show, error, info };
}
