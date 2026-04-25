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
  min-height: 100%;
  margin: 0;
  color: var(--color-base-content);
  background-color: var(--color-base-100);
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
  color: var(--color-base-content);
  background-color: var(--color-base-100);
  padding-top: env(safe-area-inset-top);
}

.nav {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.75rem 1.5rem;
  border-bottom: 1px solid rgba(128, 128, 128, 0.2);
}

.nav-links {
  display: flex;
  gap: 1rem;
}

.nav-links a {
  text-decoration: none;
  color: inherit;
  opacity: 0.6;
  font-weight: 500;
}

.nav-links a.router-link-active {
  opacity: 1;
}

.content {
  flex: 1;
  overflow: auto;
  padding-bottom: calc(4rem + env(safe-area-inset-bottom));
}
</style>
