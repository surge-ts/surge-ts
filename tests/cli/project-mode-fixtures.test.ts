import { expect, test } from 'vitest';

import { tempProject } from '../harness/cli.ts';
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
	type Json,
} from './project-mode-helpers.ts';

const TSCONFIG = '{ "compilerOptions": {}, "include": ["src/**/*.ts"] }';

function compatJson(fixture: string, extra: string[] = []): Promise<Json> {
	const root = compatProject(fixture);
	return runCliJson(['--project', tsconfigPath(root), ...extra, '--format', 'json'], root);
}

function compatReport(fixture: string): Promise<Json> {
	return compatJson(fixture, ['--compatReport']);
}

async function expectStableAcrossJobs(fixture: string): Promise<void> {
	const jobs1 = await compatJson(fixture, ['--jobs', '1']);
	const jobs4 = await compatJson(fixture, ['--jobs', '4']);
	expect(fingerprintsOf(jobs4)).toEqual(fingerprintsOf(jobs1));
}

function sameAsTsc(name: string, fixture: string, options: { messages?: boolean } = {}): void {
	test(name, async () => {
		await expectSameAsTscOnCompatProject(fixture, options);
	});
}

function sameAsTscInline(name: string, source: string, tsconfig = TSCONFIG): void {
	test(name, async () => {
		const root = tempProject({ 'tsconfig.json': tsconfig, 'src/index.ts': source });
		await expectSameAsTsc(root);
	});
}

sameAsTsc(
	'cli_project_file_discovery_fixture_loads_all_supported_extensions',
	'project-file-discovery-extensions',
);

test('cli_project_file_discovery_fixture_compat_report_counts_loaded_files', async () => {
	const parsed = await compatReport('project-file-discovery-extensions');

	expect(parsed.filesLoaded).toBe(5);
	expect(parsed).not.toHaveProperty('visibilityWarning');
});

sameAsTsc(
	'cli_import_graph_generated_relative_basic_fixture_loads_relative_candidates',
	'import-graph-generated-relative-basic',
);

test('cli_import_graph_generated_relative_basic_fixture_compat_report_loads_files', async () => {
	const parsed = await compatReport('import-graph-generated-relative-basic');

	expect(parsed.loadedSourceFiles).toBe(3);
	expect(parsed.diagnosticsTotal).toBe(1);
});

sameAsTsc(
	'cli_paths_wildcard_import_graph_basic_fixture_resolves_relative_alias_target',
	'paths-wildcard-import-graph-basic',
);

test('cli_paths_wildcard_import_graph_basic_fixture_compat_report_loads_files', async () => {
	const parsed = await compatReport('paths-wildcard-import-graph-basic');

	expect(parsed.loadedSourceFiles).toBe(2);
	expect(parsed.diagnosticsTotal).toBe(1);
});

sameAsTsc(
	'cli_import_graph_dependency_js_not_source_fixture_uses_declaration_not_js',
	'import-graph-dependency-js-not-source',
);

test('cli_import_graph_dependency_js_not_source_fixture_compat_report_tracks_dependency_js_zero', async () => {
	const parsed = await compatReport('import-graph-dependency-js-not-source');

	expect(parsed.filesLoaded).toBe(1);
	expect(parsed.loadedSourceFiles).toBe(1);
	expect(parsed.loadedDependencyDeclarationFiles).toBe(1);
	expect(parsed.diagnosticsTotal).toBe(0);
});

sameAsTsc(
	'cli_builtin_visibility_project_graph_basic_fixture_keeps_synthetic_builtins_visible',
	'builtin-visibility-project-graph-basic',
);

test('cli_builtin_visibility_project_graph_basic_fixture_compat_report_tracks_loaded_imported_file', async () => {
	const parsed = await compatReport('builtin-visibility-project-graph-basic');

	expect(parsed.filesLoaded).toBe(2);
	expect(parsed.loadedSourceFiles).toBe(2);
	expect(parsed.diagnosticsTotal).toBe(0);
});

sameAsTsc(
	'cli_builtin_visibility_import_graph_basic_fixture_keeps_synthetic_builtins_visible',
	'builtin-visibility-import-graph-basic',
);

