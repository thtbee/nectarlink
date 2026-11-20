// SPDX-License-Identifier: GPL-3.0-or-later
import { cloudflareTest, readD1Migrations } from "@cloudflare/vitest-plugin";
import { defineConfig } from "vitest/config";

export default defineConfig(async () => ({
  plugins: [
    cloudflareTest({
      wrangler: { configPath: "./wrangler.toml" },
      miniflare: {
        bindings: { TEST_MIGRATIONS: await readD1Migrations("./migrations"), POW_BITS: "8" },
      },
    }),
  ],
  test: { setupFiles: ["./test/setup.ts"] },
}));
