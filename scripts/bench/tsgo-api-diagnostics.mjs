#!/usr/bin/env node

// Prints tsgo's diagnostics for one project as JSON with what `--pretty false`
// leaves out: spans, message chains as data, and related information.
// compare-checkers.ts runs it beside the CLI and keeps the CLI's diagnostic set.
// Usage: node tsgo-api-diagnostics.mjs <tsconfig>

import path from 'node:path';
import process from 'node:process';

import { API } from 'typescript/unstable/sync';

const tsconfig = path.resolve(process.argv[2] ?? '');
const api = new API({ cwd: path.dirname(tsconfig) });
try {
  const snapshot = api.updateSnapshot({ openProjects: [tsconfig] });
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
