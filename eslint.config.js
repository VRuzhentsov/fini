// ESLint checks the JavaScript, TypeScript and Vue files with the recommended
// rule sets of ESLint, typescript-eslint and eslint-plugin-vue, and with the
// size limits of docs/code-style.md: 300 lines of code in a function, 2500
// lines in a file. Clippy (src-tauri/clippy.toml) checks the functions of the
// Rust code, and `npm run lint:file-size` checks the size of the Rust and
// Kotlin files.
import js from '@eslint/js';
import pluginVue from 'eslint-plugin-vue';
import globals from 'globals';
import tseslint from 'typescript-eslint';

// As clippy `too_many_lines`, a function counts its lines of code only. A file
// counts all of its lines, as `npm run lint:file-size` does.
const sizeRules = {
  'max-lines-per-function': ['error', { max: 300, skipBlankLines: true, skipComments: true }],
  'max-lines': ['error', { max: 2500 }],
};

export default [
  // Build output, and generated or vendored code.
  { ignores: ['dist/', 'tmp/', '**/target/', 'src-tauri/gen/', 'src-tauri/patches/', '**/test-results/', '**/playwright-report/'] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  ...pluginVue.configs['flat/recommended'],
  // The JavaScript files are Node scripts and config files.
  { files: ['**/*.{js,mjs,cjs}'], languageOptions: { globals: globals.node } },
  {
    files: ['**/*.vue'],
    languageOptions: { parserOptions: { parser: tseslint.parser } },
    rules: {
      // vue-tsc checks the names in the TypeScript of the components.
      'no-undef': 'off',
      // The templates keep the attributes of an element on one line.
      'vue/max-attributes-per-line': 'off',
      // The templates keep short element content on the line of its tags.
      'vue/singleline-html-element-content-newline': 'off',
      // The templates keep the content of a multiline element against its
      // tags; a new line there adds a space to the text of the element.
      'vue/multiline-html-element-content-newline': 'off',
      // The templates close void elements as `<input />`.
      'vue/html-self-closing': ['warn', { html: { void: 'always' } }],
    },
  },
  { files: ['**/*.{js,mjs,cjs,ts,mts,cts,vue}'], rules: sizeRules },
];
