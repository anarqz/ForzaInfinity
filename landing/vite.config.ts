import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";

// A static site (GitHub Pages or any host). VITE_BASE is the path it is served under,
// e.g. "/ForzaInfinity/" for https://<user>.github.io/ForzaInfinity/.
// VITE_SITE_URL is its full address, for the sharing tags in index.html (social sites need
// absolute links); it must end in "/".
const siteUrl = process.env.VITE_SITE_URL ?? "https://anarqz.github.io/ForzaInfinity/";

function siteUrlTags(): Plugin {
  return {
    name: "site-url",
    transformIndexHtml: (html) => html.replaceAll("%SITE_URL%", siteUrl),
  };
}

export default defineConfig({
  base: process.env.VITE_BASE ?? "/",
  plugins: [react(), siteUrlTags()],
  server: { port: 5174 },
});
