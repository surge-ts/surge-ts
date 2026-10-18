import { readFileSync } from 'node:fs';
import { sep } from 'node:path';

import { expect, test } from 'vitest';

import { surge, tempProject } from '../harness/cli.ts';
import { workspacePath } from '../harness/suite.ts';
import {
	codesOf,
	compatProject,
	diagnosticsOf,
	expectSameAsTsc,
	expectSameAsTscOnCompatProject,
	fingerprintsOf,
	linesOf,
	runCli,
	runCliJson,
	tsconfigPath,
} from './project-mode-helpers.ts';

const TSCONFIG = '{ "compilerOptions": {}, "include": ["src/**/*.ts"] }';
const ROOT_TS_TSCONFIG = '{ "include": ["*.ts"] }';

function srcProject(files: Record<string, string>): string {
	return tempProject({ 'tsconfig.json': TSCONFIG, ...files });
}

function workspaceVersion(): string {
	const manifest = readFileSync(workspacePath('Cargo.toml'), 'utf8');
	const section = manifest.slice(manifest.indexOf('[workspace.package]'));
	const version = /^version\s*=\s*"([^"]+)"/m.exec(section);
	if (version === null) throw new Error('no [workspace.package] version in Cargo.toml');
	return version[1];
}

const PACKAGE_IMPORTS = compatProject('package-imports');
const PACKAGE_IMPORTS_PROJECT = tsconfigPath(PACKAGE_IMPORTS);
const PARALLEL_ORDERING = compatProject('parallel-ordering-basic');
const PARALLEL_ORDERING_PROJECT = tsconfigPath(PARALLEL_ORDERING);

function packageImportsReport(): Promise<{ stdout: string; stderr: string }> {
	return runCli(['--project', PACKAGE_IMPORTS_PROJECT, '--compatReport'], PACKAGE_IMPORTS);
}

test('cli_compat_report_project_counts_by_code', async () => {
	const { stdout, stderr } = await packageImportsReport();

	expect(stderr).toBe('');
	expect(stdout).toContain('Compatibility report');
	expect(stdout).toContain('Files loaded: 1');
	expect(stdout).toContain('Diagnostics: 8');
	expect(stdout).toContain('TS2307  7');
	expect(stdout).toContain('TS2882  1');
});

test('cli_compat_report_project_counts_by_file', async () => {
	const { stdout, stderr } = await packageImportsReport();

	expect(stderr).toBe('');
	expect(stdout).toContain('src/index.ts  8');
});

test('cli_compat_report_includes_files_loaded', async () => {
	const { stdout, stderr } = await packageImportsReport();

	expect(stderr).toBe('');
	expect(stdout).toContain('Files loaded: 1');
});

test('cli_compat_report_includes_build_info', async () => {
	const parsed = await runCliJson(
		['--project', PACKAGE_IMPORTS_PROJECT, '--compatReport', '--format', 'json'],
		PACKAGE_IMPORTS,
	);

	expect(parsed.buildInfo.packageVersion).toBe(workspaceVersion());
	// The binary records `option_env!("PROFILE")`, which cargo sets only for
	// build scripts, so a plain build reports "unknown".
	expect(parsed.buildInfo.buildProfile).toBe(process.env.PROFILE ?? 'unknown');
	expect(typeof parsed.buildInfo.binaryPath).toBe('string');
	expect(typeof parsed.buildInfo.currentDir).toBe('string');
	expect(typeof parsed.buildInfo.workspaceRoot).toBe('string');
});

test('cli_project_reports_zero_source_files_explicitly', async () => {
	const root = tempProject({ 'tsconfig.json': '{ "include": ["src"] }' });
	const parsed = await runCliJson(['--project', tsconfigPath(root), '--format', 'json'], root);

	expect(codesOf(parsed)).toEqual(['surge::project-has-no-source-files']);
});

test('cli_compat_report_reports_visibility_warning_when_no_sources_load', async () => {
	const root = tempProject({ 'tsconfig.json': '{ "include": ["src"] }' });
	const parsed = await runCliJson(
		['--project', tsconfigPath(root), '--compatReport', '--format', 'json'],
		root,
	);

	expect(parsed.filesLoaded).toBe(0);
	expect(parsed.visibilityWarning).toBe('no source files were discovered for the project');
	expect(parsed.byCode[0].code).toBe('surge::project-has-no-source-files');
});

