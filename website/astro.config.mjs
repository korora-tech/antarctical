import { defineConfig } from "astro/config";

export default defineConfig({
  site: "https://antarctical.korora.tech",
  trailingSlash: "never",
  // terms-of-service.html, served at /terms-of-service without a redirect.
  build: { format: "file" },
  // Keep the shared SVGs cacheable, including the favicon used on every page.
  vite: { build: { assetsInlineLimit: 0 } },
});
