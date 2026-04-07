import js from "@eslint/js";

export default [
  js.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: {
      ecmaVersion: "latest",
      sourceType: "module",
    },
  },
  {
    ignores: ["dist/", "node_modules/", "src-tauri/"],
  },
];
