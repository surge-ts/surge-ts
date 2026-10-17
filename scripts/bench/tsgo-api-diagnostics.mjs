#!/usr/bin/env node

// Prints tsgo's diagnostics for one project as JSON with what `--pretty false`
// leaves out: spans, message chains as data, and related information.
// compare-checkers.ts runs it beside the CLI and keeps the CLI's diagnostic set.
// ts-rust (`tsc-rs`) ports the same API, so it serves that checker too.
// Usage: node tsgo-api-diagnostics.mjs <tsconfig> [api package] [native tsc]

import path from 'node:path';
import process from 'node:process';

const [tsconfigArg = '', apiPackage = 'typescript', exe] = process.argv.slice(2);
const { API } = await import(`${apiPackage}/unstable/sync`);
const tsconfig = path.resolve(tsconfigArg);
const api = new API({ cwd: path.dirname(tsconfig), ...(exe ? { tsserverPath: exe } : {}) });
try {
  // 7.1 split the first snapshot out of updateSnapshot into createSnapshot.
  const params = { openProjects: [tsconfig] };
  const snapshot = 'createSnapshot' in api ? api.createSnapshot(params) : api.updateSnapshot(params);
  const project = snapshot.getProject(tsconfig) ?? snapshot.getProjects()[0];
  if (!project) throw new Error(`tsgo opened no project for ${tsconfig}`);
  const program = project.program;
  const seen = new Set();
  const diagnostics = [];
  for (const phase of [
    () => program.getConfigFileParsingDiagnostics(),
    () => program.getSyntacticDiagnostics(),
    () => program.getProgramDiagnostics(),
    () => program.getGlobalDiagnostics(),
    () => program.getBindDiagnostics(),
    () => program.getSemanticDiagnostics(),
    () => program.getDeclarationDiagnostics(),
  ]) {
    for (const diagnostic of phase()) {
      const key = JSON.stringify(diagnostic);
      if (seen.has(key)) continue;
      seen.add(key);
      diagnostics.push(diagnostic);
    }
  }
  process.stdout.write(`${JSON.stringify({ files: program.getSourceFileNames(), diagnostics })}\n`);
} finally {
  api.close();
}
