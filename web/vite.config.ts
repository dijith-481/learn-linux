import { defineConfig, lazyPlugins } from "vite-plus";
import solid from "vite-plugin-solid";

export default defineConfig({
  fmt: {},
  lint: {
    jsPlugins: [{ name: "vite-plus", specifier: "vite-plus/oxlint-plugin" }],
    rules: { "vite-plus/prefer-vite-plus-imports": "error" },
    options: { typeAware: true, typeCheck: true },
  },
  plugins: lazyPlugins(() => [solid()]),
  test: {
    environment: "node",
  },
  server: {
    port: 5173,
  },
  build: {
    target: "es2022",
  },
});
