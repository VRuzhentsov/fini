// Pre-commit checks (husky runs lint-staged).
export default {
  // ESLint reports only the staged lines (eslint.staged.config.js).
  '*.{ts,mts,cts,tsx,vue,js,mjs,cjs}': 'eslint --config eslint.staged.config.js --max-warnings=0 --no-warn-ignored',
};
