// ESLint checks the size limits of docs/code-style.md in the JavaScript,
// TypeScript and Vue files: 300 lines of code in a function, 2500 lines in a
// file. Clippy (src-tauri/clippy.toml) checks the functions of the Rust code,
// and `npm run lint:file-size` checks the size of the Rust and Kotlin files.
import tseslint from 'typescript-eslint';
import vueParser from 'vue-eslint-parser';

// As clippy `too_many_lines`, a function counts its lines of code only. A file
// counts all of its lines, as `npm run lint:file-size` does.
const sizeRules = {
  'max-lines-per-function': ['error', { max: 300, skipBlankLines: true, skipComments: true }],
  'max-lines': ['error', { max: 2500 }],
};

export default [
  // Build output, and generated or vendored code.
  { ignores: ['dist/', 'tmp/', '**/target/', 'src-tauri/gen/', 'src-tauri/patches/', '**/test-results/', '**/playwright-report/'] },
  { files: ['**/*.{js,mjs,cjs}'], rules: sizeRules },
  // The plugin is loaded with no rules on, so that the `eslint-disable`
  // comments for its rules in the specs refer to rules that exist. These
  // comments disable a rule that is off, so ESLint does not report them.
  { linterOptions: { reportUnusedDisableDirectives: 'off' } },
  {
    files: ['**/*.{ts,mts,cts}'],
    plugins: { '@typescript-eslint': tseslint.plugin },
    languageOptions: { parser: tseslint.parser },
    rules: sizeRules,
  },
  {
    files: ['**/*.vue'],
    plugins: { '@typescript-eslint': tseslint.plugin },
    languageOptions: { parser: vueParser, parserOptions: { parser: tseslint.parser } },
    rules: sizeRules,
  },
];
