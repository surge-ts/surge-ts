"use strict";

// `ts.FindAllReferences.Core.eachSymbolReferenceInFile` and
// `isSymbolReferencedInFile`, from TypeScript's language service (internal):
// the identifiers in a file that refer to a declaration's symbol.

const { SyntaxKind, SymbolFlags } = require("./enums.cjs");
const { forEachChild } = require("./nodes.cjs");
const { isParameterPropertyDeclaration } = require("./utilities.cjs");

const K = SyntaxKind;

/** The identifiers under `container`, in source order, named `escapedText`. */
function* identifiersNamed(container, escapedText) {
	const stack = [container];
	while (stack.length > 0) {
		const node = stack.pop();
		if (node.kind === K.Identifier) {
			if (node.escapedText === escapedText) yield node;
			continue;
		}
		const children = [];
		forEachChild(node, (child) => {
			children.push(child);
		});
		for (let index = children.length - 1; index >= 0; index--) stack.push(children[index]);
	}
}

/** tsc's `isExportSpecifierAlias`. */
function isExportSpecifierAlias(reference, specifier) {
	return specifier.propertyName ? specifier.propertyName === reference : !specifier.parent.parent.moduleSpecifier;
}

function eachSymbolReferenceInFile(definition, checker, sourceFile, cb, searchContainer = sourceFile) {
	const symbol = isParameterPropertyDeclaration(definition.parent, definition.parent.parent)
		? checker.resolveName(definition.text, definition, SymbolFlags.Value, false)
		: checker.getSymbolAtLocation(definition);
	if (!symbol) return undefined;
	for (const token of identifiersNamed(searchContainer, definition.escapedText)) {
		if (token === definition) continue;
		const referenceSymbol = checker.getSymbolAtLocation(token);
		const parent = token.parent;
		const matches =
			referenceSymbol === symbol ||
			(parent.kind === K.ShorthandPropertyAssignment && checker.getShorthandAssignmentValueSymbol(parent) === symbol) ||
			(parent.kind === K.ExportSpecifier &&
				((isExportSpecifierAlias(token, parent) && checker.getExportSpecifierLocalTargetSymbol(parent)) || referenceSymbol) === symbol);
		if (matches) {
			const result = cb(token);
			if (result) return result;
		}
	}
	return undefined;
}

function isSymbolReferencedInFile(definition, checker, sourceFile, searchContainer = sourceFile) {
	return eachSymbolReferenceInFile(definition, checker, sourceFile, () => true, searchContainer) || false;
}

module.exports = { FindAllReferences: { Core: { eachSymbolReferenceInFile, isSymbolReferencedInFile } } };
