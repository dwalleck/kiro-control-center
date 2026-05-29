#!/usr/bin/env node
// Oracle for kiro-vgnw (slice 2): independent extraction of AGENT_TOOLS.
//
// Distinct from probe.py on three axes:
// - Runtime: Node.js vs CPython.
// - Parser: V8 reads the JS as actual JavaScript (not regex-extracted text).
// - Source-of-truth direction: probe parses the JS-as-text; oracle executes
//   the JS in a sandbox-ish shim and reads the resulting global.
//
// Loads agents-data.js by shimming `window = globalThis`, eval-ing it,
// then reading `globalThis.AGENT_TOOLS`. If the file evolves to use
// modern module syntax (`export const AGENT_TOOLS`), this approach would
// need adjustment — but the design bundle is frozen browser-script style.
//
// Emits the same normalized JSON shape as probe.py so the outputs can
// diff byte-for-byte.

import { readFileSync } from "node:fs";
import { argv, exit, stderr, stdout } from "node:process";
import vm from "node:vm";

function main() {
  if (argv.length !== 3) {
    stderr.write("usage: oracle.mjs <agents-data.js>\n");
    return 2;
  }
  const src = readFileSync(argv[2], "utf8");
  const sandbox = { window: {} };
  vm.createContext(sandbox);
  vm.runInContext(src, sandbox);

  const tools = sandbox.window.AGENT_TOOLS;
  if (!Array.isArray(tools)) {
    stderr.write("AGENT_TOOLS not found on window\n");
    return 2;
  }

  const categories = [...new Set(tools.map((t) => t.category))].sort();
  // Sort each tool's keys alphabetically (category, name, summary) so the
  // serialized output matches Python's json.dump(sort_keys=True) byte-for-byte.
  const toolsSorted = [...tools]
    .sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))
    .map((t) => ({ category: t.category, name: t.name, summary: t.summary }));

  // Top-level keys in alphabetical order: categories, category_count, tool_count, tools.
  const out = {
    categories,
    category_count: categories.length,
    tool_count: tools.length,
    tools: toolsSorted,
  };
  stdout.write(`${JSON.stringify(out, null, 2)}\n`);
  return 0;
}

exit(main());
