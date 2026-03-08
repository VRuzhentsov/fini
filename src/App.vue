<template>
  <div class="app-shell">
    <nav class="nav">
      <router-link to="/quests">Quests</router-link>
      <router-link to="/settings">Settings</router-link>
    </nav>
    <main class="content">
      <router-view />
    </main>
  </div>
</template>

<style>
:root {
  font-family: Inter, Avenir, Helvetica, Arial, sans-serif;
  font-size: 16px;
  line-height: 24px;
  font-weight: 400;
  color: #0f0f0f;
  background-color: #f6f6f6;
  font-synthesis: none;
  text-rendering: optimizeLegibility;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}

@media (prefers-color-scheme: dark) {
  :root {
    color: #f6f6f6;
    background-color: #2f2f2f;
  }
}

* {
  box-sizing: border-box;
  margin: 0;
  padding: 0;
}
</style>

<style scoped>
.app-shell {
  display: flex;
  flex-direction: column;
  height: 100vh;
}

.nav {
  display: flex;
  gap: 1rem;
  padding: 0.75rem 1.5rem;
  border-bottom: 1px solid rgba(128, 128, 128, 0.2);
}

.nav a {
  text-decoration: none;
  color: inherit;
  opacity: 0.6;
  font-weight: 500;
}

.nav a.router-link-active {
  opacity: 1;
}

.content {
  flex: 1;
  overflow: auto;
}
</style>
