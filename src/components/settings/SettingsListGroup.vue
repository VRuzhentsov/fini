<template>
  <ul class="settings-list-group flex flex-col gap-0">
    <slot />
  </ul>
</template>

<style scoped>
.settings-list-group :deep(.settings-list-item:first-child > *) {
  border-top-left-radius: 0.5rem;
  border-top-right-radius: 0.5rem;
}

.settings-list-group :deep(.settings-list-item:last-child > *) {
  border-bottom-width: 0;
  border-bottom-left-radius: 0.5rem;
  border-bottom-right-radius: 0.5rem;
}
</style>
