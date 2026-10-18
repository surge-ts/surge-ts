import { expect, test } from 'vitest';

import { tempProject } from '../harness/cli.ts';
import {
	expectSameAsTsc,
	expectSameAsTscOnCompatProject,
	runCli,
	tsconfigPath,
} from './project-mode-helpers.ts';

const MATH_AND_TRANSPORT = `
        const n = Math.max(1, 2);
        const transport: AuthenticatorTransport = "usb";
        `;

const UNTYPED_PARAMETER = 'function f(value): string { return "ok"; }';

test('project_mode_generated_default_libs_visible_by_default', async () => {
	const root = tempProject({
		'tsconfig.json': `
        {
          "compilerOptions": {},
          "include": ["src/**/*.ts"]
        }
        `,
		'src/index.ts': MATH_AND_TRANSPORT,
	});
	await expectSameAsTsc(root);
});

test('project_mode_untyped_javascript_import_reports_ts7016', async () => {
	const root = tempProject({
		'tsconfig.json': `
        {
          "compilerOptions": { "strict": true },
          "include": ["src/**/*.ts"]
        }
        `,
		'src/helper.mjs': 'export const value = 1;\n',
		'src/index.ts': 'import helper from "./helper.mjs";\nexport const used = helper;\n',
	});
	await expectSameAsTsc(root);
});

test('project_mode_untyped_javascript_import_is_silent_without_no_implicit_any', async () => {
	const root = tempProject({
		'tsconfig.json': `
        {
          "compilerOptions": { "noImplicitAny": false },
          "include": ["src/**/*.ts"]
        }
        `,
		'src/helper.mjs': 'export const value = 1;\n',
		'src/index.ts': 'import helper from "./helper.mjs";\nexport const used = helper;\n',
	});
	await expectSameAsTsc(root);
});

test('project_mode_lib_option_es_only_skips_dom_generated_libs', async () => {
	const root = tempProject({
		'tsconfig.json': `
        {
          "compilerOptions": {
            "lib": ["ES2022"]
          },
          "include": ["src/**/*.ts"]
        }
        `,
		'src/index.ts': MATH_AND_TRANSPORT,
	});
	await expectSameAsTsc(root);
});

