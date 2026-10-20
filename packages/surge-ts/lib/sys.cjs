"use strict";

// `ts.sys`: the process's file system and environment, as TypeScript's
// `System` exposes them.
const fs = require("node:fs");
const path = require("node:path");

const useCaseSensitiveFileNames = process.platform !== "win32" && process.platform !== "darwin";

function toSlash(name) {
	return name.replace(/\\/g, "/");
}

function statOrUndefined(name) {
	try {
		return fs.statSync(name);
	} catch {
		return undefined;
	}
}

/** A `tsconfig`-style glob (`**`, `*`, `?`) as a regular expression over paths relative to `base`. */
function globToRegExp(pattern, base) {
	const absolute = toSlash(path.resolve(base, pattern)).replace(/\/$/, "");
	let source = "";
	for (let index = 0; index < absolute.length; index++) {
		const char = absolute[index];
		if (char === "*" && absolute[index + 1] === "*") {
			source += "(?:.*/)?";
			index += absolute[index + 2] === "/" ? 2 : 1;
		} else if (char === "*") {
			source += "[^/]*";
		} else if (char === "?") {
			source += "[^/]";
		} else {
			source += char.replace(/[.+^${}()|[\]\\]/g, "\\$&");
		}
	}
	return new RegExp(`^${source}(?:/.*)?$`, useCaseSensitiveFileNames ? "" : "i");
}

function readDirectory(root, extensions, excludes, includes, depth) {
	const base = path.resolve(root);
	const includePatterns = (includes && includes.length ? includes : ["**/*"]).map((pattern) => globToRegExp(pattern, base));
	const excludePatterns = (excludes ?? []).map((pattern) => globToRegExp(pattern, base));
	const results = [];
	const visit = (directory, level) => {
		let entries;
		try {
			entries = fs.readdirSync(directory, { withFileTypes: true });
		} catch {
			return;
		}
		entries.sort((left, right) => (left.name < right.name ? -1 : left.name > right.name ? 1 : 0));
		for (const entry of entries) {
			const full = toSlash(path.join(directory, entry.name));
			if (excludePatterns.some((pattern) => pattern.test(full))) continue;
			let stat = entry;
			if (entry.isSymbolicLink()) stat = statOrUndefined(full) ?? entry;
			if (stat.isDirectory()) {
				if (entry.name === "node_modules" || entry.name.startsWith(".")) continue;
				if (depth === undefined || level < depth) visit(full, level + 1);
			} else if (stat.isFile()) {
				if (extensions && extensions.length && !extensions.some((extension) => full.endsWith(extension))) continue;
				if (includePatterns.some((pattern) => pattern.test(full))) results.push(full);
			}
		}
	};
	visit(base, 1);
	return results;
}

const sys = {
	args: process.argv.slice(2),
	newLine: process.platform === "win32" ? "\r\n" : "\n",
	useCaseSensitiveFileNames,
	write(text) {
		process.stdout.write(text);
	},
	writeOutputIsTTY() {
		return Boolean(process.stdout.isTTY);
	},
	// tsc's: a SHA-256 of the text in hex.
	createHash(data) {
		return require("node:crypto").createHash("sha256").update(data).digest("hex");
	},
	createSHA256Hash(data) {
		return require("node:crypto").createHash("sha256").update(data).digest("hex");
	},
	// tsc's `readFile`: the byte order mark decides, UTF-16 in either byte
	// order or UTF-8 without the mark; the encoding argument is not read.
	readFile(fileName, _encoding) {
		let buffer;
		try {
			buffer = fs.readFileSync(fileName);
		} catch {
			return undefined;
		}
		if (buffer.length >= 2 && buffer[0] === 0xfe && buffer[1] === 0xff) {
			const length = buffer.length & ~1;
			for (let index = 0; index < length; index += 2) {
				const high = buffer[index];
				buffer[index] = buffer[index + 1];
				buffer[index + 1] = high;
			}
			return buffer.toString("utf16le", 2);
		}
		if (buffer.length >= 2 && buffer[0] === 0xff && buffer[1] === 0xfe) return buffer.toString("utf16le", 2);
		if (buffer.length >= 3 && buffer[0] === 0xef && buffer[1] === 0xbb && buffer[2] === 0xbf) return buffer.toString("utf8", 3);
		return buffer.toString("utf8");
	},
	writeFile(fileName, data, writeByteOrderMark) {
		fs.writeFileSync(fileName, writeByteOrderMark ? `﻿${data}` : data, "utf8");
	},
	resolvePath(name) {
		return toSlash(path.resolve(name));
	},
	fileExists(fileName) {
		return statOrUndefined(fileName)?.isFile() ?? false;
	},
	directoryExists(directoryName) {
		return statOrUndefined(directoryName)?.isDirectory() ?? false;
	},
	createDirectory(directoryName) {
		fs.mkdirSync(directoryName, { recursive: true });
	},
	getExecutingFilePath() {
		return toSlash(__filename);
	},
	getCurrentDirectory() {
		return toSlash(process.cwd());
	},
	getDirectories(directoryName) {
		try {
			return fs
				.readdirSync(directoryName, { withFileTypes: true })
				.filter((entry) => entry.isDirectory() || (entry.isSymbolicLink() && statOrUndefined(path.join(directoryName, entry.name))?.isDirectory()))
				.map((entry) => entry.name);
		} catch {
			return [];
		}
	},
	readDirectory,
	getModifiedTime(fileName) {
		return statOrUndefined(fileName)?.mtime;
	},
	deleteFile(fileName) {
		try {
			fs.unlinkSync(fileName);
		} catch {}
	},
	realpath(name) {
		try {
			return toSlash(fs.realpathSync.native(name));
		} catch {
			return name;
		}
	},
	getEnvironmentVariable(name) {
		return process.env[name] ?? "";
	},
	exit(exitCode) {
		process.exit(exitCode);
	},
};

module.exports = { sys, toSlash };
