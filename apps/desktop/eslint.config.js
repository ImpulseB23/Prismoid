import js from "@eslint/js";
import tseslint from "typescript-eslint";

export default tseslint.config(
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
  },
  {
    ignores: ["dist/", "node_modules/", "src-tauri/"],
  },
);
