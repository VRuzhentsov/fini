// Pre-commit lint (lint-staged): the rules of eslint.config.js, but
// eslint-plugin-diff reports only the lines that are staged.
import diff from 'eslint-plugin-diff';
import pluginVue from 'eslint-plugin-vue';
import base from './eslint.config.js';

export default [
  ...base,
  ...diff.configs['flat/staged'].map((config) => ({ ...config, ignores: ['**/*.vue'] })),
  {
    name: 'fini/staged-vue',
    files: ['**/*.vue'],
    processor: diff.composeProcessor(pluginVue.processors.vue, 'staged'),
  },
];