test('cli_compat_report_includes_parser_error_count', async () => {
	const root = srcProject({
		'src/a.ts': 'import { User from "./user";',
		'src/b.ts': 'let value: string = "ok";',
	});
	const { stdout, stderr } = await runCli(
		['--project', tsconfigPath(root), '--compatReport', '--diagnosticProfile', 'native'],
		root,
	);

	expect(stderr).toBe('');
	expect(stdout).toContain('Parser errors: 1');
	expect(stdout).toContain('TS1005');
});

// The native profile adds surge's own code for a duplicate type parameter,
// which the default tsc profile leaves to TS2300.
const NATIVE_ONLY_SOURCE = 'export type Pair<T, T> = [T, T];';

test('cli_project_tsc_profile_suppresses_custom_checker_diagnostics', async () => {
	const root = srcProject({ 'src/a.ts': NATIVE_ONLY_SOURCE });

	await expectSameAsTsc(root);

	const native = await runCliJson(
		['--project', tsconfigPath(root), '--format', 'json', '--diagnosticProfile', 'native'],
		root,
	);
	expect(codesOf(native).some((code) => code.startsWith('surge::'))).toBe(true);
});

test('cli_project_jobs_accept_one_two_and_four', async () => {
	for (const jobs of ['1', '2', '4']) {
		const parsed = await runCliJson(
			['--project', PARALLEL_ORDERING_PROJECT, '--format', 'json', '--jobs', jobs],
			PARALLEL_ORDERING,
		);
		expect(diagnosticsOf(parsed), `--jobs ${jobs}`).not.toHaveLength(0);
	}
});

test('cli_project_jobs_reject_zero_and_non_numeric', async () => {
	const zero = await surge(['--project', PARALLEL_ORDERING_PROJECT, '--jobs', '0'], {
		cwd: PARALLEL_ORDERING,
	});
	expect(zero.exitCode).not.toBe(0);
	expect(zero.stderr).toContain('--jobs must be greater than 0');

	const invalid = await surge(['--project', PARALLEL_ORDERING_PROJECT, '--jobs', 'not-a-number'], {
		cwd: PARALLEL_ORDERING,
	});
	expect(invalid.exitCode).not.toBe(0);
	expect(invalid.stderr).toContain('invalid value for --jobs');
});

test('cli_project_jobs_match_serial_json_diagnostics', async () => {
	const run = (extra: string[]) =>
		runCliJson(
			['--project', PARALLEL_ORDERING_PROJECT, '--format', 'json', ...extra],
			PARALLEL_ORDERING,
		);
	const serial = fingerprintsOf(await run([]));
	const jobs1 = fingerprintsOf(await run(['--jobs', '1']));
	const jobs4 = fingerprintsOf(await run(['--jobs', '4']));

	expect(jobs1).toEqual(serial);
	expect(jobs4).toEqual(jobs1);
});

test('cli_project_jobs_keep_native_profile_opt_in', async () => {
	const root = srcProject({ 'src/a.ts': NATIVE_ONLY_SOURCE });
	const native = await runCliJson(
		[
			'--project',
			tsconfigPath(root),
			'--format',
			'json',
			'--diagnosticProfile',
			'native',
			'--jobs',
			'4',
		],
		root,
	);

	expect(codesOf(native).some((code) => code.startsWith('surge::'))).toBe(true);
});

test('cli_compat_report_with_max_diagnostics_counts_all', async () => {
	const root = srcProject({
		'src/a.ts': 'let a: number = "a";',
		'src/b.ts': 'let b: number = "b";',
	});
	const { stdout, stderr } = await runCli(
		['--project', tsconfigPath(root), '--compatReport', '--maxDiagnostics', '1'],
		root,
	);

	expect(stderr).toBe('');
	expect(stdout).toContain('Diagnostics: 2');
	expect(stdout).toContain('TS2322  2');
	expect(stdout).toContain('Showing first 1 of 2 diagnostics.');
});