test('cli_builtin_visibility_import_graph_basic_fixture_compat_report_tracks_loaded_imported_file', async () => {
	const parsed = await compatReport('builtin-visibility-import-graph-basic');

	expect(parsed.filesLoaded).toBe(2);
	expect(parsed.loadedSourceFiles).toBe(2);
	expect(parsed.diagnosticsTotal).toBe(0);
});

sameAsTsc(
	'cli_builtin_visibility_function_body_basic_fixture_keeps_synthetic_builtins_visible',
	'builtin-visibility-function-body-basic',
);

test('cli_builtin_visibility_function_body_basic_fixture_is_stable_across_jobs', async () => {
	await expectStableAcrossJobs('builtin-visibility-function-body-basic');
});

sameAsTsc(
	'cli_module_local_functions_basic_fixture_keeps_same_file_helpers_visible',
	'module-local-functions-basic',
);
sameAsTsc(
	'cli_default_parameter_inference_basic_fixture_keeps_defaults_typed',
	'default-parameter-inference-basic',
);
sameAsTsc(
	'cli_function_body_scope_hardening_fixture_keeps_locals_visible',
	'function-body-scope-hardening',
);

test('cli_function_body_scope_hardening_fixture_is_stable_across_jobs', async () => {
	await expectStableAcrossJobs('function-body-scope-hardening');
});

sameAsTsc(
	'cli_module_local_helper_functions_hardening_fixture_keeps_helpers_visible',
	'module-local-helper-functions-hardening',
);

test('cli_module_local_helper_functions_hardening_fixture_is_stable_across_jobs', async () => {
	await expectStableAcrossJobs('module-local-helper-functions-hardening');
});

sameAsTsc(
	'cli_primitive_methods_basic_fixture_supports_string_number_and_array_methods',
	'primitive-methods-basic',
);
sameAsTsc(
	'cli_new_expression_builtins_basic_fixture_supports_builtin_constructors',
	'new-expression-builtins-basic',
);
sameAsTsc(
	'cli_object_shorthand_scope_basic_fixture_keeps_shorthand_locals_visible',
	'object-shorthand-scope-basic',
);
sameAsTsc(
	'cli_function_body_local_visibility_basic_fixture_keeps_locals_visible',
	'function-body-local-visibility-basic',
);
sameAsTsc(
	'cli_dependency_incomplete_declaration_export_fallback_fixture_keeps_local_ts2305',
	'dependency-incomplete-declaration-export-fallback',
);
sameAsTsc(
	'cli_relative_directory_index_basic_fixture_resolves_loaded_directory_indexes',
	'relative-directory-index-basic',
);
sameAsTsc('cli_tsx_parser_safe_basic_fixture_reports_ts2322', 'tsx-parser-safe-basic');
// Only the meaningful assignment mismatch is reported; the well-formed JSX on
// lines 2-3 produces no cascade. The conservative `JSX.Element` stand-in renders
// as `Element`, matching tsc's message exactly.
sameAsTsc('cli_tsx_jsx_basic_fixture_reports_element_not_assignable', 'tsx-jsx-basic', {
	messages: true,
});
// React-19 shape: no global `JSX`; the namespace lives at `React.JSX` inside the
// react module. Under `jsx: react-jsx` an intrinsic tag in a file with no `React`
// binding still resolves through the runtime declarer, so the `onClick` arrow is
// contextually typed (TS2322 inside the body, no TS7006).
sameAsTsc(
	'cli_jsx_runtime_module_namespace_fixture_types_intrinsic_callbacks',
	'jsx-runtime-module-namespace-basic',
);
// A component whose props type is a LOCAL alias of an imported qualified type
// (`type ButtonProps = React.ButtonAttributes`): the alias must not bake a
// degraded signature during the binding passes (its attached scope carries no
// import layers), so the `onClick` arrow is contextually typed.
sameAsTsc(
	'cli_jsx_imported_alias_props_fixture_types_component_callbacks',
	'jsx-imported-alias-props-basic',
);
sameAsTsc(
	'cli_tsx_jsx_expression_diagnostics_basic_fixture_reports_unresolved_child',
	'tsx-jsx-expression-diagnostics-basic',
);
// The capitalized `<Button />` tag is a value reference; the intrinsic
// `<div id="root" />` and the resolved `{count}` attribute do not cascade.
sameAsTsc(
	'cli_tsx_jsx_attributes_basic_fixture_reports_unresolved_component',
	'tsx-jsx-attributes-basic',
);
// Adding JSX parsing must not disturb `.ts` generic call / angle-bracket behaviour.
sameAsTsc(
	'cli_tsx_generic_angle_regression_basic_fixture_has_no_diagnostics',
	'tsx-generic-angle-regression-basic',
);

