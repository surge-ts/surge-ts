// Module-resolution parity fixtures. Each test pins a TypeScript resolver rule
// that surge must match. See `crates/surge-ts/MODULE_RESOLUTION.md` for where
// the resolver port lives.

import { expect, test } from 'vitest';

import { expectSameDiagnosticsAsTsc, surge, tempProject } from '../harness/cli.ts';

const PROJECT_ARGS = ['--project', 'tsconfig.json'];

const BUNDLER_TSCONFIG = `
{
  "compilerOptions": {
    "moduleResolution": "bundler",
    "module": "preserve",
    "strict": true,
    "noEmit": true
  },
  "include": ["src/**/*"]
}
`;

function pathsTsconfig(paths: string): string {
	return `
{
  "compilerOptions": {
    "moduleResolution": "bundler",
    "module": "preserve",
    "strict": true,
    "noEmit": true,
    "paths": ${paths}
  },
  "include": ["src/**/*"]
}
`;
}

const PKG_TSCONFIG = `
{
  "compilerOptions": {
    "moduleResolution": "bundler",
    "module": "preserve",
    "strict": true,
    "noEmit": true
  },
  "include": ["packages/**/*.ts"]
}
`;

async function expectSameAsTsc(files: Record<string, string>): Promise<void> {
	await expectSameDiagnosticsAsTsc(tempProject(files), PROJECT_ARGS);
}

// tsc: `import "./user.js"` never resolves `user/index.ts` — an explicit
// runtime extension is a file-shaped path, not a directory lookup.
test('explicit_js_never_resolves_directory_index', async () => {
	await expectSameAsTsc({
		'tsconfig.json': BUNDLER_TSCONFIG,
		'src/index.ts': 'import { a } from "./user.js";',
		'src/user/index.ts': 'export const a = 1;',
	});
});

test('explicit_mjs_and_cjs_never_resolve_directory_index', async () => {
	await expectSameAsTsc({
		'tsconfig.json': BUNDLER_TSCONFIG,
		'src/index.ts': `
import { a } from "./user.mjs";
import { b } from "./other.cjs";
`,
		'src/user/index.mts': 'export const a = 1;',
		'src/other/index.cts': 'export const b = 2;',
	});
});

// tsc substitution matrix: `.js` never reaches `.mts`/`.cts`, and an
// extensionless specifier never probes them either.
test('js_and_extensionless_do_not_resolve_m_c_flavors', async () => {
	await expectSameAsTsc({
		'tsconfig.json': BUNDLER_TSCONFIG,
		'src/index.ts': `
import { b } from "./m.js";
import { c } from "./c.js";
import { d } from "./m2";
`,
		'src/m.mts': 'export const b = 2;',
		'src/c.cts': 'export const c = 3;',
		'src/m2.mts': 'export const d = 4;',
	});
});

test('mjs_resolves_mts_and_cjs_resolves_cts', async () => {
	await expectSameAsTsc({
		'tsconfig.json': BUNDLER_TSCONFIG,
		'src/index.ts': `
import { b } from "./m.mjs";
import { c } from "./c.cjs";
const useB: number = b;
const useC: string = c;
`,
		'src/m.mts': 'export const b: number = 2;',
		'src/c.cts': 'export const c: string = "x";',
	});
});

// tsc resolves `./comp.jsx` through the default substitution set
// (`.ts`/`.tsx`/`.d.ts`), same as `.js`.
test('jsx_specifier_resolves_tsx_source', async () => {
	await expectSameAsTsc({
		'tsconfig.json': `
{
  "compilerOptions": {
    "moduleResolution": "bundler",
    "module": "preserve",
    "jsx": "react-jsx",
    "strict": true,
    "noEmit": true
  },
  "include": ["src/**/*"]
}
`,
		'src/index.ts': 'import { c } from "./comp.jsx";',
		'src/comp.tsx': 'export const c = 3;',
	});
});

test('bundler_extensionless_relative_resolves', async () => {
	await expectSameAsTsc({
		'tsconfig.json': BUNDLER_TSCONFIG,
		'src/index.ts': `
import { a } from "./file";
import { b } from "./dir";
`,
		'src/file.ts': 'export const a = 1;',
		'src/dir/index.ts': 'export const b = 2;',
	});
});

