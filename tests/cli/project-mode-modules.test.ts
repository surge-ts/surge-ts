import { expect, test } from 'vitest';

import { tempProject } from '../harness/cli.ts';
import { expectSameAsTsc, runCli, tsconfigPath } from './project-mode-helpers.ts';

const TSCONFIG = '{ "compilerOptions": {}, "include": ["src/**/*.ts"] }';

function srcProject(files: Record<string, string>): string {
	return tempProject({ 'tsconfig.json': TSCONFIG, ...files });
}

async function runNative(root: string): Promise<{ stdout: string; stderr: string }> {
	return runCli(['--project', tsconfigPath(root), '--diagnosticProfile', 'native'], root);
}

async function runProject(
	root: string,
	extra: string[] = [],
): Promise<{ stdout: string; stderr: string }> {
	return runCli(['--project', tsconfigPath(root), ...extra], root);
}

test('project_mode_cross_file_interface_valid', async () => {
	const root = srcProject({
		'src/a.ts': 'interface User { name: string; }',
		'src/b.ts': 'let user: User = { name: "Ada" };',
	});
	const { stdout, stderr } = await runNative(root);

	expect(stderr).toBe('');
	expect(stdout.trim()).toBe('');
});

test('project_mode_cross_file_interface_mismatch', async () => {
	const root = srcProject({
		'src/a.ts': 'interface User { name: string; }',
		'src/b.ts': 'let user: User = { name: 123 };',
	});
	const { stdout, stderr } = await runNative(root);

	expect(stderr).toBe('');
	expect(stdout).toContain('src/b.ts');
	expect(stdout).toContain('TS2322');
	expect(stdout).not.toContain('src/a.ts\nerror[TS2322]');
});

test('project_mode_cross_file_type_alias_valid', async () => {
	const root = srcProject({
		'src/a.ts': 'type Name = string;',
		'src/b.ts': 'let value: Name = "Ada";',
	});
	const { stdout, stderr } = await runNative(root);

	expect(stderr).toBe('');
	expect(stdout.trim()).toBe('');
});

test('project_mode_uses_program_checker_for_cross_file_type_alias', async () => {
	const root = srcProject({
		'src/a.ts': 'type Name = string;',
		'src/b.ts': 'let value: Name = 123;',
	});
	const { stdout, stderr } = await runNative(root);

	expect(stderr).toBe('');
	expect(stdout).toContain('src/b.ts');
	expect(stdout).toContain('TS2322');
	expect(stdout).not.toContain('src/a.ts\nerror[TS2322]');
});

test('project_mode_cross_file_function_valid', async () => {
	const root = srcProject({
		'src/a.ts': 'function getName(): string { return "Ada"; }',
		'src/b.ts': 'let value: string = getName();',
	});
	const { stdout, stderr } = await runNative(root);

	expect(stderr).toBe('');
	expect(stdout.trim()).toBe('');
});

for (const name of [
	'project_mode_uses_program_checker_for_cross_file_function',
	'project_mode_cross_file_function_return_mismatch',
]) {
	test(name, async () => {
		const root = srcProject({
			'src/a.ts': 'function getName(): string { return "Ada"; }',
			'src/b.ts': 'let value: number = getName();',
		});
		const { stdout, stderr } = await runNative(root);

		expect(stderr).toBe('');
		expect(stdout).toContain('src/b.ts');
		expect(stdout).toContain('TS2322');
		expect(stdout).not.toContain('src/a.ts\nerror[TS2322]');
	});
}

test('project_mode_diagnostics_grouped_by_file', async () => {
	const root = srcProject({
		'src/a.ts': 'let a: number = "x";',
		'src/b.ts': 'let b: number = "y";',
	});
	const { stdout, stderr } = await runProject(root);

	expect(stderr).toBe('');
	const aIndex = stdout.indexOf('src/a.ts');
	const bIndex = stdout.indexOf('src/b.ts');
	expect(aIndex, 'expected a.ts block').toBeGreaterThanOrEqual(0);
	expect(bIndex, 'expected b.ts block').toBeGreaterThanOrEqual(0);
	expect(aIndex).toBeLessThan(bIndex);
});

