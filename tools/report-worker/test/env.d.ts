// SPDX-License-Identifier: GPL-3.0-or-later
import type { D1Migration } from "@cloudflare/vitest-plugin";
import type { Env as WorkerEnv } from "../src/index";

declare global {
  namespace Cloudflare {
    interface Env extends WorkerEnv {
      TEST_MIGRATIONS: D1Migration[];
    }
  }
}