test('cli_compat_report_format_json_still_report_shape', async () => {
	const { stdout, stderr } = await runCli(
		['--project', PACKAGE_IMPORTS_PROJECT, '--compatReport', '--format', 'json'],
		PACKAGE_IMPORTS,
	);

	expect(stderr).toBe('');
	const parsed = JSON.parse(stdout);
	expect(parsed.rootDir).toBe(PACKAGE_IMPORTS.split(sep).join('/'));
	expect(parsed.filesLoaded).toBe(1);
	expect(parsed.diagnosticsTotal).toBe(8);
	expect(Array.isArray(parsed.byCode)).toBe(true);
	expect(Array.isArray(parsed.byFile)).toBe(true);
	expect(Array.isArray(parsed.diagnosticsByFileKind)).toBe(true);
	expect(Array.isArray(parsed.parserErrors)).toBe(true);
	expect(parsed.byCode[0].code).toBe('TS2307');
});

test('cli_max_diagnostics_limits_json_diagnostics_but_not_report_counts', async () => {
	const root = srcProject({
		'src/a.ts': 'let a: number = "a";',
		'src/b.ts': 'let b: number = "b";',
	});
	const project = tsconfigPath(root);

	const diagnostics = await runCli(
		['--project', project, '--format', 'json', '--maxDiagnostics', '1'],
		root,
	);
	expect(diagnostics.stderr).toBe('');
	expect(JSON.parse(diagnostics.stdout).diagnostics).toHaveLength(1);

	const report = await runCli(
		['--project', project, '--compatReport', '--format', 'json', '--maxDiagnostics', '1'],
		root,
	);
	expect(report.stderr).toBe('');
	const reportJson = JSON.parse(report.stdout);
	expect(reportJson.diagnosticsTotal).toBe(2);
	expect(reportJson.byCode[0].count).toBe(2);
});

test('cli_compat_report_json_matches_plain_json_diagnostics_total', async () => {
	const root = srcProject({
		'src/index.ts': 'export * from "./models";\nexport { Missing } from "./models";',
		'src/models/index.ts': 'export interface User { name: string; }',
		'src/pages/index.ts':
			'import { User } from "..";\nexport const currentUser: User = { name: "Ada" };',
	});
	const project = tsconfigPath(root);

	const plain = await runCliJson(['--project', project, '--format', 'json'], root);
	const report = await runCliJson(['--project', project, '--compatReport', '--format', 'json'], root);

	expect(plain.diagnostics).toHaveLength(1);
	expect(report.diagnosticsTotal).toBe(1);
	expect(report.byCode[0].code).toBe('TS2305');
});

test('cli_max_diagnostics_zero_or_invalid_rejected_or_pinned', async () => {
	const root = srcProject({ 'src/a.ts': 'let a: number = "a";' });
	const output = await surge(['--project', tsconfigPath(root), '--maxDiagnostics', '0'], {
		cwd: root,
	});

	expect(output.exitCode).not.toBe(0);
	expect(output.stderr).toContain('--maxDiagnostics must be greater than 0');
});

test('compat_project_package_imports_report_stable', async () => {
	const { stdout, stderr } = await packageImportsReport();

	expect(stderr).toBe('');
	expect(stdout).toContain('Compatibility report');
	expect(stdout).toContain('Diagnostics: 8');
	expect(stdout).toContain('TS2307  7');
	expect(stdout).toContain('TS2882  1');
});

for (const name of [
	'package_imports_line5_ts2882_matches_typescript',
	'package_imports_default_no_extra_ts2307_for_ts2882_case',
	'package_imports_other_package_imports_remain_ts2307_cli',
]) {
	test(name, async () => {
		await expectSameAsTscOnCompatProject('package-imports');
	});
}

test('package_imports_stub_external_modules_ts2882_policy_pinned_cli', async () => {
	const parsed = await runCliJson(
		['--project', PACKAGE_IMPORTS_PROJECT, '--stubExternalModules', '--format', 'json'],
		PACKAGE_IMPORTS,
	);

	expect(linesOf(parsed, 'TS2307')).toEqual([]);
	expect(linesOf(parsed, 'TS2882')).toEqual([]);
});

test('compat_project_module_forms_no_panic', async () => {
	const root = compatProject('module-forms');
	const { stdout, stderr } = await runCli(
		['--project', tsconfigPath(root), '--compatReport'],
		root,
	);

	expect(stderr).toBe('');
	expect(stdout).toContain('Compatibility report');
	expect(stdout).toContain('Diagnostics: 0');
	expect(stdout).not.toContain('surge::unsupported-module-syntax');
});

test('compat_project_relative_deep_valid', async () => {
	await expectSameAsTscOnCompatProject('relative-deep');
});

