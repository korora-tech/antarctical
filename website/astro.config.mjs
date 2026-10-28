import { defineConfig } from "astro/config";

export default defineConfig({
  site: "https://antarctical.korora.tech",
  trailingSlash: "never",
  // terms-of-service.html, served at /terms-of-service without a redirect.
  build: { format: "file" },
});
