// Differential harness for the compiler API: runs the same queries through
// typescript@6.0.3 (the last JavaScript compiler API) and surge-ts on each
// fixture project and compares what a tool would observe — node kinds and
// positions, `getSymbolAtLocation`, `getTypeOfSymbolAtLocation`,
// `getTypeAtLocation` — never either side's internal object shapes.
//
// Fixtures are split deterministically by name into a development set and a
// held-out set (every fifth, by hash); work on the API should be driven by
// development mismatches only, and the held-out score reported alongside.
//
// Usage:
//   tsx scripts/api/compare-api.ts [--filter <substr>] [--limit <n>] [--set dev|held-out|all]
//                                  [--show <n>] [--json <out.json>] [--dump <out.jsonl>]
//                                  [<fixture dir>...]
// `--dump` writes every mismatch, one JSON object per line, for clustering.
// Default fixtures: tests/compat-projects/*.

import { existsSync, readdirSync, realpathSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";
import v8 from "node:v8";
import { runInNewContext } from "node:vm";
import ts from "typescript-6";

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const require = createRequire(import.meta.url);
// biome-ignore lint: the facade is untyped JavaScript
const surge: any = require(path.join(root, "packages/surge-ts/index.cjs"));

interface Options {
	filter?: string;
	limit?: number;
	set: "dev" | "held-out" | "all";
	show: number;
	json?: string;
	dump?: string;
	fixtures: string[];
}

function parseArgs(argv: string[]): Options {
	const options: Options = { set: "all", show: 20, fixtures: [] };
	for (let index = 0; index < argv.length; index++) {
		const arg = argv[index];
		if (arg === "--filter") options.filter = argv[++index];
		else if (arg === "--limit") options.limit = Number(argv[++index]);
		else if (arg === "--set") options.set = argv[++index] as Options["set"];
		else if (arg === "--show") options.show = Number(argv[++index]);
		else if (arg === "--json") options.json = argv[++index];
		else if (arg === "--dump") options.dump = argv[++index];
		else options.fixtures.push(path.resolve(arg));
	}
	if (options.fixtures.length === 0) {
		const base = path.join(root, "tests/compat-projects");
		options.fixtures = readdirSync(base)
			.map((name) => path.join(base, name))
			.filter((dir) => existsSync(path.join(dir, "tsconfig.json")));
	}
	return options;
}

/**
 * A late-bound member's name (`__@iterator@27325`) ends in tsc's process-wide
 * symbol counter, which depends on every program the process built before;
 * only the key's name is comparable.
 */
function lateBoundName(name: string): string {
	return name.replace(/^(__@.+@)\d+$/, "$1#");
}

/** FNV-1a: a stable split of fixtures into development and held-out sets. */
function isHeldOut(name: string): boolean {
	let hash = 0x811c9dc5;
	for (let index = 0; index < name.length; index++) {
		hash ^= name.charCodeAt(index);
		hash = Math.imul(hash, 0x01000193) >>> 0;
	}
	return hash % 5 === 0;
}

const libCache = new Map<string, ts.SourceFile>();

function typescriptProgram(configPath: string): ts.Program | undefined {
	const parsed = ts.getParsedCommandLineOfConfigFile(configPath, {}, {
		...ts.sys,
		onUnRecoverableConfigFileDiagnostic: () => {},
	});
	if (!parsed) return undefined;
	const host = ts.createCompilerHost(parsed.options, true);
	const getSourceFile = host.getSourceFile;
	host.getSourceFile = (fileName, languageVersion, onError) => {
		const isLib = fileName.includes("/typescript-6/lib/");
		const key = `${fileName}\0${JSON.stringify(languageVersion)}`;
		if (isLib && libCache.has(key)) return libCache.get(key);
		const sourceFile = getSourceFile(fileName, languageVersion, onError);
		if (isLib && sourceFile) libCache.set(key, sourceFile);
		return sourceFile;
	};
	return ts.createProgram({ rootNames: parsed.fileNames, options: parsed.options, host });
}

// biome-ignore lint: surge's objects are untyped
function surgeProgram(configPath: string): any {
	const parsed = surge.getParsedCommandLineOfConfigFile(configPath, {}, surge.sys);
	if (!parsed) return undefined;
	return surge.createProgram({ rootNames: parsed.fileNames, options: parsed.options });
}

function canonical(fileName: string): string {
	try {
		return realpathSync(fileName);
	} catch {
		return fileName;
	}
}

interface NodeRecord {
	key: string;
	kind: string;
	// biome-ignore lint: either implementation's node
	node: any;
}

// biome-ignore lint: either implementation's node and forEachChild
function walk(sourceFile: any, forEachChild: any, kindName: (kind: number) => string): NodeRecord[] {
	const out: NodeRecord[] = [];
	// biome-ignore lint: either implementation's node
	const visit = (node: any) => {
		const kind = kindName(node.kind);
		out.push({ key: `${kind}@${node.getStart(sourceFile)}-${node.end}`, kind, node });
		forEachChild(node, visit);
	};
	forEachChild(sourceFile, visit);
	return out;
}

const typescriptKindNames = new Map<number, string>();
for (const [name, value] of Object.entries(ts.SyntaxKind)) {
	if (typeof value === "number" && !typescriptKindNames.has(value)) typescriptKindNames.set(value, name);
}
const surgeKindNames = new Map<number, string>();
for (const [name, value] of Object.entries(surge.SyntaxKind)) {
	if (typeof value === "number" && !surgeKindNames.has(value)) surgeKindNames.set(value, name);
}

/** Symbol flags both implementations are meant to agree on. */
const FLAG_MASK =
	ts.SymbolFlags.Variable |
	ts.SymbolFlags.Property |
	ts.SymbolFlags.EnumMember |
	ts.SymbolFlags.Function |
	ts.SymbolFlags.Class |
	ts.SymbolFlags.Interface |
	ts.SymbolFlags.Enum |
	ts.SymbolFlags.ValueModule |
	ts.SymbolFlags.NamespaceModule |
	ts.SymbolFlags.Method |
	ts.SymbolFlags.Accessor |
	ts.SymbolFlags.TypeParameter |
	ts.SymbolFlags.TypeAlias |
	ts.SymbolFlags.Alias;

const EXPRESSION_KINDS = new Set([
	"Identifier",
	"PropertyAccessExpression",
	"ElementAccessExpression",
	"CallExpression",
	"NewExpression",
	"ObjectLiteralExpression",
	"ArrayLiteralExpression",
	"StringLiteral",
	"NumericLiteral",
	"TemplateExpression",
	"NoSubstitutionTemplateLiteral",
	"BinaryExpression",
	"PrefixUnaryExpression",
	"ConditionalExpression",
	"ArrowFunction",
	"FunctionExpression",
	"AsExpression",
	"NonNullExpression",
	"AwaitExpression",
	"ParenthesizedExpression",
	"TrueKeyword",
	"FalseKeyword",
	"NullKeyword",
]);

/** Every mismatch as a JSON line, when `--dump` asks for them. */
let dumpLines: string[] | undefined;

class Metric {
	total = 0;
	matched = 0;
	mismatches: string[] = [];
	byKind = new Map<string, [number, number]>();
	constructor(readonly name: string) {}
	record(kind: string, matched: boolean, detail: () => string, data?: () => Record<string, unknown>) {
		this.total++;
		const entry = this.byKind.get(kind) ?? [0, 0];
		entry[0]++;
		if (matched) {
			this.matched++;
			entry[1]++;
		} else {
			if (this.mismatches.length < 2000) this.mismatches.push(detail());
			if (dumpLines && data) dumpLines.push(JSON.stringify({ metric: this.name, kind, ...data() }));
		}
		this.byKind.set(kind, entry);
	}
	rate() {
		return this.total ? (100 * this.matched) / this.total : 100;
	}
}

interface Metrics {
	structure: Metric;
	symbolPresence: Metric;
	symbolName: Metric;
	symbolFlags: Metric;
	symbolDeclaration: Metric;
	typeOfSymbol: Metric;
	typeAtLocation: Metric;
}

function newMetrics(): Metrics {
	return {
		structure: new Metric("structure"),
		symbolPresence: new Metric("symbolPresence"),
		symbolName: new Metric("symbolName"),
		symbolFlags: new Metric("symbolFlags"),
		symbolDeclaration: new Metric("symbolDeclaration"),
		typeOfSymbol: new Metric("typeOfSymbol"),
		typeAtLocation: new Metric("typeAtLocation"),
	};
}

// biome-ignore lint: either implementation's symbol
function declarationKey(symbol: any, fixture: string): string | undefined {
	const declaration = symbol.declarations?.[0];
	if (!declaration) return "none";
	const file = declaration.getSourceFile().fileName as string;
	const canonicalFile = canonical(file);
	if (!canonicalFile.startsWith(fixture)) return undefined;
	return `${path.relative(fixture, canonicalFile)}:${declaration.getStart()}`;
}

function compareFixture(fixture: string, metrics: Metrics, errors: string[]) {
	const configPath = path.join(fixture, "tsconfig.json");
	let typescript: ts.Program | undefined;
	// biome-ignore lint: surge's program
	let program: any;
	try {
		typescript = typescriptProgram(configPath);
		program = surgeProgram(configPath);
	} catch (error) {
		errors.push(`${path.basename(fixture)}: ${(error as Error).message}`);
		return;
	}
	if (!typescript || !program) return;
	const tsChecker = typescript.getTypeChecker();
	const surgeChecker = program.getTypeChecker();
	const fixtureRoot = canonical(fixture);
	for (const tsFile of typescript.getSourceFiles()) {
		const fileName = canonical(tsFile.fileName);
		if (tsFile.isDeclarationFile || !fileName.startsWith(fixtureRoot) || fileName.includes("/node_modules/")) continue;
		if (/[^\x00-\x7f]/.test(tsFile.text)) continue;
		const surgeFile = program.getSourceFile(fileName) ?? program.getSourceFile(tsFile.fileName);
		const label = path.relative(fixtureRoot, fileName);
		const where = `${path.basename(fixture)}/${label}`;
		if (!surgeFile) {
			metrics.structure.record("SourceFile", false, () => `${where}: no surge source file`);
			continue;
		}
		const tsNodes = walk(tsFile, ts.forEachChild, (kind) => typescriptKindNames.get(kind) ?? String(kind));
		const surgeNodes = walk(surgeFile, surge.forEachChild, (kind) => surgeKindNames.get(kind) ?? String(kind));
		const surgeByKey = new Map(surgeNodes.map((record) => [record.key, record]));
		for (const record of tsNodes) {
			const match = surgeByKey.get(record.key);
			metrics.structure.record(record.kind, match !== undefined, () => `${where}: TypeScript node ${record.key} has no surge counterpart`);
			if (!match) continue;
			const tsSymbol = tsChecker.getSymbolAtLocation(record.node);
			const surgeSymbol = surgeChecker.getSymbolAtLocation(match.node);
			const text = () => record.node.getText().slice(0, 40).replace(/\s+/g, " ");
			const parent = () => typescriptKindNames.get(record.node.parent?.kind) ?? "none";
			metrics.symbolPresence.record(
				record.kind,
				Boolean(tsSymbol) === Boolean(surgeSymbol),
				() => `${where}: ${record.key} \`${text()}\` symbol ${tsSymbol ? tsSymbol.getName() : "none"} vs ${surgeSymbol ? surgeSymbol.getName() : "none"}`,
				() => ({ where, key: record.key, text: text(), ts: tsSymbol?.getName() ?? null, surge: surgeSymbol?.getName() ?? null }),
			);
			if (tsSymbol && surgeSymbol) {
				metrics.symbolName.record(
					record.kind,
					lateBoundName(tsSymbol.getName()) === lateBoundName(surgeSymbol.getName()),
					() => `${where}: ${record.key} name ${tsSymbol.getName()} vs ${surgeSymbol.getName()}`,
					() => ({ where, key: record.key, text: text(), parent: parent(), ts: tsSymbol.getName(), surge: surgeSymbol.getName() }),
				);
				const tsFlags = tsSymbol.flags & FLAG_MASK;
				const surgeFlags = surgeSymbol.flags & FLAG_MASK;
				metrics.symbolFlags.record(
					record.kind,
					tsFlags === surgeFlags,
					() => `${where}: ${record.key} \`${text()}\` flags ${tsFlags} vs ${surgeFlags}`,
					() => ({ where, key: record.key, text: text(), ts: tsFlags, surge: surgeFlags }),
				);
				const tsDeclaration = declarationKey(tsSymbol, fixtureRoot);
				const surgeDeclaration = declarationKey(surgeSymbol, fixtureRoot);
				if (tsDeclaration !== undefined && surgeDeclaration !== undefined) {
					metrics.symbolDeclaration.record(
						record.kind,
						tsDeclaration === surgeDeclaration,
						() => `${where}: ${record.key} \`${text()}\` declaration ${tsDeclaration} vs ${surgeDeclaration}`,
						() => ({ where, key: record.key, text: text(), ts: tsDeclaration, surge: surgeDeclaration }),
					);
				}
				const tsType = tsChecker.typeToString(tsChecker.getTypeOfSymbolAtLocation(tsSymbol, record.node));
				const surgeType = surgeChecker.typeToString(surgeChecker.getTypeOfSymbolAtLocation(surgeSymbol, match.node));
				metrics.typeOfSymbol.record(
					record.kind,
					tsType === surgeType,
					() => `${where}: ${record.key} \`${text()}\` ${tsType} vs ${surgeType}`,
					() => ({ where, key: record.key, text: text(), parent: parent(), ts: tsType, surge: surgeType }),
				);
			}
			if (EXPRESSION_KINDS.has(record.kind)) {
				const tsType = tsChecker.typeToString(tsChecker.getTypeAtLocation(record.node));
				const surgeType = surgeChecker.typeToString(surgeChecker.getTypeAtLocation(match.node));
				metrics.typeAtLocation.record(
					record.kind,
					tsType === surgeType,
					() => `${where}: ${record.key} \`${text()}\` ${tsType} vs ${surgeType}`,
					() => ({ where, key: record.key, text: text(), parent: parent(), ts: tsType, surge: surgeType }),
				);
			}
		}
	}
}

function report(label: string, metrics: Metrics, show: number) {
	console.log(`\n== ${label}`);
	for (const [name, metric] of Object.entries(metrics)) {
		console.log(`  ${name.padEnd(18)} ${metric.rate().toFixed(1).padStart(6)}%  (${metric.matched}/${metric.total})`);
	}
	if (show > 0) {
		for (const [name, metric] of Object.entries(metrics)) {
			if (metric.mismatches.length === 0) continue;
			console.log(`\n  -- ${name} mismatches (first ${Math.min(show, metric.mismatches.length)} of ${metric.total - metric.matched})`);
			for (const line of metric.mismatches.slice(0, show)) console.log(`     ${line}`);
			const worst = [...metric.byKind.entries()]
				.map(([kind, [total, matched]]) => [kind, total - matched, total] as const)
				.filter(([, missed]) => missed > 0)
				.sort((left, right) => right[1] - left[1])
				.slice(0, 8);
			console.log(`     by kind: ${worst.map(([kind, missed, total]) => `${kind} ${missed}/${total}`).join(", ")}`);
		}
	}
}

// A surge program's native state is freed by its finalizer, which Node runs
// only between turns of the event loop: a loop that compares every fixture in
// one turn keeps all of them alive (tens of GB). Collect and yield after each.
v8.setFlagsFromString("--expose-gc");
const collectGarbage = runInNewContext("gc") as () => void;

function releasePrograms(): Promise<void> {
	collectGarbage();
	return new Promise((resolve) => setImmediate(resolve));
}

async function main() {
	const options = parseArgs(process.argv.slice(2));
	if (options.dump) dumpLines = [];
	let fixtures = options.fixtures.filter((dir) => !options.filter || path.basename(dir).includes(options.filter));
	if (options.set !== "all") fixtures = fixtures.filter((dir) => isHeldOut(path.basename(dir)) === (options.set === "held-out"));
	if (options.limit) fixtures = fixtures.slice(0, options.limit);
	const development = newMetrics();
	const heldOut = newMetrics();
	const errors: string[] = [];
	const started = Date.now();
	for (const [index, fixture] of fixtures.entries()) {
		compareFixture(fixture, isHeldOut(path.basename(fixture)) ? heldOut : development, errors);
		await releasePrograms();
		if ((index + 1) % 50 === 0) console.error(`${index + 1}/${fixtures.length} fixtures (${((Date.now() - started) / 1000).toFixed(0)}s)`);
	}
	if (options.set !== "held-out") report("development", development, options.show);
	if (options.set !== "dev") report("held-out", heldOut, options.set === "held-out" ? options.show : 0);
	if (errors.length) console.log(`\n${errors.length} fixtures failed:\n  ${errors.slice(0, 20).join("\n  ")}`);
	if (options.dump && dumpLines) writeFileSync(options.dump, dumpLines.length ? `${dumpLines.join("\n")}\n` : "");
	if (options.json) {
		const summary = (metrics: Metrics) =>
			Object.fromEntries(Object.entries(metrics).map(([name, metric]) => [name, { total: metric.total, matched: metric.matched, rate: metric.rate() }]));
		writeFileSync(
			options.json,
			`${JSON.stringify({ fixtures: fixtures.length, development: summary(development), heldOut: summary(heldOut), errors }, null, 2)}\n`,
		);
	}
}

await main();