test('project_mode_top_level_variable_not_shared_policy', async () => {
	const root = srcProject({
		'src/a.ts': 'let greeting = "Ada";',
		'src/b.ts': 'let value: string = greeting;',
	});
	await expectSameAsTsc(root);
});

test('project_mode_parser_diagnostic_grouped_by_file', async () => {
	const root = srcProject({
		'src/a.ts': 'let value: string | = "bad";',
		'src/b.ts': 'let ok: string = "ok";',
	});
	const { stdout, stderr } = await runNative(root);

	expect(stderr).toBe('');
	expect(stdout).toContain('src/a.ts');
	expect(stdout).not.toContain('src/b.ts\nerror[surge::parser-error]');
});

test('project_mode_single_file_position_arg_still_works', async () => {
	const root = tempProject({ 'index.ts': 'let value: string = 123;' });
	await expectSameAsTsc(root, ['index.ts']);
});

test('project_mode_exported_interface_not_global_yet', async () => {
	const root = srcProject({
		'src/a.ts': 'export interface User { name: string; }',
		'src/b.ts': 'let user: User = { name: "Ada" };',
	});
	await expectSameAsTsc(root);
});

test('project_mode_import_named_unresolved_until_resolution_phase', async () => {
	const root = srcProject({
		'src/user.ts': 'export interface User { name: string; }',
		'src/a.ts': 'import { User } from "./user";\nlet user: User = { name: "Ada" };',
	});
	await expectSameAsTsc(root);
});

test('project_mode_import_side_effect_valid', async () => {
	const root = srcProject({
		'src/setup.ts': 'export {};',
		'src/a.ts': 'import "./setup";\nlet value: string = "ok";',
	});
	await expectSameAsTsc(root);
});

test('project_mode_empty_export_marks_module_current_policy', async () => {
	const root = srcProject({ 'src/a.ts': 'export {};\nlet value: string = "ok";' });
	await expectSameAsTsc(root);
});

test('project_mode_single_file_positional_export_valid', async () => {
	const root = tempProject({
		'index.ts': 'export interface User { name: string; }\nlet user: User = { name: "Ada" };',
	});
	await expectSameAsTsc(root, ['index.ts']);
});

test('project_mode_single_file_positional_does_not_resolve_external_files', async () => {
	const root = tempProject({
		'index.ts': 'import { User } from "./user";\nlet user: User = { name: "Ada" };',
		'user.ts': 'export interface User { name: string; }',
	});
	await expectSameAsTsc(root, ['index.ts']);
});

test('project_mode_import_named_unresolved_grouped_by_file', async () => {
	const root = srcProject({
		'src/a.ts': 'import { User } from "./user";\nlet user: User = { name: "Ada" };',
		'src/b.ts': 'let value: string = "ok";',
	});
	await expectSameAsTsc(root);
});

test('project_mode_import_type_named_unresolved_grouped_by_file', async () => {
	const root = srcProject({
		'src/a.ts': 'import type { User } from "./user";\nlet user: User = { name: "Ada" };',
		'src/b.ts': 'let value: string = "ok";',
	});
	await expectSameAsTsc(root);
});

test('project_mode_relative_interface_import_valid', async () => {
	const root = srcProject({
		'src/user.ts': 'export interface User { name: string; }',
		'src/index.ts': 'import { User } from "./user";\nlet user: User = { name: "Ada" };',
	});
	await expectSameAsTsc(root);
});

test('project_mode_side_effect_import_script_file_valid', async () => {
	const root = srcProject({
		'src/setup.ts': 'let initialized: boolean = true;',
		'src/index.ts': 'import "./setup";\nlet value: string = "ok";',
	});
	await expectSameAsTsc(root);
});

test('project_mode_named_import_from_script_file_reports_missing_export', async () => {
	const root = srcProject({
		'src/setup.ts': 'let value = 1;',
		'src/index.ts': 'import { value } from "./setup";',
	});
	await expectSameAsTsc(root);
});

test('project_mode_relative_type_alias_import_valid', async () => {
	const root = srcProject({
		'src/user.ts': 'export type UserId = string;',
		'src/index.ts': 'import type { UserId } from "./user";\nlet id: UserId = "u1";',
	});
	await expectSameAsTsc(root);
});