// These fixtures exercise the real `lib*.d.ts` graph. The bundled snapshot
// ships with the binary, so they run unconditionally.
const LIB_FIXTURES: [string, string][] = [
	// `values.map(v => v.toString())` must infer `string[]`, so assigning it to
	// `number[]` is the only error.
	['project_mode_physical_libs_resolve_array_callback_return', 'physical-lib-es-array-basic'],
	['project_mode_physical_libs_resolve_map_generic_methods', 'physical-lib-es-map-set-basic'],
	['project_mode_physical_libs_resolve_index_signature', 'physical-lib-index-signature-basic'],
	// A `Promise<void>` executor (contextual or explicit `<void>`) may call
	// `resolve()` with no argument: the constructor infers `T = void` from the
	// expected type, so the executor's `resolve` parameter is optional.
	[
		'project_mode_physical_libs_new_promise_void_executor',
		'physical-lib-new-promise-executor-basic',
	],
	// `Required<Omit<T, K>> & Pick<T, K>` (ky's `InternalRetryOptions`) must
	// resolve: `Required` makes each property required while keeping an explicit
	// `| undefined` member. Regression for a spurious TS2353 ('limit' missing).
	['project_mode_physical_libs_required_omit_pick', 'physical-lib-required-omit-pick-basic'],
	// A generic alias indexing a nested-namespace interface
	// (`T extends keyof Inner.Table ? Inner.Table[T] : never`, React's
	// `ComponentProps<"button">` shape) finds the interface through its bare
	// dual-registration key, which carries no resolution scope of its own. The
	// lazy peel must reinstall the scope active where the reference was created,
	// or every member referencing an outer sibling degrades to `unknown`.
	[
		'project_mode_nested_namespace_member_resolves_siblings_on_lazy_peel',
		'namespace-nested-member-lazy-scope-basic',
	],
	// A function-type parameter written as a destructuring pattern
	// (react-hook-form's `ControllerProps.render` shape) must parse: failing it
	// degrades the whole function type, and any intersection containing it, to
	// `unknown`.
	[
		'project_mode_function_type_binding_pattern_parameter',
		'function-type-binding-pattern-param-basic',
	],
	// A call signature declared on a base interface (React's
	// `ForwardRefExoticComponent extends ExoticComponent` shape) must survive the
	// extends merge, or `T extends (props: infer P) => unknown` cannot recover the
	// props type from the component value.
	[
		'project_mode_interface_extends_inherits_call_signature',
		'interface-extends-call-signature-basic',
	],
	// `noLib: true` disables the default libs, so `Promise`/`Date` are missing.
	['project_mode_physical_libs_no_lib_disables_globals', 'physical-lib-no-lib-basic'],
	// No `compilerOptions.lib`: the target's `.full` lib graph (ES + DOM) loads by
	// default, so `Map`, `Promise`, `JSON`, `Number` and `Array.from` resolve.
	[
		'project_mode_physical_libs_are_the_default_es_dom',
		'default-lib-physical-default-es-dom-basic',
	],
	// `target: es2017` (no explicit lib) seeds `lib.es2017.full`, so the es2017
	// additions `Object.entries`/`Object.values`/`String.padStart` resolve.
	['project_mode_physical_libs_target_selects_graph', 'default-lib-physical-target-graph-basic'],
	['project_mode_physical_libs_lib_option_dom', 'default-lib-physical-lib-option-dom-basic'],
	[
		'project_mode_physical_libs_default_no_lib_disables_globals',
		'default-lib-physical-no-lib-basic',
	],
	// `navigator`, `document`, `window` come from the real DOM lib.
	['project_mode_physical_libs_dom_globals', 'default-lib-physical-dom-globals-basic'],
	['project_mode_physical_libs_timers', 'default-lib-physical-timers-basic'],
	['project_mode_physical_libs_formdata', 'default-lib-physical-formdata-basic'],
	// `HTMLDivElement` and `HTMLElement` come from the real DOM lib, not a
	// hardcoded global.
	['project_mode_physical_libs_html_element', 'default-lib-physical-html-element-basic'],
];

for (const [name, fixture] of LIB_FIXTURES) {
	test(name, async () => {
		await expectSameAsTscOnCompatProject(fixture);
	});
}

test('project_mode_lib_option_dom_enables_authenticator_transport', async () => {
	const root = tempProject({
		'tsconfig.json': `
        {
          "compilerOptions": {
            "lib": ["ES2022", "DOM"]
          },
          "include": ["src/**/*.ts"]
        }
        `,
		'src/index.ts': MATH_AND_TRANSPORT,
	});
	await expectSameAsTsc(root);
});

test('show_config_omits_base_url_and_keeps_paths', async () => {
	const root = tempProject({
		'tsconfig.json': `
        {
          "compilerOptions": {
            "paths": {
              "@app/*": ["src/*"]
            }
          },
          "include": ["src/**/*.ts"]
        }
        `,
		'src/index.ts': 'export const value = 1;',
	});
	const { stdout, stderr } = await runCli(['--project', tsconfigPath(root), '--showConfig'], root);

	expect(stderr).toBe('');
	expect(stdout).toContain('"paths"');
	expect(stdout).not.toContain('"baseUrl"');
	expect(JSON.parse(stdout).compilerOptions.paths['@app/*']).toEqual(['src/*']);
});

test('show_config_uses_ts7_defaults', async () => {
	const root = tempProject({ 'tsconfig.json': '{ "compilerOptions": {} }' });
	const { stdout, stderr } = await runCli(['--project', tsconfigPath(root), '--showConfig'], root);

	expect(stderr).toBe('');
	expect(stdout).toContain('"strict": true');
	expect(stdout).toContain('"noImplicitAny": true');
	expect(stdout).toContain('"target": "es2024"');
	expect(stdout).toContain('"module": "preserve"');
	expect(stdout).toContain('"moduleResolution": "bundler"');
});

