import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import { fileURLToPath, URL } from "url";

export default defineConfig({
  plugins: [solid()],
  resolve: {
    alias: {
      "~": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  server: {
    port: 5173,
    strictPort: true,
  },
  build: {
    target: "esnext",
  },
  test: {
    environment: "jsdom",
    coverage: {
      provider: "v8",
      reporter: ["text", "lcov"],
      reportsDirectory: "./coverage",
    },
  },
});