test('project_mode_default_import_cross_file_valid', async () => {
	const root = srcProject({
		'src/user.ts': 'export default function getName(): string { return "Ada"; }',
		'src/index.ts': 'import getName from "./user";\nlet value: string = getName();',
	});
	await expectSameAsTsc(root);
});

test('project_mode_namespace_import_cross_file_valid', async () => {
	const root = srcProject({
		'src/user.ts': 'export const version: number = 1;',
		'src/index.ts': 'import * as user from "./user";\nlet value: number = user.version;',
	});
	await expectSameAsTsc(root);
});

test('project_mode_star_re_export_missing_module_no_consumer_cascade', async () => {
	const root = srcProject({
		'src/index.ts': 'export * from "./missing";',
		'src/app.ts': 'import { User } from "./index";\nlet value = User;',
	});
	await expectSameAsTsc(root);
});

test('project_mode_regular_type_export_value_usage_unresolved', async () => {
	const root = srcProject({
		'src/user.ts': 'export interface User { name: string; }',
		'src/index.ts': 'import { User } from "./user";\nlet value = User;',
	});
	await expectSameAsTsc(root);
});

test('project_mode_regular_value_export_used_as_type_reports_ts2749', async () => {
	const root = srcProject({
		'src/user.ts': 'export const User: string = "Ada";',
		'src/index.ts': 'import { User } from "./user";\nlet value: User = "Ada";',
	});
	await expectSameAsTsc(root);
});

test('project_mode_relative_function_import_valid', async () => {
	const root = srcProject({
		'src/user.ts': 'export function getName(): string { return "Ada"; }',
		'src/index.ts': 'import { getName } from "./user";\nlet name: string = getName();',
	});
	await expectSameAsTsc(root);
});

test('project_mode_show_spans_module_missing_export', async () => {
	const root = srcProject({
		'src/user.ts': 'export interface User { name: string; }',
		'src/index.ts': 'import { Missing } from "./user";',
	});
	const { stdout, stderr } = await runProject(root, ['--showSpans']);

	expect(stderr).toBe('');
	expect(stdout).toContain('TS2305');
	expect(stdout).toContain('start=');
	expect(stdout).toContain('end=');
});

for (const name of [
	'project_mode_show_spans_module_missing_relative',
	'project_mode_show_spans_relative_import_error',
]) {
	test(name, async () => {
		const root = srcProject({ 'src/index.ts': 'import { User } from "./missing";' });
		const { stdout, stderr } = await runProject(root, ['--showSpans']);

		expect(stderr).toBe('');
		expect(stdout).toContain('TS2307');
		expect(stdout).toContain('start=');
		expect(stdout).toContain('end=');
	});
}

test('project_mode_relative_variable_import_valid', async () => {
	const root = srcProject({
		'src/user.ts': 'export const version: string = "1";',
		'src/index.ts': 'import { version } from "./user";\nlet current: string = version;',
	});
	await expectSameAsTsc(root);
});

test('project_mode_relative_missing_export_grouped_by_importer_file', async () => {
	const root = srcProject({
		'src/user.ts': 'export interface User { name: string; }',
		'src/index.ts': 'import { Missing } from "./user";\nlet value: Missing = "x";',
		'src/other.ts': 'let value: string = "ok";',
	});
	await expectSameAsTsc(root);
});

test('project_mode_relative_export_declaration_error_grouped_by_exporter_file', async () => {
	const root = srcProject({
		'src/user.ts': 'export type Name = Missing;',
		'src/index.ts': 'import { Name } from "./user";\nlet value: Name = "Ada";',
	});
	await expectSameAsTsc(root);
});

test('project_mode_export_empty_valid', async () => {
	const root = srcProject({ 'src/a.ts': 'export {};\nlet value: string = "ok";' });
	await expectSameAsTsc(root);
});

test('project_mode_exported_interface_same_file_valid', async () => {
	const root = srcProject({
		'src/a.ts': 'export interface User { name: string; }\nlet user: User = { name: "Ada" };',
	});
	await expectSameAsTsc(root);
});

test('project_mode_exported_interface_not_global', async () => {
	const root = srcProject({
		'src/a.ts': 'export interface User { name: string; }',
		'src/b.ts': 'let user: User = { name: "Ada" };',
	});
	await expectSameAsTsc(root);
});

