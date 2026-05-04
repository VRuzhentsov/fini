<script setup lang="ts">
import { computed, useSlots } from "vue";
import { RouterLink } from "vue-router";

const props = defineProps<{
  to?: string;
  href?: string;
  button?: boolean;
}>();

const emit = defineEmits<{
  click: [event: MouseEvent];
}>();

const slots = useSlots();
const hasTwoColumns = computed(() => Boolean(slots.start || slots.end));
const rowComponent = computed(() => {
  if (props.to) return RouterLink;
  if (props.href) return "a";
  if (props.button) return "button";
  return "div";
});
const rowAttrs = computed(() => {
  if (props.to) return { to: props.to };
  if (props.href) return { href: props.href, target: "_blank", rel: "noreferrer noopener" };
  if (props.button) return { type: "button" };
  return {};
});

function handleClick(event: MouseEvent) {
  if (props.button) emit("click", event);
}
</script>

<template>
  <li class="settings-list-item">
    <component
      :is="rowComponent"
      v-bind="rowAttrs"
      class="flex min-h-10 w-full items-center gap-3 border-b border-base-200 bg-base-100 px-3 py-2 text-left text-sm"
      @click="handleClick"
    >
      <slot name="leading" />

      <div v-if="hasTwoColumns" class="flex min-w-0 flex-1 items-center justify-between gap-3">
        <div class="min-w-0 flex-1">
          <slot name="start" />
        </div>
        <div class="max-w-[50%] shrink-0 text-right">
          <slot name="end" />
        </div>
      </div>
      <div v-else class="min-w-0 flex-1">
        <slot />
      </div>

      <slot name="trailing" />
    </component>
  </li>
</template>
