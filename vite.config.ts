import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "path";

const host = process.env.TAURI_DEV_HOST;

// vendor chunk groups: heavy libs split off so the initial bundle stays small
function manualChunks(id: string): string | undefined {
  if (!id.includes("node_modules")) return undefined;

  if (id.includes("/recharts/") || id.includes("/d3-")) return "vendor-charts";
  if (id.includes("/@radix-ui/")) return "vendor-radix";
  if (id.includes("/lucide-react/")) return "vendor-icons";
  if (id.includes("/@tauri-apps/")) return "vendor-tauri";
  if (id.includes("/libphonenumber-js/")) return "vendor-phone";
  if (id.includes("/react-day-picker/") || id.includes("/date-fns/")) return "vendor-dates";
  if (id.includes("/embla-carousel")) return "vendor-carousel";
  if (id.includes("/react-hook-form/") || id.includes("/@hookform/") || id.includes("/zod/")) return "vendor-forms";
  if (id.includes("/cmdk/") || id.includes("/sonner/") || id.includes("/vaul/")) return "vendor-ui";
  if (id.includes("/react/") || id.includes("/react-dom/") || id.includes("/scheduler/")) return "vendor-react";

  return "vendor";
}

export default defineConfig(async () => ({
  plugins: [react()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  clearScreen: false,
  build: {
    chunkSizeWarningLimit: 1024,
    cssCodeSplit: true,
    rollupOptions: {
      output: {
        manualChunks,
        // randomized filenames: drop component/vendor names, use only content hash
        entryFileNames: "assets/[hash].js",
        chunkFileNames: "assets/[hash].js",
        assetFileNames: "assets/[hash][extname]",
      },
    },
  },
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
}));