test('project_mode_module_file_does_not_see_script_global_current_policy', async () => {
	const root = srcProject({
		'src/a.ts': 'interface User { name: string; }',
		'src/b.ts': 'export {};\nlet user: User = { name: "Ada" };',
	});
	await expectSameAsTsc(root);
});

test('project_mode_script_files_still_share_global_interface', async () => {
	const root = srcProject({
		'src/a.ts': 'interface User { name: string; }',
		'src/b.ts': 'let user: User = { name: "Ada" };',
	});
	await expectSameAsTsc(root);
});

for (const [name, source] of [
	['project_mode_malformed_import_parser_error_grouped_by_file', 'import { User from "./user";'],
	['project_mode_malformed_export_parser_error_grouped_by_file', 'export { User;'],
]) {
	test(name, async () => {
		const root = srcProject({ 'src/a.ts': source, 'src/b.ts': 'let value: string = "ok";' });
		await expectSameAsTsc(root);
	});
}

test('project_mode_single_file_positional_module_syntax_valid', async () => {
	const root = tempProject({
		'index.ts': 'export interface User { name: string; }\nlet user: User = { name: "Ada" };',
	});
	await expectSameAsTsc(root, ['index.ts']);
});

for (const name of ['cli_show_spans_single_file_includes_start_end', 'cli_show_spans_still_works']) {
	test(name, async () => {
		const root = tempProject({ 'index.ts': 'let value: number = "a";' });
		const { stdout, stderr } = await runCli(['--showSpans', 'index.ts'], root);

		expect(stderr).toBe('');
		expect(stdout).toContain('TS2322');
		expect(stdout).toContain('start=');
		expect(stdout).toContain('end=');
	});
}

test('cli_show_spans_single_file_normal_output_unchanged_without_flag', async () => {
	const root = tempProject({ 'index.ts': 'let value: number = "a";' });
	const { stdout, stderr } = await runCli(['index.ts'], root);

	expect(stderr).toBe('');
	expect(stdout).toContain('TS2322');
	expect(stdout).not.toContain('start=');
	expect(stdout).not.toContain('end=');
});

test('cli_show_spans_project_mode_groups_by_file_if_supported', async () => {
	const root = srcProject({ 'src/index.ts': 'let value: number = "a";' });
	const { stdout, stderr } = await runProject(root, ['--showSpans']);

	expect(stderr).toBe('');
	expect(stdout).toContain('src/index.ts');
	expect(stdout).toContain('TS2322');
	expect(stdout).toContain('start=');
});

test('cli_show_spans_show_config_still_exits_successfully', async () => {
	const root = srcProject({ 'src/index.ts': 'let value: number = "a";' });
	const { stdout, stderr } = await runProject(root, ['--showSpans', '--showConfig']);

	expect(stderr).toBe('');
	expect(stdout).toContain('"compilerOptions"');
	expect(stdout).not.toContain('start=');
});

test('cli_show_config_still_exits_successfully', async () => {
	const root = srcProject({ 'src/index.ts': 'let value: number = "a";' });
	const { stdout, stderr } = await runProject(root, ['--showConfig']);

	expect(stderr).toBe('');
	expect(stdout).toContain('"compilerOptions"');
});

test('cli_project_normal_output_unchanged_without_compat_report', async () => {
	const root = srcProject({ 'src/index.ts': 'let value: number = "a";' });
	const { stdout, stderr } = await runProject(root);

	expect(stderr).toBe('');
	expect(stdout).toContain('TS2322');
	expect(stdout).not.toContain('Compatibility report');
});

test('project_mode_non_relative_import_grouped_by_importer_file', async () => {
	const root = srcProject({
		'src/a.ts': 'import { User } from "pkg";\nlet user: User = { name: 123 };',
		'src/b.ts': 'let value: string = "ok";',
	});
	await expectSameAsTsc(root);
});

test('cli_max_diagnostics_limits_rendered_output', async () => {
	const root = srcProject({
		'src/a.ts': 'let a: number = "a";',
		'src/b.ts': 'let b: number = "b";',
	});
	const { stdout, stderr } = await runProject(root, ['--maxDiagnostics', '1']);

	expect(stderr).toBe('');
	expect(stdout).toContain('src/a.ts');
	expect(stdout).toContain('Showing first 1 of 2 diagnostics.');
	expect(stdout).not.toContain('src/b.ts');
});
