#!/usr/bin/env node
// SPDX-License-Identifier: GPL-3.0-or-later

import { main } from '../src/main.js';

main(process.argv.slice(2)).then(
  (status) => {
    process.exitCode = status;
  },
  (err) => {
    process.stderr.write(`gource-online: ${err instanceof Error ? (err.stack ?? err.message) : String(err)}\n`);
    process.exitCode = 1;
  },
);
