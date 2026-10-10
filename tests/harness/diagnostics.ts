import { basename, isAbsolute, relative, resolve } from 'node:path';

export type Diagnostic = {
	file: string | null;
	line: number | null;
	column: number | null;
	code: string;
	message: string;
	/** 1-based inclusive end position, present only when parsed from pretty output. */
	end?: { line: number; column: number };
};

const PLAIN_HEADER =
	/^(?:(.+?)\((\d+),(\d+)\): )?(?:error|warning|message) (TS\d+|surge::[\w-]+): (.*)$/;

/** Parses `--pretty false` output, which tsc and surge print identically. */
export function parsePlainDiagnostics(output: string, projectDir: string): Diagnostic[] {
	const diagnostics: Diagnostic[] = [];
	for (const line of output.split(/\r?\n/)) {
		const match = PLAIN_HEADER.exec(line);
		if (match !== null) {
			const [, file, lineNo, column, code, message] = match;
			diagnostics.push({
				file: file === undefined ? null : normalizeFile(file, projectDir),
				line: lineNo === undefined ? null : Number(lineNo),
				column: column === undefined ? null : Number(column),
				code,
				message,
			});
			continue;
		}
		const last = diagnostics.at(-1);
		if (last !== undefined && /^\s+\S/.test(line)) {
			last.message += `\n${line}`;
		}
	}
	return diagnostics;
}

const ANSI = /\u001b\[[0-9;]*m/g;
const PRETTY_HEADER =
	/^(?:(.+?):(\d+):(\d+) - )?(?:error|warning|message) (TS\d+|surge::[\w-]+): (.*)$/;
const SOURCE_ROW = /^(\d+) (.*)$/;
const MARKER_ROW = /^( +)(~+)\s*$/;

/**
 * Parses `--pretty true` output to recover each diagnostic's end position from
 * the `~` underline, which is the only place tsc prints a span's extent.
 */
export function parsePrettyDiagnostics(output: string, projectDir: string): Diagnostic[] {
	const lines = output.replace(ANSI, '').split(/\r?\n/);
	const diagnostics: Diagnostic[] = [];
	let index = 0;
	while (index < lines.length) {
		const match = PRETTY_HEADER.exec(lines[index]);
		index += 1;
		if (match === null) continue;
		const [, file, lineNo, column, code, message] = match;
		const diagnostic: Diagnostic = {
			file: file === undefined ? null : normalizeFile(file, projectDir),
			line: lineNo === undefined ? null : Number(lineNo),
			column: column === undefined ? null : Number(column),
			code,
			message,
		};
		diagnostics.push(diagnostic);
		if (file === undefined) continue;

		while (index < lines.length && lines[index].trim() !== '') {
			diagnostic.message += `\n${lines[index]}`;
			index += 1;
		}
		index += 1;

		// The snippet: `N source` rows, each followed by a `~` row under the
		// spanned columns, ending at the next blank line.
		let gutter = 0;
		let sourceLine: number | null = null;
		while (index < lines.length && lines[index].trim() !== '') {
			const row = lines[index];
			index += 1;
			const source = SOURCE_ROW.exec(row);
			if (source !== null) {
				sourceLine = Number(source[1]);
				gutter = source[1].length + 1;
				continue;
			}
			const marker = MARKER_ROW.exec(row);
			if (marker === null || sourceLine === null) continue;
			const endColumn = marker[1].length - gutter + marker[2].length;
			diagnostic.end = { line: sourceLine, column: endColumn };
		}
	}
	return diagnostics;
}

function normalizeFile(file: string, projectDir: string): string {
	const absolute = isAbsolute(file) ? file : resolve(projectDir, file);
	const inProject = relative(projectDir, absolute);
	if (inProject.startsWith('..') || isAbsolute(inProject) || file.startsWith('<surge-lib>/')) {
		// Standard-library and other out-of-project files: tsc points into its
		// package, surge into its bundled snapshot (`<surge-lib>/` when no
		// installed package is found). Only the file name is shared.
		return `<external>/${basename(file)}`;
	}
	return inProject.split('\\').join('/');
}

export function formatDiagnostic(
	diagnostic: Diagnostic,
	fields: { messages: boolean; spans: boolean },
): string {
	let location = '';
	if (diagnostic.file !== null) {
		location = `${diagnostic.file}(${diagnostic.line},${diagnostic.column})`;
		if (fields.spans) {
			const end = diagnostic.end;
			location += end === undefined ? '-(?)' : `-(${end.line},${end.column})`;
		}
		location += ': ';
	}
	const text = `${location}${diagnostic.code}`;
	return fields.messages ? `${text}: ${diagnostic.message}` : text;
}
