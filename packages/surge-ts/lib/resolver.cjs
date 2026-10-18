"use strict";

// TypeScript's `EmitResolver` (`checker.getEmitResolver()`), internal to
// TypeScript: the part tools read to decide what an emitter keeps.

const { SyntaxKind, SymbolFlags, ModifierFlags } = require("./enums.cjs");
const { forEachChild } = require("./nodes.cjs");
const { hasSyntacticModifier, isEntityNameExpression } = require("./utilities.cjs");
const { parseIsolatedEntityName } = require("./internals.cjs");

const K = SyntaxKind;

/** tsc's `isAliasSymbolDeclaration`, for TypeScript files. */
function isAliasSymbolDeclaration(node) {
	switch (node.kind) {
		case K.ImportEqualsDeclaration:
		case K.NamespaceExportDeclaration:
		case K.NamespaceImport:
		case K.NamespaceExport:
		case K.ImportSpecifier:
		case K.ExportSpecifier:
			return true;
		case K.ImportClause:
			return !!node.name;
		case K.ExportAssignment:
			return isEntityNameExpression(node.expression);
	}
	return false;
}

/** An alias declaration that binds a name in its file's scope: an import's. */
function isLocalAliasDeclaration(node) {
	switch (node.kind) {
		case K.ImportEqualsDeclaration:
		case K.NamespaceImport:
		case K.ImportSpecifier:
			return true;
		case K.ImportClause:
			return !!node.name;
	}
	return false;
}

/** A position tsc reads as a type, where an alias read marks nothing. */
function isInTypePosition(identifier) {
	for (let node = identifier; node.parent; node = node.parent) {
		const parent = node.parent;
		switch (parent.kind) {
			case K.ComputedPropertyName:
				return false;
			case K.ExpressionWithTypeArguments: {
				const clause = parent.parent;
				return clause.kind === K.HeritageClause && (clause.token === K.ImplementsKeyword || clause.parent.kind === K.InterfaceDeclaration);
			}
			case K.HeritageClause:
			case K.ImportEqualsDeclaration:
				return false;
		}
		if (parent.kind >= K.FirstTypeNode && parent.kind <= K.LastTypeNode) return true;
	}
	return false;
}

function isAliasDeclarationName(identifier) {
	const parent = identifier.parent;
	switch (parent?.kind) {
		case K.ImportSpecifier:
		case K.ImportClause:
		case K.NamespaceImport:
		case K.ImportEqualsDeclaration:
		case K.NamespaceExport:
		case K.ExportSpecifier:
			return true;
	}
	return false;
}

/** The first name of an entity name (`React` of `React.createElement`). */
function firstIdentifierText(entity) {
	while (entity.kind === K.QualifiedName) entity = entity.left;
	return entity.text;
}

function syntheticEntityName(namespace, member) {
	const identifier = (text) => ({ kind: K.Identifier, pos: -1, end: -1, flags: 16, text, escapedText: text });
	return { kind: K.QualifiedName, pos: -1, end: -1, flags: 16, left: identifier(namespace), right: identifier(member) };
}

class EmitResolver {
	constructor(checker) {
		this._checker = checker;
		this._options = checker._program.getCompilerOptions();
		this._referenced = new WeakMap();
	}

	/** The source file's `@jsx` / `@jsxFrag` pragma factory, if written. */
	_pragmaFactory(location, name) {
		const pragma = location?.getSourceFile?.().pragmas.get(name);
		const args = Array.isArray(pragma) ? pragma[0] : pragma;
		return args?.arguments.factory;
	}

	getJsxFactoryEntity(location) {
		const local = this._pragmaFactory(location, "jsx");
		const entity = local && parseIsolatedEntityName(local);
		if (entity) return entity;
		if (this._options.jsxFactory) {
			const configured = parseIsolatedEntityName(this._options.jsxFactory);
			if (configured) return configured;
		}
		return syntheticEntityName(this._options.reactNamespace || "React", "createElement");
	}

	getJsxFragmentFactoryEntity(location) {
		const local = this._pragmaFactory(location, "jsxfrag");
		const entity = local && parseIsolatedEntityName(local);
		if (entity) return entity;
		return this._options.jsxFragmentFactory ? parseIsolatedEntityName(this._options.jsxFragmentFactory) : undefined;
	}