function ambientModulesOf(parsed: Json): string[] {
	expect(Array.isArray(parsed.ambientExternalModules)).toBe(true);
	return parsed.ambientExternalModules;
}

test('cli_project_loads_d_ts_files', async () => {
	const parsed = await compatReport('declarations-basic');

	expect(parsed.declarationFilesLoaded).toBe(2);
	expect(ambientModulesOf(parsed)).toEqual(['pkg', 'pkg/subpath']);
});

sameAsTsc('cli_project_declaration_global_type_valid', 'declarations-basic');
sameAsTsc('cli_project_declaration_global_function_valid', 'declarations-basic');

test('cli_project_ambient_module_import_valid', async () => {
	const parsed = await compatReport('declarations-basic');

	expect(ambientModulesOf(parsed)).toEqual(['pkg', 'pkg/subpath']);
});

test('cli_project_ambient_module_missing_export', async () => {
	const root = tempProject({
		'tsconfig.json': `{
          "include": ["src/**/*.ts", "types/**/*.d.ts"]
        }`,
		'src/index.ts': 'import { missing } from "pkg";',
		'types/pkg.d.ts': 'declare module "pkg" { export const foo: number; }',
	});
	await expectSameAsTsc(root);
});

sameAsTsc('cli_project_ambient_module_unknown_package_fallback_default', 'declarations-basic');

test('cli_project_ambient_module_unknown_package_fallback_stub_external_modules', async () => {
	const parsed = await compatJson('declarations-basic', ['--stubExternalModules']);

	expect(linesOf(parsed, 'TS2307')).toEqual([]);
	expect(codesOf(parsed)).toContain('TS2322');
});

test('cli_project_declaration_compat_report', async () => {
	const root = compatProject('declarations-basic');
	const { stdout } = await runCli(['--project', tsconfigPath(root), '--compatReport'], root);

	expect(stdout).toContain('Declaration files loaded');
});

test('cli_project_declaration_format_json', async () => {
	const parsed = await compatReport('declarations-basic');

	expect(parsed.declarationFilesLoaded).toBe(2);
	expect(ambientModulesOf(parsed)).toEqual(['pkg', 'pkg/subpath']);
});

sameAsTsc('cli_declarations_basic_loads_globals_d_ts', 'declarations-basic');
sameAsTsc('cli_declarations_basic_loads_pkg_d_ts', 'declarations-basic');
sameAsTsc('cli_declarations_basic_no_ts2307_for_declared_pkg', 'declarations-basic');
sameAsTsc('cli_declarations_basic_no_ts2307_for_declared_subpath', 'declarations-basic');
sameAsTsc('cli_declarations_basic_missing_pkg_fallback_ts2307', 'declarations-basic');

test('cli_declarations_basic_stub_external_modules_suppresses_only_missing_pkg', async () => {
	const parsed = await compatJson('declarations-basic', ['--stubExternalModules']);
	const codes = codesOf(parsed);

	expect(codes).not.toContain('TS2307');
	expect(codes).toContain('TS2322');
});

test('cli_declarations_basic_format_json_stable', async () => {
	const diagnostics = diagnosticsOf(await compatJson('declarations-basic'));

	expect(diagnostics).not.toHaveLength(0);
	for (const diagnostic of diagnostics) {
		expect(diagnostic).toHaveProperty('code');
		expect(diagnostic).toHaveProperty('fileName');
		expect(diagnostic).toHaveProperty('message');
	}
});

test('cli_declarations_hardening_loads_ambient_modules', async () => {
	const parsed = await compatReport('declarations-hardening');

	expect(parsed.declarationFilesLoaded).toBe(1);
	expect(ambientModulesOf(parsed)).toEqual([
		'barrel-pkg',
		'barrel-star-pkg',
		'barrel-type-pkg',
		'merge-pkg',
		'pkg-default',
		'pkg-default-function',
		'pkg-ns',
		'source-pkg',
	]);
});

