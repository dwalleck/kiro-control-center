// S13 (AgentEditor.svelte) IPC + parent contract probe.
//
// Smallest factual question: what is the exact shape that
// AgentEditor.svelte must consume from (a) bindings.ts (Tauri IPC,
// post-A1 SaveOutcome) and (b) agent-list-helpers.ts (post-A3
// AgentsTabMode + headerLabel)?
//
// Reads the *generated* bindings.ts and the helpers module — the
// source of truth that S13 will actually import from at compile
// time. Pairs with oracle.ps1 which reads the upstream Rust
// source for an independent computation.
//
// Honest: this is text extraction with regexes, not a real TS
// parser. If the regex misses, the extracted field is missing
// from the report and the disagreement will show up vs. oracle.

import fs from "node:fs";

const root = process.argv[2] ?? ".";
const read = (p) => fs.readFileSync(`${root}/${p}`, "utf8");

const bindings = read("crates/kiro-control-center/src/lib/bindings.ts");
const helpers = read("crates/kiro-control-center/src/lib/agent-list-helpers.ts");

function grabType(src, name) {
  const re = new RegExp(`export type ${name} =([\\s\\S]*?);\\s*\\n\\s*\\n`);
  const m = src.match(re);
  return m ? `export type ${name} =${m[1]};` : `MISSING: ${name}`;
}

function grabCmd(src, name) {
  const re = new RegExp(`\\b${name}: \\([^\\n]*\\)[^\\n,]*`);
  const m = src.match(re);
  return m ? m[0] : `MISSING: ${name}`;
}

console.log("=== S13 contract probe (generated bindings + helpers) ===");
console.log("# 1. SaveOutcome wire shape (post-A1)\n");
console.log(grabType(bindings, "SaveOutcome"));

console.log("\n# 2. UserAgentRow (parent passes one in mode.row)\n");
console.log(grabType(bindings, "UserAgentRow"));

console.log("\n# 3. UserAgentLineage (controls keep-linked vs detach modal)\n");
console.log(grabType(bindings, "UserAgentLineage"));

console.log("\n# 4. CommandError + ErrorType (banner branching)\n");
console.log(grabType(bindings, "CommandError"));
console.log(grabType(bindings, "ErrorType"));

console.log("\n# 5. AgentsTabMode discriminated union (post-A3)\n");
console.log(grabType(helpers, "AgentsTabMode"));

console.log("\n# 6. headerLabel arms (consumed by editor topbar)\n");
const labelMatch = helpers.match(/case "(list|new|edit)":\s*\n\s*return [^\n]+/g);
console.log(labelMatch ? labelMatch.join("\n") : "MISSING: headerLabel cases");

console.log("\n# 7. The 5 user-agent command signatures S13 calls\n");
for (const cmd of [
  "listUserAgents",
  "createUserAgent",
  "saveUserAgent",
  "deleteUserAgent",
  "duplicateUserAgent",
]) {
  console.log(grabCmd(bindings, cmd));
}