	/** Whether an alias's target is a value tsc keeps the alias for. */
	_isKeptValueTarget(target) {
		// An alias that resolves nowhere: tsc's `unknownSymbol`, a value.
		if (target.flags & SymbolFlags.Alias) return true;
		if (!(target.flags & SymbolFlags.Value)) return false;
		if (this._options.isolatedModules || this._options.preserveConstEnums || this._options.verbatimModuleSyntax) return true;
		return !(target.flags & SymbolFlags.ConstEnum);
	}

	/** The aliases a file reads as values: tsc's `markAliasReferenced`. */
	_referencedAliases(sourceFile) {
		let referenced = this._referenced.get(sourceFile);
		if (referenced) return referenced;
		referenced = new Set();
		this._referenced.set(sourceFile, referenced);
		const checker = this._checker;
		// The aliases the file's names bind, by name: a read surge answers with
		// the alias's target (a class's `extends` clause) still reads the
		// alias. An export specifier binds no local name, so `export { x }`
		// never stands for the import of `x`.
		const aliasesByName = new Map();
		forEachChild(sourceFile, function collect(node) {
			if (isLocalAliasDeclaration(node) && node.name?.kind === K.Identifier && node.symbol) aliasesByName.set(node.name.text, node.symbol);
			forEachChild(node, collect);
		});
		const mark = (symbol, name) => {
			if (!symbol) return;
			if (!(symbol.flags & SymbolFlags.Alias)) {
				const alias = name !== undefined && aliasesByName.get(name);
				if (!alias || checker.getAliasedSymbol(alias) !== symbol) return;
				symbol = alias;
			}
			// tsc never marks an alias a type-only declaration leads through.
			if (checker.getTypeOnlyAliasDeclaration(symbol)) return;
			if (this._isKeptValueTarget(checker.getAliasedSymbol(symbol))) referenced.add(symbol);
		};
		let usesJsx = false;
		const visit = (node) => {
			switch (node.kind) {
				case K.JsxElement:
				case K.JsxSelfClosingElement:
				case K.JsxFragment:
					usesJsx = true;
					break;
				case K.ExportSpecifier:
					// tsc's `markExportAsReferenced`: a local `export { x }` reads `x`,
					// which is the import that binds `x` when one does.
					if (!node.isTypeOnly && !node.parent.parent.isTypeOnly && !node.parent.parent.moduleSpecifier) {
						const local = node.propertyName ?? node.name;
						const name = local.kind === K.Identifier ? local.text : undefined;
						mark(checker.getSymbolAtLocation(local), name);
						if (name !== undefined) mark(aliasesByName.get(name));
						mark(node.symbol);
					}
					return;
				case K.Identifier:
					if (aliasesByName.has(node.text) && !isAliasDeclarationName(node) && !isInTypePosition(node)) {
						const symbol =
							node.parent.kind === K.ShorthandPropertyAssignment && node.parent.name === node
								? checker.getShorthandAssignmentValueSymbol(node.parent)
								: checker.getSymbolAtLocation(node);
						mark(symbol, node.text);
					}
					return;
			}
			forEachChild(node, visit);
		};
		visit(sourceFile);
		// tsc's `markJsxAliasReferenced`: JSX reads its factory's namespace.
		if (usesJsx && (this._options.jsx === 2 || this._options.jsx === undefined)) {
			const namespace = firstIdentifierText(this.getJsxFactoryEntity(sourceFile));
			forEachChild(sourceFile, function find(node) {
				if (isAliasSymbolDeclaration(node) && node.name?.text === namespace && node.symbol) referenced.add(node.symbol);
				forEachChild(node, find);
			});
		}
		return referenced;
	}

	isReferencedAliasDeclaration(node, checkChildren) {
		if (isAliasSymbolDeclaration(node)) {
			const symbol = node.symbol;
			if (symbol && this._referencedAliases(node.getSourceFile()).has(symbol)) return true;
			if (symbol && hasSyntacticModifier(node, ModifierFlags.Export) && this._isKeptValueTarget(this._checker.getAliasedSymbol(symbol))) return true;
		}
		if (checkChildren) return !!forEachChild(node, (child) => this.isReferencedAliasDeclaration(child, checkChildren));
		return false;
	}
}

module.exports = { EmitResolver, isAliasSymbolDeclaration };