// The wildcard pattern with the longest literal prefix wins, regardless of
// config order. If `@/*` won here, `value` would import the fallback module
// that does not export it.
test('paths_longest_prefix_wins', async () => {
	await expectSameAsTsc({
		'tsconfig.json': pathsTsconfig(`{
      "@/*": ["./src/fallback/*"],
      "@/core/*": ["./src/core/*"]
    }`),
		'src/index.ts': `
import { value } from "@/core/value";
const use: number = value;
`,
		'src/core/value.ts': 'export const value: number = 1;',
		'src/fallback/value.ts': 'export const notValue = 0;',
	});
});

test('paths_exact_pattern_beats_wildcard', async () => {
	await expectSameAsTsc({
		'tsconfig.json': pathsTsconfig(`{
      "lib/special": ["./src/special.ts"],
      "lib/*": ["./src/generic/*"]
    }`),
		'src/index.ts': `
import { special } from "lib/special";
const use: string = special;
`,
		'src/special.ts': 'export const special: string = "s";',
		'src/generic/special.ts': 'export const wrong = 0;',
	});
});

test('paths_first_target_missing_second_succeeds', async () => {
	await expectSameAsTsc({
		'tsconfig.json': pathsTsconfig('{ "multi/*": ["./src/missing/*", "./src/real/*"] }'),
		'src/index.ts': `
import { second } from "multi/second";
const use: number = second;
`,
		'src/real/second.ts': 'export const second: number = 2;',
	});
});

// tsc resolves targets without a leading `./` against the mapping base (it
// additionally flags the config with TS5090 under TS7 — a config-level
// diagnostic surge does not model — but resolution still succeeds).
test('paths_target_without_dot_slash_resolves', async () => {
	await expectSameAsTsc({
		'tsconfig.json': pathsTsconfig('{ "nodot/*": ["src/core/*"] }'),
		'src/index.ts': `
import { nd } from "nodot/nd";
const use: number = nd;
`,
		'src/core/nd.ts': 'export const nd: number = 3;',
	});
});

// Two importers resolve the same bare specifier to different files through
// their own nested `node_modules`. A specifier-keyed resolution map would let
// the first importer's result leak into the second.
test('same_package_name_different_importers', async () => {
	await expectSameAsTsc({
		'tsconfig.json': PKG_TSCONFIG,
		'packages/a/node_modules/dep/package.json': '{ "name": "dep", "types": "./index.d.ts" }',
		'packages/a/node_modules/dep/index.d.ts': 'export declare const value: string;',
		'packages/b/node_modules/dep/package.json': '{ "name": "dep", "types": "./index.d.ts" }',
		'packages/b/node_modules/dep/index.d.ts': 'export declare const value: number;',
		'packages/a/src/index.ts': `
import { value } from "dep";
const useA: string = value;
`,
		'packages/b/src/index.ts': `
import { value } from "dep";
const useB: number = value;
`,
	});
});

// `#alias` imports resolve against the importer's nearest enclosing package
// scope; the same alias in two scopes must not collide.
test('package_imports_nearest_scope', async () => {
	await expectSameAsTsc({
		'tsconfig.json': PKG_TSCONFIG,
		'packages/a/package.json': '{ "name": "a", "imports": { "#util": "./src/util.ts" } }',
		'packages/b/package.json': '{ "name": "b", "imports": { "#util": "./src/util.ts" } }',
		'packages/a/src/util.ts': 'export const util: string = "a";',
		'packages/b/src/util.ts': 'export const util: number = 2;',
		'packages/a/src/index.ts': `
import { util } from "#util";
const useA: string = util;
`,
		'packages/b/src/index.ts': `
import { util } from "#util";
const useB: number = util;
`,
	});
});

