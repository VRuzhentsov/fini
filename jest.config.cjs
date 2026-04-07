module.exports = {
  testEnvironment: "jsdom",
  roots: ["<rootDir>/src/spec"],
  testMatch: ["**/*.spec.ts"],
  moduleFileExtensions: ["ts", "js", "json", "vue"],
  transform: {
    "^.+\\.vue$": "@vue/vue3-jest",
    "^.+\\.[tj]s$": [
      "ts-jest",
      {
        tsconfig: "<rootDir>/tsconfig.jest.json",
      },
    ],
  },
  moduleNameMapper: {
    "^@/(.*)$": "<rootDir>/src/$1",
    "^@vue/test-utils$": "<rootDir>/node_modules/@vue/test-utils/dist/vue-test-utils.cjs.js",
    "\\.(css|less|sass|scss)$": "identity-obj-proxy",
  },
  setupFilesAfterEnv: ["<rootDir>/src/spec/jest.setup.ts"],
};
