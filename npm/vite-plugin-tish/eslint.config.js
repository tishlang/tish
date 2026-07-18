// ESLint flat config for @tishlang/vite-plugin-tish (published ESM, Node >= 22).
// Declares the modern ES target + Node globals so static analysis does not misfire
// on block-scoped vars / nullish-coalescing / null (this is not ES5), while keeping
// the core rules that catch real bugs.
export default [
  {
    files: ["**/*.js"],
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: "module",
      globals: {
        process: "readonly",
        console: "readonly",
        URL: "readonly",
        Buffer: "readonly",
      },
    },
    rules: {
      "no-unused-vars": "warn",
      "no-undef": "error",
      "no-dupe-keys": "error",
      "no-dupe-args": "error",
      "no-unreachable": "error",
      "no-cond-assign": "error",
      "use-isnan": "error",
      "valid-typeof": "error",
    },
  },
];