sameAsTsc('cli_declarations_hardening_no_diagnostics', 'declarations-hardening');

for (const name of [
	'cli_package_declarations_resolves_subpath_d_ts_file',
	'cli_package_declarations_resolves_subpath_index_d_ts_fallback',
	'cli_package_declarations_resolves_scoped_subpath',
	'cli_package_declarations_ignores_wildcard_exports',
	'cli_package_declarations_resolves_exports_types_subpath',
	'cli_package_declarations_ignores_runtime_only_exports',
	'cli_package_declarations_side_effect_resolved_subpath_no_ts2882',
	'cli_package_declarations_unresolved_subpath_reports_ts2307',
	'cli_package_declarations_unresolved_side_effect_subpath_reports_ts2882',
]) {
	sameAsTsc(name, 'package-declarations');
}

test('cli_package_declarations_stub_external_modules_suppresses_unresolved_subpath_only', async () => {
	const parsed = await compatJson('package-declarations', ['--stubExternalModules']);
	const lines2307 = linesOf(parsed, 'TS2307');
	const lines2882 = linesOf(parsed, 'TS2882');

	expect(lines2307).not.toContain(1); // pkg/subpath
	expect(lines2307).not.toContain(17); // runtime-only
	expect(lines2882).not.toContain(3); // runtime-only side-effect
	// Errors inside resolved packages survive stub mode.
	expect(codesOf(parsed)).toContain('TS2322');
});

sameAsTsc(
	'cli_package_declarations_missing_export_from_resolved_subpath_reports_ts2305',
	'package-declarations',
);
sameAsTsc(
	'cli_package_types_node_modules_basic_resolves_bundled_types',
	'package-types-node-modules-basic',
);
sameAsTsc(
	'cli_package_types_exports_conditions_basic_resolves_nested_types',
	'package-types-exports-conditions-basic',
);
sameAsTsc(
	'cli_package_types_at_types_fallback_basic_resolves_root_package',
	'package-types-at-types-fallback-basic',
);
sameAsTsc(
	'cli_package_types_scoped_at_types_fallback_basic_resolves_scoped_package',
	'package-types-scoped-at-types-fallback-basic',
);
sameAsTsc(
	'cli_package_types_export_equals_import_require_valid_binds_value',
	'package-types-export-equals-import-require-valid',
);
sameAsTsc(
	'cli_package_types_export_equals_property_call_valid_resolves_method',
	'package-types-export-equals-property-call-valid',
);
sameAsTsc(
	'cli_package_types_export_equals_property_call_argument_mismatch_reports_ts2345',
	'package-types-export-equals-property-call-argument-mismatch',
);
// The package's `export = missingValue` target is undefined; the consumer binds
// an unknown value and must not cascade name/property errors.
sameAsTsc(
	'cli_package_types_export_equals_missing_export_target_no_cascade',
	'package-types-export-equals-missing-export-target-no-cascade',
);
sameAsTsc(
	'cli_package_types_import_require_missing_package_reports_ts2307',
	'package-types-import-require-missing-package',
);
sameAsTsc(
	'cli_package_types_import_require_subpath_valid_binds_value',
	'package-types-import-require-subpath-valid',
);
sameAsTsc(
	'cli_package_exports_conditional_types_basic_resolves_types_condition',
	'package-exports-conditional-types-basic',
);
sameAsTsc('cli_package_exports_subpath_basic_resolves_subpath_types', 'package-exports-subpath-basic');
sameAsTsc(
	'cli_package_exports_pattern_basic_resolves_wildcard_subpath',
	'package-exports-pattern-basic',
);
sameAsTsc(
	'cli_package_exports_custom_condition_basic_selects_development_branch',
	'package-exports-custom-condition-basic',
);
sameAsTsc('cli_package_typesversions_basic_rewrites_root_types', 'package-typesversions-basic');
sameAsTsc(
	'cli_package_typesversions_subpath_basic_rewrites_exact_and_pattern',
	'package-typesversions-subpath-basic',
);
sameAsTsc('cli_package_imports_field_basic_resolves_internal_alias', 'package-imports-field-basic');
sameAsTsc(
	'cli_package_imports_pattern_basic_resolves_alias_wildcard',
	'package-imports-pattern-basic',
);
sameAsTsc(
	'cli_package_self_name_import_basic_resolves_own_exports',
	'package-self-name-import-basic',
);
sameAsTsc(
	'cli_package_exports_unresolved_no_cascade_reports_ts2307_only',
	'package-exports-unresolved-no-cascade',
);
sameAsTsc(
	'cli_package_exports_missing_export_basic_reports_ts2305',
	'package-exports-missing-export-basic',
);
sameAsTsc(
	'cli_package_exports_stub_external_preserved_basic_default_reports_ts2307_and_ts2322',
	'package-exports-stub-external-preserved-basic',
);