test('compat_project_private_types_valid', async () => {
	await expectSameAsTscOnCompatProject('private-types');
});

test('compat_project_report_counts_by_code', async () => {
	const { stdout, stderr } = await packageImportsReport();

	expect(stderr).toBe('');
	expect(stdout).toContain('TS2307  7');
	expect(stdout).toContain('TS2882  1');
});

test('compat_project_report_counts_by_file', async () => {
	const { stdout, stderr } = await packageImportsReport();

	expect(stderr).toBe('');
	expect(stdout).toContain('src/index.ts  8');
});

test('cli_stub_external_modules_project_suppresses_package_ts2307', async () => {
	const root = tempProject({
		'tsconfig.json': ROOT_TS_TSCONFIG,
		'index.ts': 'import { useState } from "react";',
	});
	await expectSameAsTsc(root);

	const { stdout, stderr } = await runCli(
		['--project', tsconfigPath(root), '--stubExternalModules'],
		root,
	);
	expect(stdout).not.toContain('TS2307');
	expect(stderr).toBe('');
});

test('cli_stub_external_modules_project_keeps_relative_ts2307', async () => {
	const root = tempProject({
		'tsconfig.json': ROOT_TS_TSCONFIG,
		'index.ts': 'import { X } from "./missing";',
	});
	const { stdout, stderr } = await runCli(
		['--project', tsconfigPath(root), '--stubExternalModules'],
		root,
	);

	expect(stdout).toContain('TS2307');
	expect(stderr).toBe('');
});

test('cli_stub_external_modules_single_file_ignore_config_suppresses_package_ts2307', async () => {
	const root = tempProject({ 'index.ts': 'import { useState } from "react";' });
	await expectSameAsTsc(root, ['--ignoreConfig', 'index.ts']);

	const { stdout } = await runCli(['--ignoreConfig', 'index.ts', '--stubExternalModules'], root);
	expect(stdout).not.toContain('TS2307');
});

test('cli_stub_external_modules_does_not_affect_ts5112', async () => {
	const root = tempProject({ 'tsconfig.json': ROOT_TS_TSCONFIG, 'index.ts': 'let x = 1;' });
	// Run from the project directory so the CLI detects tsconfig.json itself.
	const output = await surge(['index.ts', '--stubExternalModules'], { cwd: root });

	expect(output.stdout).toContain('TS5112');
});

test('cli_stub_external_modules_compat_report', async () => {
	const root = tempProject({
		'tsconfig.json': ROOT_TS_TSCONFIG,
		'index.ts': 'import { useState } from "react"; import { create } from "zustand";',
	});
	const project = tsconfigPath(root);

	const report = await runCli(['--project', project, '--compatReport'], root);
	expect(report.stdout).toContain('External module stubs: 2');
	expect(report.stdout).toContain('TS2307');
	expect(report.stdout).toContain('By code:');

	const stubbed = await runCli(
		['--project', project, '--compatReport', '--stubExternalModules'],
		root,
	);
	expect(stubbed.stdout).toContain('External module stubs: 2');
	expect(stubbed.stdout).not.toContain('error[TS2307]');
});

test('cli_default_external_import_reports_ts2307_no_cascade', async () => {
	const root = tempProject({
		'tsconfig.json': ROOT_TS_TSCONFIG,
		'index.ts': 'import * as Zustand from "zustand"; let x = Zustand.create;',
	});
	await expectSameAsTsc(root);
});

test('cli_external_namespace_property_access_no_cascade', async () => {
	const root = tempProject({
		'index.ts': 'import * as Zustand from "zustand"; let store = Zustand.createStore;',
	});
	await expectSameAsTsc(root, ['--ignoreConfig', 'index.ts']);
});

test('compat_report_external_module_stubs_json', async () => {
	const root = tempProject({
		'tsconfig.json': ROOT_TS_TSCONFIG,
		'index.ts': 'import { useState } from "react"; import { create } from "zustand";',
	});
	const { stdout } = await runCli(
		['--project', tsconfigPath(root), '--compatReport', '--format', 'json'],
		root,
	);

	const stubs = JSON.parse(stdout).externalModuleStubs;
	// Both `react` and `zustand` are non-relative references (total) and neither
	// resolves in this fixture, so both are unresolved and none resolved.
	expect(stubs.total).toBe(2);
	expect(stubs.unresolved).toBe(2);
	expect(stubs.resolved).toBe(0);
});
