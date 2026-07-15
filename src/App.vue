<script setup lang="ts">
import { onMounted } from "vue";
import IncomingSpaceResolutionDialog from "./components/DeviceView/IncomingSpaceResolutionDialog.vue";
import ToastStack from "./components/ToastStack.vue";
import ContextMenu from "./components/ContextMenu.vue";
import SpacePicker from "./components/SpacePicker.vue";
import { useDeviceStore } from "./stores/device";
import { useNotificationActions } from "./composables/useNotificationActions";

type StartupRecovery = {
  kind: string;
  title: string;
  message: string;
};

const props = defineProps<{
  startupRecovery?: StartupRecovery | null;
}>();

const deviceStore = useDeviceStore();

useNotificationActions();

onMounted(() => {
  if (props.startupRecovery) {
    return;
  }

  void deviceStore.hydrate();
});
</script>

<template>
  <main v-if="startupRecovery" class="recovery-shell">
    <section class="recovery-panel" role="alert" aria-live="assertive">
      <p class="recovery-kicker">Fini cannot open this database</p>
      <h1>{{ startupRecovery.title }}</h1>
      <p class="recovery-message">{{ startupRecovery.message }}</p>
    </section>
  </main>
  <div v-else class="app-shell">
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
  --content-bottom-inset: 10rem;
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

.recovery-shell {
  display: grid;
  min-height: 100vh;
  padding: 1rem;
  place-items: center;
  color: var(--color-base-content);
  background: var(--color-page-bg);
}

.recovery-panel {
  width: min(100%, 42rem);
  padding: 1.5rem;
  border: 1px solid color-mix(in srgb, var(--color-error) 35%, transparent);
  border-radius: 8px;
  background: var(--color-base-100);
  box-shadow: var(--shadow-sm);
}

.recovery-kicker {
  margin: 0 0 0.5rem;
  color: var(--color-error);
  font-size: 0.8125rem;
  font-weight: 700;
}

.recovery-panel h1 {
  margin: 0;
  font-size: 1.5rem;
  line-height: 1.2;
}

.recovery-message {
  margin: 1rem 0 0;
  white-space: pre-wrap;
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
  padding: 1rem 1rem calc(var(--content-bottom-inset) + env(safe-area-inset-bottom));
}

@media (max-width: 640px) {
  .nav { padding: 0.625rem 0.875rem; }
  .content { padding-inline: 0.75rem; }
}
</style>