// A package importing its own name resolves through its own `exports` map.
test('package_self_name_import', async () => {
	await expectSameAsTsc({
		'tsconfig.json': `
{
  "compilerOptions": {
    "moduleResolution": "bundler",
    "module": "preserve",
    "strict": true,
    "noEmit": true
  },
  "include": ["src/**/*.ts"]
}
`,
		'package.json':
			'{ "name": "self-pkg", "exports": { ".": { "types": "./types/index.d.ts" } } }',
		'types/index.d.ts': 'export declare const marker: string;',
		'src/index.ts': `
import { marker } from "self-pkg";
const use: string = marker;
`,
	});
});

// An `exports` map is authoritative: a `null` target blocks the subpath with
// no filesystem fallback.
test('package_exports_null_blocks_subpath', async () => {
	await expectSameAsTsc({
		'tsconfig.json': BUNDLER_TSCONFIG,
		'node_modules/pkg/package.json':
			'{ "name": "pkg", "exports": { ".": { "types": "./index.d.ts" }, "./blocked": null } }',
		'node_modules/pkg/index.d.ts': 'export declare const root: number;',
		'node_modules/pkg/blocked.d.ts': 'export declare const blocked: number;',
		'src/index.ts': `
import { root } from "pkg";
import { blocked } from "pkg/blocked";
`,
	});
});

test('package_exports_exact_beats_pattern', async () => {
	await expectSameAsTsc({
		'tsconfig.json': BUNDLER_TSCONFIG,
		'node_modules/pkg/package.json': `{
  "name": "pkg",
  "exports": {
    "./features/*": { "types": "./dist/features/*.d.ts" },
    "./features/special": { "types": "./dist/special.d.ts" }
  }
}`,
		'node_modules/pkg/dist/features/auth.d.ts': 'export declare const auth: number;',
		'node_modules/pkg/dist/special.d.ts': 'export declare const special: string;',
		'src/index.ts': `
import { auth } from "pkg/features/auth";
import { special } from "pkg/features/special";
const useAuth: number = auth;
const useSpecial: string = special;
`,
	});
});

// Under node16, an `.mts` importer selects the `import` condition and a
// `.cts` importer selects `require` from the same package `exports` map.
test('node16_importer_flavor_selects_export_condition', async () => {
	await expectSameAsTsc({
		'tsconfig.json': `
{
  "compilerOptions": {
    "moduleResolution": "node16",
    "module": "node16",
    "strict": true,
    "noEmit": true
  },
  "include": ["src/**/*"]
}
`,
		'node_modules/pkg/package.json': `{
  "name": "pkg",
  "exports": {
    ".": {
      "import": { "types": "./import.d.mts" },
      "require": { "types": "./require.d.cts" }
    }
  }
}`,
		'node_modules/pkg/import.d.mts': 'export declare const flavor: "esm";',
		'node_modules/pkg/require.d.cts': 'export declare const flavor: "cjs";',
		'src/a.mts': 'import { flavor } from "pkg";\nconst f: "esm" = flavor;\n',
		'src/b.cts': 'import { flavor } from "pkg";\nconst f: "cjs" = flavor;\n',
	});
});

// Repeated runs over the same project must produce byte-identical JSON output
// (deterministic resolution and diagnostic ordering).
test('repeated_runs_are_deterministic', async () => {
	const root = tempProject({
		'tsconfig.json': PKG_TSCONFIG,
		'packages/a/node_modules/dep/package.json': '{ "name": "dep", "types": "./index.d.ts" }',
		'packages/a/node_modules/dep/index.d.ts': 'export declare const value: string;',
		'packages/b/node_modules/dep/package.json': '{ "name": "dep", "types": "./index.d.ts" }',
		'packages/b/node_modules/dep/index.d.ts': 'export declare const value: number;',
		'packages/a/src/index.ts': 'import { value } from "dep";\nconst wrong: number = value;\n',
		'packages/b/src/index.ts': 'import { value } from "dep";\nconst wrong: string = value;\n',
	});
	const args = [...PROJECT_ARGS, '--format', 'json'];

	const first = await surge(args, { cwd: root });
	for (let run = 0; run < 3; run += 1) {
		const next = await surge(args, { cwd: root });
		expect(next.stdout).toBe(first.stdout);
	}

	// Both importers keep their own (wrong) assignment.
	await expectSameDiagnosticsAsTsc(root, PROJECT_ARGS);
});
