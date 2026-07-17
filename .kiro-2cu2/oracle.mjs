#!/usr/bin/env node
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import svelteCompiler from "../crates/kiro-control-center/node_modules/svelte/compiler/index.js";
import { walk } from "../crates/kiro-control-center/node_modules/estree-walker/src/index.js";
import * as ts from "../crates/kiro-control-center/node_modules/typescript/lib/typescript.js";
const { parse } = svelteCompiler;

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const browseSource = readFileSync(
  join(root, "crates/kiro-control-center/src/lib/components/BrowseTab.svelte"),
  "utf8",
);
const browseAst = parse(browseSource, { filename: "BrowseTab.svelte", modern: true });
let drawerAcceptMcp;
let wholeAcceptMcp;
walk(browseAst.instance, {
  enter(node) {
    if (
      node.type === "CallExpression" &&
      node.callee?.type === "MemberExpression" &&
      node.callee.object?.name === "commands" &&
      node.callee.property?.name === "installAgents"
    ) {
      drawerAcceptMcp = node.arguments[4];
    }
    if (node.type === "Property" && node.key?.name === "acceptMcp") {
      wholeAcceptMcp = node.value;
    }
  },
});

const bindingsSource = readFileSync(
  join(root, "crates/kiro-control-center/src/lib/bindings.ts"),
  "utf8",
);
const bindingsAst = ts.createSourceFile(
  "bindings.ts",
  bindingsSource,
  ts.ScriptTarget.Latest,
  true,
  ts.ScriptKind.TS,
);
let agentItemInfo;
let warningPresent = false;
ts.forEachChild(bindingsAst, function visit(node) {
  if (ts.isTypeAliasDeclaration(node) && node.name.text === "AgentItemInfo") {
    agentItemInfo = node;
  }
  if (ts.isStringLiteral(node) && node.text === "mcp_servers_require_opt_in") {
    warningPresent = true;
  }
  ts.forEachChild(node, visit);
});
if (!agentItemInfo || !drawerAcceptMcp || !wholeAcceptMcp) {
  throw new Error("expected MCP call-site and AgentItemInfo AST nodes were not found");
}
function classify(node) {
  return node.type === "Literal" && typeof node.value === "boolean"
    ? String(node.value)
    : "dynamic";
}
const fields = agentItemInfo.type.members.map((member) => member.name.text);
console.log(JSON.stringify({
  agent_catalog_fields: fields,
  catalog_has_preinstall_mcp_signal: fields.some((field) => field.includes("mcp")),
  drawer_accept_mcp: classify(drawerAcceptMcp),
  post_install_warning_available: warningPresent,
  whole_plugin_accept_mcp: classify(wholeAcceptMcp),
}, null, 2));
