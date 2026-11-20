// Pre-commit checks (husky runs lint-staged).
export default {
  // ESLint reports only the staged lines (eslint.staged.config.js).
  '*.{ts,mts,cts,tsx,vue,js,mjs,cjs}': 'eslint --config eslint.staged.config.js --max-warnings=0 --no-warn-ignored',
  // The Rust code is formatted and clippy-clean, so check the full crates, the
  // same as CI. A function, so that lint-staged does not add the file names.
  '*.rs': () => ['npm run lint:rust:fmt', 'npm run lint:rust:clippy'],
};
