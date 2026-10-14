import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The fh2-server serves the build at /ui/ and the API at /api + /ws.
// In dev (npm run dev) Vite proxies those to the running server.
const server = process.env.FH2_SERVER ?? "http://127.0.0.1:8081";

export default defineConfig({
  base: "/ui/",
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      "/api": server,
      "/ws": { target: server.replace(/^http/, "ws"), ws: true },
    },
  },
});