test('cli_package_exports_stub_external_preserved_basic_stub_suppresses_only_unresolved', async () => {
	const parsed = await compatJson('package-exports-stub-external-preserved-basic', [
		'--stubExternalModules',
	]);
	const codes = codesOf(parsed);

	// Unresolved external package is suppressed under stub mode...
	expect(codes).not.toContain('TS2307');
	// ...but errors inside the resolved package declaration are preserved.
	expect(codes).toContain('TS2322');
});

sameAsTsc(
	'cli_no_implicit_any_uninitialized_let_basic_matches_typescript',
	'no-implicit-any-uninitialized-let-basic',
);
sameAsTsc(
	'cli_jwt_payload_same_file_visibility_basic_reports_type_error_only',
	'jwt-payload-same-file-visibility-basic',
);
sameAsTsc(
	'cli_imported_interface_extends_downstream_assignability_basic_reports_missing_property',
	'imported-interface-extends-downstream-assignability-basic',
);
sameAsTsc(
	'cli_imported_type_bindings_in_declaration_bodies_basic_resolves_imports',
	'imported-type-bindings-in-declaration-bodies-basic',
);
sameAsTsc(
	'cli_contextual_async_object_property_return_basic_reports_type_errors_only',
	'contextual-async-object-property-return-basic',
);
// The only diagnostic is the intentional TS2355 on the value-promise function;
// every `Promise<void|undefined|any>` shape (function, arrow, object method,
// class method, alias) must stay clean.
sameAsTsc(
	'cli_async_void_promise_return_basic_exempts_awaited_void_from_ts2355',
	'async-void-promise-return-basic',
);
sameAsTsc(
	'cli_array_find_contextual_callback_basic_resolves_find_and_reports_property_error',
	'array-find-contextual-callback-basic',
);

// A call signature that mentions `symbol` (param or return) must not poison the
// interface's callability. Regression for `Symbol('x')` reported as TS2349.
sameAsTscInline(
	'cli_symbol_in_call_signature_stays_callable',
	`
        interface MySymbolCtor {
          (description?: string | number): symbol;
          for(key: string): symbol;
        }
        declare const make: MySymbolCtor;
        export const made = make('x');
        export const bridged: symbol = make(1);
        `,
);

// tsc skips definite-assignment analysis for a binding whose declared type
// permits `undefined` (`any`, or a union containing `undefined`). Regression for
// TS2454 on `let x: any` / `let x: T | undefined` assigned in try/loop.
sameAsTscInline(
	'cli_definite_assignment_exempts_any_and_undefined_typed_let',
	`
        export function anyVar() {
          let x: any;
          for (const i of [1, 2]) { x = i; }
          return x === undefined ? 0 : x + 1;
        }
        export function undefUnion() {
          let u: number | undefined;
          try { u = 1; } catch {}
          return u ? u : 0;
        }
        export function stillReports() {
          let s: string;
          return s.length;
        }
        `,
);

// A function body may reference a module-scope `const` declared after it; the
// body runs once the module is fully evaluated. Regression for TS2304 on forward
// references like ky's `deepMerge`.
sameAsTscInline(
	'cli_function_body_resolves_later_module_const',
	`
        export const useEarly = () => later(1);
        const later = (x: number) => x + 1;
        export function stillReports() { return totallyMissing(1); }
        `,
);