test('project_mode_empty_config_triggers_ts7006', async () => {
	const root = tempProject({
		'tsconfig.json': `
        {
          "compilerOptions": {},
          "include": ["src/**/*.ts"]
        }
        `,
		'src/index.ts': UNTYPED_PARAMETER,
	});
	const { stdout, stderr } = await runCli(
		['--project', tsconfigPath(root), '--diagnosticProfile', 'native'],
		root,
	);

	expect(stderr).toBe('');
	expect(stdout).toContain('TS7006');
});

test('project_mode_package_extends_reports_ts7006_and_show_config_defaults', async () => {
	const root = tempProject({
		'node_modules/@tsconfig/strictest/tsconfig.json': `
        {
          "compilerOptions": {
            "strict": true,
            "target": "es2024",
            "module": "preserve",
            "moduleResolution": "bundler"
          }
        }
        `,
		'tsconfig.json': `
        {
          "extends": "@tsconfig/strictest",
          "include": ["src/**/*.ts"]
        }
        `,
		'src/index.ts': UNTYPED_PARAMETER,
	});
	const project = tsconfigPath(root);

	const native = await runCli(['--project', project, '--diagnosticProfile', 'native'], root);
	expect(native.stderr).toBe('');
	expect(native.stdout).toContain('TS7006');

	const config = await runCli(['--project', project, '--showConfig'], root);
	expect(config.stderr).toBe('');
	const parsed = JSON.parse(config.stdout);
	expect(parsed.compilerOptions.strict).toBe(true);
	expect(parsed.compilerOptions.noImplicitAny).toBe(true);
});

test('cli_reports_removed_compiler_options_with_tsc_spans', async () => {
	const root = tempProject({
		'tsconfig.json':
			'{\n  "include": ["*.ts"],\n  "compilerOptions": {\n    "esModuleInterop": false,\n    "outFile": "out.js"\n  }\n}\n',
		'index.ts': 'export const x = 1;\n',
	});
	await expectSameAsTsc(root, ['-p', 'tsconfig.json'], { messages: true });
});

test('cli_does_not_report_supported_compiler_options_as_removed', async () => {
	const root = tempProject({
		'tsconfig.json':
			'{ "include": ["*.ts"], "compilerOptions": { "esModuleInterop": true, "target": "es2022" } }',
		'index.ts': 'export const x = 1;\n',
	});
	await expectSameAsTsc(root);
});

test('project_mode_root_dir_admits_the_sources_of_referenced_projects', async () => {
	// A file reached through a project reference (directly or through the
	// referenced project's own references) belongs to that project, so this
	// project's `rootDir` does not hold it (TS6059).
	const root = tempProject({
		'tsconfig.json': `
        {
          "include": ["src"],
          "references": [{ "path": "projects/shared" }],
          "compilerOptions": { "strict": true, "rootDir": "src", "outDir": "out", "noEmit": true }
        }
        `,
		'src/index.ts': 'import { shared } from "../projects/shared/src/index";\nexport const value: number = shared;\n',
		'projects/shared/tsconfig.json': `
        {
          "include": ["src"],
          "references": [{ "path": "../common" }],
          "compilerOptions": { "composite": true, "rootDir": "src", "outDir": "out" }
        }
        `,
		'projects/shared/src/index.ts': 'import { common } from "../../common/src/index";\nexport const shared = common;\n',
		'projects/common/tsconfig.json': `
        {
          "include": ["src"],
          "compilerOptions": { "composite": true, "rootDir": "src", "outDir": "out" }
        }
        `,
		'projects/common/src/index.ts': 'export const common = 1;\n',
		// Built: tsc reads a referenced project's output (TS6305 otherwise).
		'projects/shared/out/index.d.ts': 'export declare const shared: number;\n',
		'projects/common/out/index.d.ts': 'export declare const common: number;\n',
	});
	await expectSameAsTsc(root);
});
