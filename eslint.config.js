// Full lint: `npm run lint`. The pre-commit hook uses eslint.staged.config.js,
// which reports only the staged lines.
import js from '@eslint/js';
import pluginVue from 'eslint-plugin-vue';
import { defineConfigWithVueTs, vueTsConfigs } from '@vue/eslint-config-typescript';
import globals from 'globals';

export default defineConfigWithVueTs(
  {
    name: 'fini/files-to-lint',
    files: ['**/*.{ts,mts,cts,tsx,vue}'],
  },
  {
    name: 'fini/files-to-ignore',
    ignores: [
      '**/dist/**',
      'src-tauri/**',
      'xtask/**',
      'ble-mock-broker/**',
      'tmp/**',
      'test-results/**',
      'playwright-report/**',
    ],
  },
  js.configs.recommended,
  pluginVue.configs['flat/recommended'],
  vueTsConfigs.recommended,
  {
    name: 'fini/node-scripts',
    files: ['**/*.{js,mjs,cjs}'],
    languageOptions: { globals: globals.node },
  },
);