// A `typeof X` type query resolving a value during statement checking must
// consult the module's full value table, not just the active type-resolution
// scope. Regression for ky's `readonly retry: typeof retry` (TS2304) and zod's
// `(typeof ZodString)["create"]`.
sameAsTscInline(
	'cli_typeof_value_in_type_query_resolves_via_module_table',
	`
        export class Foo {
          static create(x: number) { return new Foo(); }
        }
        export const coerce = {
          make: ((a) => Foo.create(a)) as (typeof Foo)["create"],
        };
        export const missing = (() => 0) as typeof totallyUndefined;
        `,
);

// Calling an `any`-typed value is allowed and yields `any`. Regression for ky's
// `for (const hook of hooks?.init ?? []) hook(opts)`, where `?? []` widens the
// iterated element to `any`.
sameAsTscInline(
	'cli_any_typed_callee_is_callable',
	`
        type Hook = (x: number) => void;
        declare const hooks: { init?: Hook[] };
        export function run() {
          for (const hook of hooks.init ?? []) {
            hook(1);
          }
        }
        declare const f: any;
        export const r = f(1, 2, 3);
        `,
);

// `a.b && a.b > c` evaluates the right side only when `a.b` is truthy, so `a.b`
// narrows to non-nullish there.
sameAsTscInline(
	'cli_and_chain_narrows_truthy_property',
	`
        declare const c: { x?: number };
        declare const p: { i: number };
        export const r = c.x && p.i > c.x;
        export function f() {
          if (c.x && p.i > c.x) { return 1; }
          return 0;
        }
        `,
	'{ "compilerOptions": { "strict": true }, "include": ["src/**/*.ts"] }',
);

// `if (x instanceof A || x instanceof B)` narrows `x` to `A | B` in the
// then-branch. Regression for ky's `body instanceof ArrayBuffer ||
// ArrayBuffer.isView(body)`.
sameAsTscInline(
	'cli_or_of_instanceof_guards_narrows_union',
	`
        class A { byteLength = 2; }
        class B { byteLength = 3; }
        type U = A | B | string;
        export const orNarrow = (x: U): number => {
          if (x instanceof A || x instanceof B) {
            return x.byteLength;
          }
          return 0;
        };
        `,
);

// `obj[k]` where `k` is a non-literal computed key (`keyof T` or a type parameter
// `K extends keyof T`) resolves to an indexed-access type. Regression for a
// TS2339 that mis-named the receiver as the absent property (ky's
// `incoming[property]`).
sameAsTscInline(
	'cli_computed_key_index_on_object_is_not_missing_property',
	`
        type Hooks = { init?: number[]; before?: string[] };
        declare const h: Hooks;
        declare const k: keyof Hooks;
        export const viaKeyof = h[k];
        export function viaGeneric<K extends keyof Hooks>(o: Hooks, p: K) {
          return o[p];
        }
        `,
);

// `typeof x === "tag"` must narrow the union in the then-branch, the else branch,
// and the fall-through after an early-returning `if`.
sameAsTscInline(
	'cli_typeof_guard_narrows_union_in_all_branches',
	`
        export function earlyReturn(x: number | string) {
          if (typeof x === 'number') { return 0; }
          return x.length;
        }
        export function withElse(x: number | { a: number }) {
          if (typeof x === 'number') { return x; }
          else { return x.a; }
        }
        export function discriminantElse(
          v: { kind: 'a'; x: number } | { kind: 'b'; y: string },
        ) {
          if (v.kind === 'a') { return v.x; }
          return v.y;
        }
        `,
);

sameAsTscInline(
	'cli_instanceof_guard_narrows_union',
	`
        class Cat { meow(): number { return 1; } }
        class Dog { bark(): number { return 2; } }
        export function sound(animal: Cat | Dog) {
          if (animal instanceof Cat) { return animal.meow(); }
          return animal.bark();
        }
        export function withElse(animal: Cat | Dog) {
          if (animal instanceof Dog) { return animal.bark(); }
          else { return animal.meow(); }
        }
        `,
);

sameAsTscInline(
	'cli_tuple_exposes_array_methods',
	`
        const methods = ['get', 'post', 'put'] as const;
        export const hasGet = methods.includes('get');
        export const upper = methods.map(m => m.toUpperCase());
        `,
);

// `{ [P in string]: T }` must resolve to a string index signature, not collapse
// to `unknown`: the TS2322 below appears only if it did not collapse, since surge
// treats `unknown` leniently.
sameAsTscInline(
	'cli_string_keyed_mapped_type_resolves_to_index_signature',
	`
        type Idx = { [P in string]: number };
        const d = {} as Idx;
        export const mismatch: string = d.anyKey;
        `,
);

