import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import path from "node:path";

export default defineConfig({
  plugins: [vue()],
  resolve: { alias: { "@": path.resolve(__dirname, "src") } },
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/target/**"] },
  },
  clearScreen: false,
  build: {
    target: "es2022",
    outDir: "dist",
    rollupOptions: {
      output: {
        manualChunks: {
          datepicker: ["@vuepic/vue-datepicker", "date-fns"],
          editor: ["codemirror", "@codemirror/autocomplete", "@codemirror/commands", "@codemirror/lang-sql", "@codemirror/language", "@codemirror/state", "@codemirror/view"],
          sql: ["sql-formatter"],
          vue: ["vue", "pinia"],
        },
      },
    },
  },
});
