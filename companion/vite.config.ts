import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri 期望前端跑在固定端口，dev 时由 Tauri 通过 devUrl 加载。
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
});