test('cli_skip_lib_check_dependency_dts_loads_as_symbol_source_without_noise', async () => {
	const parsed = await compatReport('skip-lib-check-dependency-dts');

	expect(parsed.loadedDependencyDeclarationFiles).toBe(1);
	expect(parsed.diagnosticsDependencyDeclarationTotal).toBe(0);
	expect(parsed.diagnosticsTotal).toBe(0);
});

// `process` resolves through configured @types/node.
sameAsTsc('cli_configured_types_node_loads_at_types_declarations', 'configured-types-node-basic');
// `@scope/pkg` maps to node_modules/@types/scope__pkg, whose `declare const`
// becomes a global.
sameAsTsc(
	'cli_configured_types_scoped_maps_to_at_types_scope_dir',
	'configured-types-scoped-basic',
);
// `subpath-pkg/globals` exists only under the package's `exports` map, so the
// type-root lookup cannot find it; the secondary node_modules lookup must.
sameAsTsc(
	'cli_configured_types_subpath_resolves_through_package_exports',
	'configured-types-subpath-basic',
);
test('cli_configured_types_missing_reports_ts2688', async () => {
	await expectSameAsTscOnCompatProject('configured-types-missing-basic');

	const diagnostics = diagnosticsOf(await compatJson('configured-types-missing-basic'));
	expect(diagnostics).toHaveLength(1);
	expect(diagnostics[0].fileName).toBe('');
	expect(diagnostics[0].message).toBe(
		"Cannot find type definition file for 'configured-types-missing-pkg'.\n" +
			'  The file is in the program because:\n' +
			"    Entry point of type library 'configured-types-missing-pkg' specified in compilerOptions",
	);
});
// Without `compilerOptions.types`, configured @types are not pulled in, so
// `process` stays unresolved.
sameAsTsc('cli_configured_types_no_node_does_not_autoload_at_types', 'configured-types-no-node-basic');
// `types: ["*"]` auto-discovers the visible @types/node package.
sameAsTsc('cli_auto_types_wildcard_discovers_node', 'auto-types-node-basic');
// `types: []` disables automatic inclusion.
sameAsTsc('cli_auto_types_empty_types_disables_discovery', 'auto-types-disabled-empty-types-basic');
// `types: ["node"]` loads node but not the visible react package.
sameAsTsc('cli_auto_types_narrowed_loads_only_listed', 'auto-types-narrowed-types-basic');
// `types: ["*"]` discovers @types/scope__pkg and unmangles it to a global.
sameAsTsc('cli_auto_types_wildcard_discovers_scoped_package', 'auto-types-scoped-basic');
// The visible @types/node lives in an ancestor directory; `types: ["*"]` walks up
// the node_modules/@types chain to find it.
sameAsTsc(
	'cli_auto_types_wildcard_discovers_ancestor_node',
	'auto-types-ancestor-visibility-basic/packages/app',
);
// The app-local @types/node (`marker.token: string`) wins over the ancestor copy
// (`marker.token: number`).
sameAsTsc('cli_auto_types_nearest_package_wins', 'auto-types-nearest-wins-basic/packages/app');
// `typeRoots: ["./custom-types"]` with `types: ["*"]` discovers the package under
// the custom root.
sameAsTsc('cli_type_roots_wildcard_includes_custom_root_package', 'type-roots-basic');
// With `typeRoots` set, the default node_modules/@types/node is not consulted.
sameAsTsc(
	'cli_type_roots_ignores_default_node_modules',
	'type-roots-ignore-default-node-modules-basic',
);
// `typeRoots` + `types: ["node"]` loads only node from the custom root.
sameAsTsc(
	'cli_type_roots_with_types_filter_loads_only_listed',
	'type-roots-with-types-filter-basic',
);
// `/// <reference types="node" />` resolves @types/node even though
// `compilerOptions.types` is empty.
sameAsTsc('cli_reference_types_directive_loads_node_at_types', 'reference-types-node-basic');
sameAsTsc(
	'cli_reference_types_directive_resolves_scoped_package',
	'reference-types-scoped-basic',
);
// pkg-a's own `/// <reference types="pkg-b" />` is followed.
sameAsTsc(
	'cli_reference_types_directive_follows_recursive_references',
	'reference-types-recursive-basic',
);
sameAsTsc(
	'cli_reference_types_directive_missing_reports_located_ts2688',
	'reference-types-missing-basic',
	{ messages: true },
);
// A reference directive inside a resolved package's declaration file pulls in
// its @types dependency.
sameAsTsc(
	'cli_reference_types_directive_in_dependency_dts_is_followed',
	'reference-types-dependency-dts-basic',
);
sameAsTsc(
	'cli_reference_types_missing_in_dependency_dts_respects_skip_lib_check',
	'reference-types-missing-dependency-dts-skip-lib-check-basic',
);
// With custom `typeRoots`, the directive resolves only through the allowed root
// (verbatim name, no @types mangling).
sameAsTsc(
	'cli_reference_types_directive_resolves_through_type_roots',
	'reference-types-with-type-roots-basic',
);
sameAsTsc(
	'cli_reference_types_directive_dedupes_across_files',
	'reference-types-dedupe-order-basic',
);

