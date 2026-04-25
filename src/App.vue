<script setup lang="ts">
import { onMounted } from "vue";
import IncomingSpaceResolutionDialog from "./components/DeviceView/IncomingSpaceResolutionDialog.vue";
import ToastStack from "./components/ToastStack.vue";
import ContextMenu from "./components/ContextMenu.vue";
import SpacePicker from "./components/SpacePicker.vue";
import { useDeviceStore } from "./stores/device";

const deviceStore = useDeviceStore();

onMounted(() => {
  void deviceStore.hydrate();
});
</script>

<template>
  <div class="app-shell">
    <nav class="nav">
      <div class="nav-links">
        <router-link to="/main">Focus</router-link>
        <router-link to="/history">History</router-link>
        <router-link to="/settings">Settings</router-link>
      </div>
      <SpacePicker />
    </nav>
    <main class="content">
      <router-view />
    </main>
    <IncomingSpaceResolutionDialog />
    <ToastStack />
    <ContextMenu />
  </div>
</template>

<style>
:root {
  font-family: Inter, Avenir, Helvetica, Arial, sans-serif;
  font-size: 16px;
  line-height: 24px;
  font-weight: 400;
  font-synthesis: none;
  text-rendering: optimizeLegibility;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}

html, body, #app {
  width: 100%;
  height: 100%;
  margin: 0;
  overflow: hidden;
  color: var(--color-base-content);
  background-color: var(--color-page-bg);
}

*, *::before, *::after {
  box-sizing: border-box;
}
</style>

<style scoped>
.app-shell {
  display: flex;
  flex-direction: column;
  height: 100vh;
  min-height: 0;
  overflow: hidden;
  color: var(--color-base-content);
  background-color: var(--color-page-bg);
  padding-top: env(safe-area-inset-top);
}

.nav {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-shrink: 0;
  padding: 0.75rem 1.5rem;
  border-bottom: 1px solid rgba(128, 128, 128, 0.2);
}

.nav-links {
  display: flex;
  gap: 0.1875rem;
  padding: 0.1875rem;
  background: var(--color-base-200);
  border-radius: 10px;
}

.nav-links a {
  padding: 0.375rem 0.75rem;
  text-decoration: none;
  color: inherit;
  opacity: 0.6;
  font-weight: 500;
  font-size: 0.8125rem;
  border-radius: 8px;
}

.nav-links a.router-link-active {
  opacity: 1;
  background: var(--color-base-100);
  box-shadow: var(--shadow-sm);
}

.content {
  flex: 1;
  min-height: 0;
  overflow: auto;
  overscroll-behavior: contain;
  width: 100%;
  padding: 1rem 1rem calc(10rem + env(safe-area-inset-bottom));
}

@media (max-width: 640px) {
  .nav { padding: 0.625rem 0.875rem; }
  .content { padding-inline: 0.75rem; }
}
</style>
