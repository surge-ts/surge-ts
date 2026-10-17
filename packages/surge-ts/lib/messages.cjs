"use strict";

// `ts.Diagnostics`, TypeScript's diagnostic message table (typescript@6.0.3,
// generated into diagnostic-messages.json), and the internal helpers that
// build a diagnostic from one of its messages.

let table;

/** The message table, read on first use. */
function diagnostics() {
	if (table === undefined) {
		table = {};
		for (const [name, code, category, message, flags, key] of require("./diagnostic-messages.json")) {
			const entry = { code, category, key: key ?? `${name}_${code}`, message };
			if (flags & 1) entry.reportsUnnecessary = true;
			if (flags & 2) entry.elidedInCompatabilityPyramid = true;
			if (flags & 4) entry.reportsDeprecated = true;
			table[name] = entry;
		}
	}
	return table;
}

const getLocaleSpecificMessage = (message) => message.message;

function formatStringFromArgs(text, args) {
	return text.replace(/\{(\d+)\}/g, (_match, index) => {
		const argument = args[+index];
		if (argument === undefined) throw new Error(`surge-ts: no argument {${index}} for "${text}"`);
		return `${argument}`;
	});
}

function formatMessage(message, ...args) {
	const text = getLocaleSpecificMessage(message);
	return args.length > 0 ? formatStringFromArgs(text, args) : text;
}

function createFileDiagnostic(file, start, length, message, ...args) {
	return {
		file,
		start,
		length,
		messageText: formatMessage(message, ...args),
		category: message.category,
		code: message.code,
		reportsUnnecessary: message.reportsUnnecessary,
		reportsDeprecated: message.reportsDeprecated,
	};
}

function createCompilerDiagnostic(message, ...args) {
	return createFileDiagnostic(undefined, undefined, undefined, message, ...args);
}

module.exports = { diagnostics, getLocaleSpecificMessage, formatStringFromArgs, formatMessage, createFileDiagnostic, createCompilerDiagnostic };