sameAsTsc('project_mode_interface_merging_same_scope', 'interface-merging-basic');
sameAsTsc('project_mode_interface_merging_across_files', 'interface-merging-across-files-basic');
// Incompatible property types across merged interfaces report TS2717 once; the
// first declaration's type wins, so the assignment does not cascade.
sameAsTsc('project_mode_interface_merging_property_conflict', 'interface-merging-conflict-basic');
sameAsTsc('project_mode_declare_global_interface_merging', 'declare-global-interface-basic');
sameAsTsc(
	'project_mode_declare_global_window_physical_lib',
	'declare-global-window-physical-lib-basic',
);
sameAsTsc(
	'project_mode_module_augmentation_package_interface',
	'module-augmentation-package-interface-basic',
);
sameAsTsc('project_mode_module_augmentation_add_export', 'module-augmentation-add-export-basic');
sameAsTsc('project_mode_ambient_module_reopen_merge', 'ambient-module-reopen-merge-basic');
// `namespace X` declaration-merges with a same-named `const`/`function` in the
// same ambient module instead of shadowing it, in either declaration order (the
// `@types/node` `path` shape).
sameAsTsc('project_mode_ambient_namespace_value_merge', 'ambient-namespace-value-merge-basic');
// A type-only `declare namespace X` in a script declaration file must not claim
// the global name ahead of the `declare global { var X: … }` block lowered in a
// later pass (the bun-types shape), which pinned `X` to `{}`.
sameAsTsc(
	'project_mode_ambient_global_namespace_value_merge',
	'ambient-global-namespace-value-merge-basic',
);
// Augmenting an unresolved module is reported once as TS2307 with no downstream
// cascade from the unresolved import binding.
sameAsTsc(
	'project_mode_module_augmentation_unresolved_no_cascade',
	'module-augmentation-unresolved-no-cascade',
);
sameAsTsc('project_mode_interface_method_merge', 'interface-method-merge-basic');
sameAsTsc(
	'project_mode_class_interface_merge_instance_members',
	'class-interface-merge-policy-pinned',
);
// `export { X }` / `export type { X }` naming an imported type must enter the
// export table, or the name is lost through every re-export hop.
sameAsTsc('project_mode_re_export_imported_type', 're-export-imported-type-basic');
// `import type { f }` imports the symbol, so a function/const export is a legal
// target and `typeof f` must resolve.
sameAsTsc('project_mode_import_type_of_value_only_export', 'import-type-value-only-export-basic');
// A named import reaches the members of an `export = X` surface, including
// through an `import x = require(...)` alias module.
sameAsTsc('project_mode_export_assignment_named_import', 'export-assignment-named-import-basic');
// A type-only namespace publishes only qualified `NS.Member` keys, so the
// re-export path needs the same fallback the import path has.
sameAsTsc('project_mode_reexport_type_only_namespace', 'reexport-type-only-namespace-basic');
// A module ending in `export default <expression>` has a default export even when
// the expression form is unmodelled, and `export { default }` republishes it.
sameAsTsc(
	'project_mode_default_export_expression_reexport',
	'default-export-expression-reexport-basic',
);
