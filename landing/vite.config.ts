import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// A static site (GitHub Pages or any host). VITE_BASE is the path it is served under,
// e.g. "/ForzaInfinity/" for https://<user>.github.io/ForzaInfinity/.
export default defineConfig({
  base: process.env.VITE_BASE ?? "/",
  plugins: [react()],
  server: { port: 5174 },
});
