"use strict";

// tsc's emit resolver for declaration emit (checker.ts `createResolver`) and
// the node builder it prints inferred types with (`typeToTypeNodeHelper`,
// expressionToTypeNode.ts), over surge's checker: which declarations a
// declaration file keeps, which names reach a symbol from where it is
// written (and the imports that keeps), and what an inferred type prints as.
const {
	SyntaxKind: K,
	SymbolFlags: SF,
	ModifierFlags,
	NodeFlags,
	TypeFlags: TF,
	ObjectFlags: OF,
	ElementFlags,
	SignatureKind,
	ModuleResolutionKind,
} = require("../enums.cjs");
const paths = require("../paths.cjs");
const f = require("./factory.cjs");
const emitPaths = require("./paths.cjs");
const syntax = require("./declarations.cjs");

const { modifierFlags, isFunctionLike, isFunctionLikeDeclarationKind, isDeclaration, hasInferredType, isPrimitiveLiteralValue, isAmbientModule } = syntax;

const Accessibility = { Accessible: 0, NotAccessible: 1, CannotBeNamed: 2, NotResolved: 3 };

const unsupported = (what) => new Error(`surge-ts: declaration emit of ${what} is not supported yet`);

const hasSyntacticModifier = (node, flag) => (modifierFlags(node) & flag) !== 0;
const isBindingPattern = (node) => !!node && (node.kind === K.ObjectBindingPattern || node.kind === K.ArrayBindingPattern);
const getQualifiedLeftMeaning = (meaning) => (meaning === SF.Value ? SF.Value : SF.Namespace);
const unescape = (name) => (name.length >= 3 && name.charCodeAt(0) === 95 && name.charCodeAt(1) === 95 && name.charCodeAt(2) === 95 ? name.slice(1) : name);

function isExternalOrCommonJsModule(file) {
	return file.externalModuleIndicator !== undefined;
}
const isGlobalSourceFile = (node) => node.kind === K.SourceFile && !isExternalOrCommonJsModule(node);
const isModuleWithStringLiteralName = (node) => node.kind === K.ModuleDeclaration && node.name.kind === K.StringLiteral;
const hasNonGlobalAugmentationExternalModuleSymbol = (declaration) =>
	isModuleWithStringLiteralName(declaration) || (declaration.kind === K.SourceFile && isExternalOrCommonJsModule(declaration));
const hasExternalModuleSymbol = (declaration) => isAmbientModule(declaration) || (declaration.kind === K.SourceFile && isExternalOrCommonJsModule(declaration));

function isExternalModuleAugmentation(node) {
	if (!isAmbientModule(node)) return false;
	switch (node.parent.kind) {
		case K.SourceFile:
			return isExternalOrCommonJsModule(node.parent);
		case K.ModuleBlock:
			return isAmbientModule(node.parent.parent) && node.parent.parent.parent.kind === K.SourceFile && !isExternalOrCommonJsModule(node.parent.parent.parent);
	}
	return false;
}

function findAncestor(node, callback) {
	while (node) {
		const result = callback(node);
		if (result === "quit") return undefined;
		if (result) return node;
		node = node.parent;
	}
	return undefined;
}

function getRootDeclaration(node) {
	while (node.kind === K.BindingElement) node = node.parent.parent;
	return node;
}

/** tsc's `getDeclarationContainer`. */
function getDeclarationContainer(node) {
	return findAncestor(getRootDeclaration(node), (ancestor) => {
		switch (ancestor.kind) {
			case K.VariableDeclaration:
			case K.VariableDeclarationList:
			case K.ImportSpecifier:
			case K.NamedImports:
			case K.NamespaceImport:
			case K.ImportClause:
				return false;
			default:
				return true;
		}
	}).parent;
}

/** tsc's `getCombinedModifierFlags`. */
function combinedModifierFlags(node) {
	while (node.kind === K.BindingElement || isBindingPattern(node)) node = node.parent;
	let flags = modifierFlags(node);
	if (node.kind === K.VariableDeclaration) node = node.parent;
	if (node && node.kind === K.VariableDeclarationList) {
		flags |= modifierFlags(node);
		node = node.parent;
	}
	if (node && node.kind === K.VariableStatement) flags |= modifierFlags(node);
	return flags;
}

/** tsc's `getCombinedNodeFlags`. */
function combinedNodeFlags(node) {
	while (node.kind === K.BindingElement || isBindingPattern(node)) node = node.parent;
	let flags = node.flags;
	if (node.kind === K.VariableDeclaration) node = node.parent;
	if (node && node.kind === K.VariableDeclarationList) {
		flags |= node.flags;
		node = node.parent;
	}
	if (node && node.kind === K.VariableStatement) flags |= node.flags;
	return flags;
}

function isVarConstLike(node) {
	const blockScope = combinedNodeFlags(node) & NodeFlags.BlockScoped;
	return blockScope === NodeFlags.Const || blockScope === NodeFlags.Using || blockScope === NodeFlags.AwaitUsing;
}

const isParameterPropertyDeclaration = (node, parent) =>
	node.kind === K.Parameter && hasSyntacticModifier(node, ModifierFlags.ParameterPropertyModifier) && parent.kind === K.Constructor;

function isDeclarationReadonly(node) {
	return !!(combinedModifierFlags(node) & ModifierFlags.Readonly && !isParameterPropertyDeclaration(node, node.parent));
}

/** tsc's `getAnyImportSyntax`. */
function getAnyImportSyntax(node) {
	switch (node.kind) {
		case K.ImportEqualsDeclaration:
			return node;
		case K.ImportClause:
			return node.parent;
		case K.NamespaceImport:
			return node.parent.parent;
		case K.ImportSpecifier:
			return node.parent.parent.parent;
	}
	return undefined;
}

function getFirstIdentifier(node) {
	switch (node.kind) {
		case K.Identifier:
			return node;
		case K.QualifiedName:
			do node = node.left;
			while (node.kind !== K.Identifier);
			return node;
		case K.PropertyAccessExpression:
			do node = node.expression;
			while (node.kind !== K.Identifier);
			return node;
	}
	return undefined;
}

/** tsc's `isPartOfTypeNode` for an `ExpressionWithTypeArguments`: written in a type position. */
function isExpressionWithTypeArgumentsInTypePosition(node) {
	const clause = node.parent;
	if (clause.kind !== K.HeritageClause) return false;
	return !(clause.token === K.ExtendsKeyword && (clause.parent.kind === K.ClassDeclaration || clause.parent.kind === K.ClassExpression));
}

function isPropertyOrMethodDeclarationSymbol(symbol) {
	const declarations = symbol.declarations;
	if (!declarations || !declarations.length) return false;
	for (const declaration of declarations) {
		switch (declaration.kind) {
			case K.PropertyDeclaration:
			case K.MethodDeclaration:
			case K.GetAccessor:
			case K.SetAccessor:
				continue;
			default:
				return false;
		}
	}
	return true;
}

/** tsc's `isContextuallyTyped` (expressionToTypeNode.ts). */
function isContextuallyTyped(node) {
	return !!findAncestor(
		node.parent,
		(ancestor) =>
			ancestor.kind === K.CallExpression ||
			(!isFunctionLikeDeclarationKind(ancestor.kind) && ancestor.type !== undefined && typeof ancestor.type === "object") ||
			ancestor.kind === K.JsxElement ||
			ancestor.kind === K.JsxExpression,
	);
}

function walkUpParenthesizedExpressions(node) {
	while (node && node.kind === K.ParenthesizedExpression) node = node.parent;
	return node;
}

function isOptionalDeclaration(declaration) {
	switch (declaration.kind) {
		case K.PropertyDeclaration:
		case K.PropertySignature:
			return !!declaration.questionToken;
		case K.Parameter:
			return !!declaration.questionToken;
	}
	return false;
}

/** tsc's `forEachReturnStatement`. */
function forEachReturnStatement(body, visitor) {
	const traverse = (node) => {
		switch (node.kind) {
			case K.ReturnStatement:
				return visitor(node);
			case K.CaseBlock:
			case K.Block:
			case K.IfStatement:
			case K.DoStatement:
			case K.WhileStatement:
			case K.ForStatement:
			case K.ForInStatement:
			case K.ForOfStatement:
			case K.WithStatement:
			case K.SwitchStatement:
			case K.CaseClause:
			case K.DefaultClause:
			case K.LabeledStatement:
			case K.TryStatement:
			case K.CatchClause:
				return node.forEachChild(traverse);
		}
		return undefined;
	};
	return traverse(body);
}

const isValueSignatureDeclaration = (node) =>
	node.kind === K.FunctionExpression ||
	node.kind === K.ArrowFunction ||
	node.kind === K.MethodDeclaration ||
	node.kind === K.GetAccessor ||
	node.kind === K.SetAccessor ||
	node.kind === K.FunctionDeclaration ||
	node.kind === K.Constructor;

const isKeywordKind = (kind) => kind >= K.FirstKeyword && kind <= K.LastKeyword;

function canAddUndefined(node, strictNullChecks) {
	if (!strictNullChecks) return true;
	if (
		isKeywordKind(node.kind) ||
		node.kind === K.LiteralType ||
		node.kind === K.FunctionType ||
		node.kind === K.ConstructorType ||
		node.kind === K.ArrayType ||
		node.kind === K.TupleType ||
		node.kind === K.TypeLiteral ||
		node.kind === K.TemplateLiteralType ||
		node.kind === K.ThisType
	) {
		return true;
	}
	if (node.kind === K.ParenthesizedType) return canAddUndefined(node.type, strictNullChecks);
	if (node.kind === K.UnionType || node.kind === K.IntersectionType) return node.types.every((type) => canAddUndefined(type, strictNullChecks));
	return false;
}

/** The module specifiers a file writes: tsc's `SourceFile.imports`. */
function moduleSpecifiersOf(file) {
	const specifiers = [];
	const visit = (node) => {
		switch (node.kind) {
			case K.ImportDeclaration:
			case K.ExportDeclaration:
				if (node.moduleSpecifier && node.moduleSpecifier.kind === K.StringLiteral) specifiers.push(node.moduleSpecifier);
				return;
			case K.ImportEqualsDeclaration:
				if (node.moduleReference.kind === K.ExternalModuleReference && node.moduleReference.expression.kind === K.StringLiteral) {
					specifiers.push(node.moduleReference.expression);
				}
				return;
			case K.ModuleDeclaration:
				if (node.body) visit(node.body);
				return;
			case K.ModuleBlock:
				for (const statement of node.statements) visit(statement);
				return;
		}
	};
	for (const statement of file.statements) visit(statement);
	const dynamic = (node) => {
		if (node.kind === K.CallExpression && node.expression.kind === K.ImportKeyword && node.arguments.length && node.arguments[0].kind === K.StringLiteral) {
			specifiers.push(node.arguments[0]);
		} else if (node.kind === K.ImportType && node.argument.kind === K.LiteralType && node.argument.literal.kind === K.StringLiteral) {
			specifiers.push(node.argument.literal);
		}
		node.forEachChild(dynamic);
	};
	file.forEachChild(dynamic);
	return specifiers;
}

class DeclarationEmitResolver {
	constructor(program) {
		this._program = program;
		this._checker = program.getTypeChecker();
		this._native = this._checker._native;
		this._options = program.getCompilerOptions();
		this._strictNullChecks = this._options.strictNullChecks ?? !!this._options.strict;
		// tsc's NodeLinks.isVisible and SymbolLinks caches: the checker's, shared by every emit of the program.
		this._visible = new Map();
		this._locals = new Map();
		this._accessibleChains = new Map();
		this._specifiers = new Map();
		this._extendedContainers = new Map();
		this._nodeIds = new WeakMap();
		this._nextNodeId = 1;
		this._globals = { isGlobals: true, get: (name) => this._checker.resolveName(unescape(name), undefined, SF.Value | SF.Type | SF.Namespace) };
	}

	_nodeId(node) {
		let id = this._nodeIds.get(node);
		if (id === undefined) this._nodeIds.set(node, (id = this._nextNodeId++));
		return id;
	}

	// --- visibility ------------------------------------------------------------

	/** tsc's `isDeclarationVisible`. */
	isDeclarationVisible(node) {
		if (!node) return false;
		let visible = this._visible.get(node);
		if (visible === undefined) {
			visible = !!this._determineIfDeclarationIsVisible(node);
			this._visible.set(node, visible);
		}
		return visible;
	}

	_determineIfDeclarationIsVisible(node) {
		switch (node.kind) {
			case K.JSDocCallbackTag:
			case K.JSDocTypedefTag:
			case K.JSDocEnumTag:
				return !!(node.parent && node.parent.parent && node.parent.parent.parent && node.parent.parent.parent.kind === K.SourceFile);
			case K.BindingElement:
				return this.isDeclarationVisible(node.parent.parent);
			case K.VariableDeclaration:
				if (isBindingPattern(node.name) && !node.name.elements.length) return false;
			// falls through
			case K.ModuleDeclaration:
			case K.ClassDeclaration:
			case K.InterfaceDeclaration:
			case K.TypeAliasDeclaration:
			case K.FunctionDeclaration:
			case K.EnumDeclaration:
			case K.ImportEqualsDeclaration: {
				if (isExternalModuleAugmentation(node)) return true;
				const parent = getDeclarationContainer(node);
				if (
					!(combinedModifierFlags(node) & ModifierFlags.Export) &&
					!(node.kind !== K.ImportEqualsDeclaration && parent.kind !== K.SourceFile && parent.flags & NodeFlags.Ambient)
				) {
					return isGlobalSourceFile(parent);
				}
				return this.isDeclarationVisible(parent);
			}
			case K.PropertyDeclaration:
			case K.PropertySignature:
			case K.GetAccessor:
			case K.SetAccessor:
			case K.MethodDeclaration:
			case K.MethodSignature:
				if (hasSyntacticModifier(node, ModifierFlags.Private | ModifierFlags.Protected)) return false;
			// falls through
			case K.Constructor:
			case K.ConstructSignature:
			case K.CallSignature:
			case K.IndexSignature:
			case K.Parameter:
			case K.ModuleBlock:
			case K.FunctionType:
			case K.ConstructorType:
			case K.TypeLiteral:
			case K.TypeReference:
			case K.ArrayType:
			case K.TupleType:
			case K.UnionType:
			case K.IntersectionType:
			case K.ParenthesizedType:
			case K.NamedTupleMember:
				return this.isDeclarationVisible(node.parent);
			case K.ImportClause:
			case K.NamespaceImport:
			case K.ImportSpecifier:
				return false;
			case K.TypeParameter:
			case K.SourceFile:
			case K.NamespaceExportDeclaration:
				return true;
			case K.ExportAssignment:
				return false;
		}
		return false;
	}

	/** tsc's `hasVisibleDeclarations`. */
	_hasVisibleDeclarations(symbol, shouldComputeAliasToMakeVisible) {
		let aliasesToMakeVisible;
		const addVisibleAlias = (declaration, aliasingStatement) => {
			if (shouldComputeAliasToMakeVisible) {
				this._visible.set(declaration, true);
				aliasesToMakeVisible ??= [];
				if (!aliasesToMakeVisible.includes(aliasingStatement)) aliasesToMakeVisible.push(aliasingStatement);
			}
			return true;
		};
		const isVisible = (declaration) => {
			if (this.isDeclarationVisible(declaration)) return true;
			const anyImportSyntax = getAnyImportSyntax(declaration);
			if (anyImportSyntax && !hasSyntacticModifier(anyImportSyntax, ModifierFlags.Export) && this.isDeclarationVisible(anyImportSyntax.parent)) {
				return addVisibleAlias(declaration, anyImportSyntax);
			}
			if (
				declaration.kind === K.VariableDeclaration &&
				declaration.parent.parent.kind === K.VariableStatement &&
				!hasSyntacticModifier(declaration.parent.parent, ModifierFlags.Export) &&
				this.isDeclarationVisible(declaration.parent.parent.parent)
			) {
				return addVisibleAlias(declaration, declaration.parent.parent);
			}
			if (
				syntax.isLateVisibilityPaintedStatement(declaration) &&
				!hasSyntacticModifier(declaration, ModifierFlags.Export) &&
				this.isDeclarationVisible(declaration.parent)
			) {
				return addVisibleAlias(declaration, declaration);
			}
			if (declaration.kind === K.BindingElement) {
				if (symbol.flags & SF.Alias && declaration.parent.parent.kind === K.VariableDeclaration && declaration.parent.parent.parent.parent.kind === K.VariableStatement) {
					const statement = declaration.parent.parent.parent.parent;
					if (!hasSyntacticModifier(statement, ModifierFlags.Export) && this.isDeclarationVisible(statement.parent)) return addVisibleAlias(declaration, statement);
				} else if (symbol.flags & SF.Variable) {
					const rootDeclaration = getRootDeclaration(declaration);
					if (rootDeclaration.kind === K.Parameter) return false;
					const statement = rootDeclaration.parent.parent;
					if (statement.kind !== K.VariableStatement) return false;
					if (hasSyntacticModifier(statement, ModifierFlags.Export)) return true;
					if (!this.isDeclarationVisible(statement.parent)) return false;
					return addVisibleAlias(declaration, statement);
				}
			}
			return false;
		};
		const declarations = (symbol.declarations ?? []).filter((declaration) => declaration.kind !== K.Identifier);
		if (!declarations.every(isVisible)) return undefined;
		return { accessibility: Accessibility.Accessible, aliasesToMakeVisible };
	}

	/** tsc's `isEntityNameVisible`. */
	isEntityNameVisible(entityName, enclosingDeclaration, shouldComputeAliasToMakeVisible = true) {
		const parent = entityName.parent;
		let meaning;
		if (
			parent.kind === K.TypeQuery ||
			(parent.kind === K.ExpressionWithTypeArguments && !isExpressionWithTypeArgumentsInTypePosition(parent)) ||
			parent.kind === K.ComputedPropertyName ||
			(parent.kind === K.TypePredicate && parent.parameterName === entityName)
		) {
			meaning = SF.Value | SF.ExportValue;
		} else if (
			entityName.kind === K.QualifiedName ||
			entityName.kind === K.PropertyAccessExpression ||
			parent.kind === K.ImportEqualsDeclaration ||
			(parent.kind === K.QualifiedName && parent.left === entityName) ||
			(parent.kind === K.PropertyAccessExpression && parent.expression === entityName) ||
			(parent.kind === K.ElementAccessExpression && parent.expression === entityName)
		) {
			meaning = SF.Namespace;
		} else {
			meaning = SF.Type;
		}
		const firstIdentifier = getFirstIdentifier(entityName);
		const symbol = this._checker.resolveName(firstIdentifier.text ?? unescape(firstIdentifier.escapedText), enclosingDeclaration, meaning, false);
		if (symbol && symbol.flags & SF.TypeParameter && meaning & SF.Type) return { accessibility: Accessibility.Accessible };
		if (!symbol) {
			return { accessibility: Accessibility.NotResolved, errorSymbolName: firstIdentifier.text, errorNode: firstIdentifier };
		}
		return (
			this._hasVisibleDeclarations(symbol, shouldComputeAliasToMakeVisible) || {
				accessibility: Accessibility.NotAccessible,
				errorSymbolName: firstIdentifier.text,
				errorNode: firstIdentifier,
			}
		);
	}

	// --- accessibility ---------------------------------------------------------

	_resolveAlias(symbol) {
		return symbol.flags & SF.Alias ? (this._checker.getAliasedSymbol(symbol) ?? symbol) : symbol;
	}

	_mergedSymbol(symbol) {
		return symbol && this._checker.getMergedSymbol(symbol);
	}

	/** tsc's `getSymbolIfSameReference`. */
	_isSameReference(s1, s2) {
		return this._mergedSymbol(this._resolveAlias(this._mergedSymbol(s1))) === this._mergedSymbol(this._resolveAlias(this._mergedSymbol(s2)));
	}

	_localsTable(node) {
		let table = this._locals.get(node);
		if (table === undefined) {
			table = null;
			if (node.kind !== K.SourceFile || node._program === this._program) {
				const [file, index] = this._checker._node(node);
				const ids = this._native.nodeLocals(file, index);
				if (ids.length) {
					table = new Map();
					for (const id of ids) {
						const symbol = this._checker._symbol(id);
						table.set(symbol.escapedName, symbol);
					}
				}
			}
			this._locals.set(node, table);
		}
		return table ?? undefined;
	}

	/** tsc's `getExportsOfSymbol`. */
	_exportsOf(symbol) {
		if (symbol.flags & SF.Module) {
			const table = new Map();
			for (const exported of this._checker.getExportsOfModule(symbol)) table.set(exported.escapedName, exported);
			return table;
		}
		return symbol.exports ?? new Map();
	}

	/** tsc's `forEachSymbolTableInScope`. */
	_forEachSymbolTableInScope(enclosingDeclaration, callback) {
		let result;
		for (let location = enclosingDeclaration; location; location = location.parent) {
			if (!isGlobalSourceFile(location)) {
				const locals = this._localsTable(location);
				if (locals && (result = callback(locals, undefined, true, location))) return result;
			}
			switch (location.kind) {
				case K.SourceFile:
					if (!isExternalOrCommonJsModule(location)) break;
				// falls through
				case K.ModuleDeclaration: {
					const symbol = location.symbol;
					if ((result = callback(symbol?.exports ?? new Map(), undefined, true, location))) return result;
					break;
				}
				case K.ClassDeclaration:
				case K.ClassExpression:
				case K.InterfaceDeclaration: {
					let table;
					for (const [key, member] of location.symbol?.members ?? []) {
						if (member.flags & (SF.Type & ~SF.Assignment)) (table ??= new Map()).set(key, member);
					}
					if (table && (result = callback(table, undefined, false, location))) return result;
					break;
				}
			}
		}
		return callback(this._globals, undefined, true);
	}

	/** tsc's `needsQualification`. */
	_needsQualification(symbol, enclosingDeclaration, meaning) {
		let qualify = false;
		this._forEachSymbolTableInScope(enclosingDeclaration, (table) => {
			let fromTable = this._mergedSymbol(table.get(symbol.escapedName));
			if (!fromTable) return false;
			if (fromTable === symbol) return true;
			const shouldResolveAlias = fromTable.flags & SF.Alias && !fromTable.declarations?.some((declaration) => declaration.kind === K.ExportSpecifier);
			fromTable = shouldResolveAlias ? this._resolveAlias(fromTable) : fromTable;
			if (fromTable.flags & meaning) {
				qualify = true;
				return true;
			}
			return false;
		});
		return qualify;
	}

	/** tsc's `getAccessibleSymbolChain`. */
	_getAccessibleSymbolChain(symbol, enclosingDeclaration, meaning, useOnlyExternalAliasing, visitedSymbolTablesMap = new Map()) {
		if (!symbol || isPropertyOrMethodDeclarationSymbol(symbol)) return undefined;
		let cache = this._accessibleChains.get(symbol);
		if (!cache) this._accessibleChains.set(symbol, (cache = new Map()));
		const key = `${useOnlyExternalAliasing ? 0 : 1}|${enclosingDeclaration ? this._nodeId(enclosingDeclaration) : ""}|${meaning}`;
		if (cache.has(key)) return cache.get(key);
		let visitedSymbolTables = visitedSymbolTablesMap.get(symbol);
		if (!visitedSymbolTables) visitedSymbolTablesMap.set(symbol, (visitedSymbolTables = []));

		const canQualifySymbol = (fromTable, meaningToQualify) =>
			!this._needsQualification(fromTable, enclosingDeclaration, meaningToQualify) ||
			!!this._getAccessibleSymbolChain(fromTable.parent, enclosingDeclaration, getQualifiedLeftMeaning(meaningToQualify), useOnlyExternalAliasing, visitedSymbolTablesMap);
		const isAccessible = (fromTable, resolvedAlias, ignoreQualification) => {
			const target = resolvedAlias || fromTable;
			return (
				(symbol === target || this._mergedSymbol(symbol) === this._mergedSymbol(target)) &&
				!(fromTable.declarations ?? []).some(hasNonGlobalAugmentationExternalModuleSymbol) &&
				(ignoreQualification || canQualifySymbol(this._mergedSymbol(fromTable), meaning))
			);
		};
		const getCandidateListForSymbol = (fromTable, resolvedImportedSymbol, ignoreQualification) => {
			if (isAccessible(fromTable, resolvedImportedSymbol, ignoreQualification)) return [fromTable];
			const candidateTable = this._exportsOf(resolvedImportedSymbol);
			const fromExports = candidateTable && fromSymbolTable(candidateTable, true);
			if (fromExports && canQualifySymbol(fromTable, getQualifiedLeftMeaning(meaning))) return [fromTable, ...fromExports];
			return undefined;
		};
		const trySymbolTable = (symbols, ignoreQualification, isLocalNameLookup) => {
			const direct = symbols.get(symbol.escapedName);
			if (direct && isAccessible(direct, undefined, ignoreQualification)) return [symbol];
			if (symbols.isGlobals) return undefined;
			for (const fromTable of symbols.values()) {
				if (
					fromTable.flags & SF.Alias &&
					fromTable.escapedName !== "export=" &&
					fromTable.escapedName !== "default" &&
					(!useOnlyExternalAliasing || fromTable.declarations?.some(isExternalModuleImportEqualsDeclaration)) &&
					(isLocalNameLookup ? !fromTable.declarations?.some(isNamespaceReexportDeclaration) : true) &&
					(ignoreQualification || !fromTable.declarations?.some((declaration) => declaration.kind === K.ExportSpecifier))
				) {
					const resolvedImportedSymbol = this._resolveAlias(fromTable);
					const candidate = getCandidateListForSymbol(fromTable, resolvedImportedSymbol, ignoreQualification);
					if (candidate) return candidate;
				}
				if (fromTable.escapedName === symbol.escapedName && fromTable.exportSymbol) {
					if (isAccessible(this._mergedSymbol(fromTable.exportSymbol), undefined, ignoreQualification)) return [symbol];
				}
			}
			return undefined;
		};
		const fromSymbolTable = (symbols, ignoreQualification, isLocalNameLookup) => {
			if (visitedSymbolTables.includes(symbols)) return undefined;
			visitedSymbolTables.push(symbols);
			const result = trySymbolTable(symbols, ignoreQualification, isLocalNameLookup);
			visitedSymbolTables.pop();
			return result;
		};
		const result = this._forEachSymbolTableInScope(enclosingDeclaration, fromSymbolTable);
		cache.set(key, result);
		return result;
	}

	/** tsc's `getAliasForSymbolInContainer`. */
	_aliasForSymbolInContainer(container, symbol) {
		if (container === this._parentOf(symbol)) return symbol;
		const exportEquals = container.exports?.get("export=");
		if (exportEquals && this._isSameReference(exportEquals, symbol)) return container;
		const exports = this._exportsOf(container);
		const quick = exports.get(symbol.escapedName);
		if (quick && this._isSameReference(quick, symbol)) return quick;
		for (const exported of exports.values()) if (this._isSameReference(exported, symbol)) return exported;
		return undefined;
	}

	_parentOf(symbol) {
		return this._mergedSymbol(symbol.parent);
	}

	/** tsc's `getExternalModuleContainer`. */
	_externalModuleContainer(declaration) {
		const node = findAncestor(declaration, hasExternalModuleSymbol);
		return node && node.symbol;
	}

	/** tsc's `getAlternativeContainingModules`. */
	_alternativeContainingModules(symbol, enclosingDeclaration) {
		const containingFile = enclosingDeclaration.getSourceFile();
		let byFile = this._extendedContainers.get(symbol);
		if (!byFile) this._extendedContainers.set(symbol, (byFile = new Map()));
		if (byFile.has(containingFile)) return byFile.get(containingFile);
		let results;
		for (const specifier of moduleSpecifiersOf(containingFile)) {
			const resolvedModule = this._checker.resolveExternalModuleName(specifier);
			if (!resolvedModule) continue;
			if (!this._aliasForSymbolInContainer(resolvedModule, symbol)) continue;
			(results ??= []).push(resolvedModule);
		}
		if (results) {
			byFile.set(containingFile, results);
			return results;
		}
		if (byFile.has(undefined)) return byFile.get(undefined);
		results = [];
		for (const file of this._program.getSourceFiles()) {
			if (!isExternalOrCommonJsModule(file)) continue;
			const moduleSymbol = file.symbol;
			if (moduleSymbol && this._aliasForSymbolInContainer(moduleSymbol, symbol)) results.push(moduleSymbol);
		}
		byFile.set(undefined, results);
		return results;
	}

	/** tsc's `getVariableDeclarationOfObjectLiteral`. */
	_variableDeclarationOfObjectLiteral(symbol, meaning) {
		const first = symbol.declarations?.[0];
		if (meaning & SF.Value && first && first.parent && first.parent.kind === K.VariableDeclaration) {
			if ((first.kind === K.ObjectLiteralExpression && first === first.parent.initializer) || (first.kind === K.TypeLiteral && first === first.parent.type)) {
				return first.parent.symbol;
			}
		}
		return undefined;
	}

	/** tsc's `getContainersOfSymbol`. */
	_containersOfSymbol(symbol, enclosingDeclaration, meaning) {
		const container = this._parentOf(symbol);
		const fileSymbolIfExportEqualsContainer = (declaration) => {
			if (!container) return undefined;
			const fileSymbol = this._externalModuleContainer(declaration);
			const exported = fileSymbol?.exports?.get("export=");
			return exported && this._isSameReference(exported, container) ? fileSymbol : undefined;
		};
		const withAlternativeContainers = (each) => {
			const additional = (each.declarations ?? []).map(fileSymbolIfExportEqualsContainer).filter(Boolean);
			const reexportContainers = enclosingDeclaration ? this._alternativeContainingModules(symbol, enclosingDeclaration) : undefined;
			const objectLiteralContainer = this._variableDeclarationOfObjectLiteral(each, meaning);
			if (
				enclosingDeclaration &&
				each.flags & getQualifiedLeftMeaning(meaning) &&
				this._getAccessibleSymbolChain(each, enclosingDeclaration, SF.Namespace, false)
			) {
				const result = [each, ...additional, ...(reexportContainers ?? [])];
				if (objectLiteralContainer) result.push(objectLiteralContainer);
				return result;
			}
			if (!(each.flags & getQualifiedLeftMeaning(meaning)) && each.flags & SF.Type && meaning === SF.Value) {
				throw unsupported("a value reached through a type's container");
			}
			const result = [...additional, each];
			if (objectLiteralContainer) result.push(objectLiteralContainer);
			if (reexportContainers) result.push(...reexportContainers);
			return result;
		};
		if (container && !(symbol.flags & SF.TypeParameter)) return withAlternativeContainers(container);
		const candidates = [];
		for (const declaration of symbol.declarations ?? []) {
			if (!isAmbientModule(declaration) && declaration.parent) {
				if (hasNonGlobalAugmentationExternalModuleSymbol(declaration.parent)) {
					candidates.push(declaration.parent.symbol);
					continue;
				}
				if (declaration.parent.kind === K.ModuleBlock && declaration.parent.parent && this._resolveExternalModuleSymbol(declaration.parent.parent.symbol) === symbol) {
					candidates.push(declaration.parent.parent.symbol);
					continue;
				}
			}
		}
		if (!candidates.length) return undefined;
		const containers = candidates.filter((candidate) => candidate && this._aliasForSymbolInContainer(candidate, symbol));
		const best = [];
		const alternatives = [];
		for (const each of containers) {
			const [bestMatch, ...rest] = withAlternativeContainers(each);
			best.push(bestMatch);
			alternatives.push(...rest);
		}
		return [...best, ...alternatives];
	}

	_resolveExternalModuleSymbol(moduleSymbol) {
		const exportEquals = moduleSymbol?.exports?.get("export=");
		return exportEquals ? this._resolveAlias(exportEquals) : moduleSymbol;
	}

	/** tsc's `isAnySymbolAccessible`. */
	_isAnySymbolAccessible(symbols, enclosingDeclaration, initialSymbol, meaning, shouldComputeAliasesToMakeVisible, allowModules) {
		if (!symbols || !symbols.length) return undefined;
		let hadAccessibleChain;
		let earlyModuleBail = false;
		for (const symbol of symbols) {
			const chain = this._getAccessibleSymbolChain(symbol, enclosingDeclaration, meaning, false);
			if (chain) {
				hadAccessibleChain = symbol;
				const visible = this._hasVisibleDeclarations(chain[0], shouldComputeAliasesToMakeVisible);
				if (visible) return visible;
			}
			if (allowModules && (symbol.declarations ?? []).some(hasNonGlobalAugmentationExternalModuleSymbol)) {
				if (shouldComputeAliasesToMakeVisible) {
					earlyModuleBail = true;
					continue;
				}
				return { accessibility: Accessibility.Accessible };
			}
			const containers = this._containersOfSymbol(symbol, enclosingDeclaration, meaning);
			const parentResult = this._isAnySymbolAccessible(
				containers,
				enclosingDeclaration,
				initialSymbol,
				initialSymbol === symbol ? getQualifiedLeftMeaning(meaning) : meaning,
				shouldComputeAliasesToMakeVisible,
				allowModules,
			);
			if (parentResult) return parentResult;
		}
		if (earlyModuleBail) return { accessibility: Accessibility.Accessible };
		if (hadAccessibleChain) return { accessibility: Accessibility.NotAccessible, errorSymbolName: initialSymbol.name };
		return undefined;
	}

	/** tsc's `isSymbolAccessible`. */
	isSymbolAccessible(symbol, enclosingDeclaration, meaning, shouldComputeAliasesToMakeVisible, allowModules = true) {
		if (symbol && enclosingDeclaration) {
			const result = this._isAnySymbolAccessible([symbol], enclosingDeclaration, symbol, meaning, shouldComputeAliasesToMakeVisible, allowModules);
			if (result) return result;
			const symbolExternalModule = (symbol.declarations ?? []).map((declaration) => this._externalModuleContainer(declaration)).find(Boolean);
			if (symbolExternalModule && symbolExternalModule !== this._externalModuleContainer(enclosingDeclaration)) {
				return { accessibility: Accessibility.CannotBeNamed, errorSymbolName: symbol.name, errorModuleName: symbolExternalModule.name };
			}
			return { accessibility: Accessibility.NotAccessible, errorSymbolName: symbol.name };
		}
		return { accessibility: Accessibility.Accessible };
	}

	// --- declarations ----------------------------------------------------------

	/** tsc's `isOptionalParameter`. */
	isOptionalParameter(node) {
		if (node.questionToken) return true;
		if (node.kind !== K.Parameter) return false;
		if (node.initializer) {
			const parameters = node.parent.parameters;
			let minArgumentCount = 0;
			let index = 0;
			for (const parameter of parameters) {
				if (parameter === parameters[0] && parameter.name.kind === K.Identifier && parameter.name.escapedText === "this") continue;
				index++;
				if (!(parameter.initializer || parameter.questionToken || parameter.dotDotDotToken)) minArgumentCount = index;
			}
			return parameters.indexOf(node) >= minArgumentCount;
		}
		return false;
	}

	/** tsc's `requiresAddingImplicitUndefined` for a parameter. */
	requiresAddingImplicitUndefined(parameter, enclosingDeclaration) {
		if (parameter.kind !== K.Parameter) return false;
		const required =
			this._strictNullChecks &&
			!this.isOptionalParameter(parameter) &&
			!!parameter.initializer &&
			(!hasSyntacticModifier(parameter, ModifierFlags.ParameterPropertyModifier) || (!!enclosingDeclaration && isFunctionLikeDeclarationKind(enclosingDeclaration.kind)));
		const optionalProperty =
			this._strictNullChecks &&
			this.isOptionalParameter(parameter) &&
			!parameter.initializer &&
			hasSyntacticModifier(parameter, ModifierFlags.ParameterPropertyModifier);
		if (!required && !optionalProperty) return false;
		if (!parameter.type) return true;
		const type = this._checker.getTypeFromTypeNode(parameter.type);
		const containsUndefined = (each) => each.flags & (TF.Undefined | TF.Any) || (each.flags & TF.Union && each.types.some((member) => member.flags & TF.Undefined));
		return !containsUndefined(type);
	}

	/** tsc's `isImplementationOfOverload`. */
	isImplementationOfOverload(node) {
		if (!node.body || node.body.kind === undefined) return false;
		if (node.kind === K.GetAccessor || node.kind === K.SetAccessor) return false;
		const symbol = node.symbol;
		if (!symbol) return false;
		const declarations = (symbol.declarations ?? []).filter((declaration) => isFunctionLike(declaration));
		const signatures = declarations.filter((declaration, index) => {
			if (index > 0 && declaration.body) {
				const previous = declarations[index - 1];
				if (declaration.parent === previous.parent && declaration.kind === previous.kind && declaration.pos === previous.end) return false;
			}
			return true;
		});
		return signatures.length > 1 || (signatures.length === 1 && signatures[0] !== node);
	}

	isLateBound(node) {
		const name = node.name;
		if (!name || name.kind !== K.ComputedPropertyName) return false;
		const type = this._checker.getTypeAtLocation(name.expression);
		return !!(type.flags & (TF.UniqueESSymbol | TF.StringLiteral | TF.NumberLiteral));
	}

	isFirstDeclarationOfSymbol(node) {
		const declarations = node.symbol?.declarations;
		return !declarations || declarations[0] === node;
	}

	isImportRequiredByAugmentation(node) {
		const file = node.getSourceFile();
		if (!file.symbol) return false;
		const target = this._checker.resolveExternalModuleName(node.moduleSpecifier);
		const targetFile = target?.declarations?.find((declaration) => declaration.kind === K.SourceFile);
		if (!targetFile || targetFile === file) return false;
		for (const exported of this._checker.getExportsOfModule(file.symbol)) {
			for (const declaration of exported.declarations ?? []) {
				if (declaration.getSourceFile() === targetFile) return true;
			}
		}
		return false;
	}

	isExpandoFunctionDeclaration(node) {
		const declaration = f.getParseTreeNode(node);
		if (!declaration || (declaration.kind !== K.FunctionDeclaration && declaration.kind !== K.VariableDeclaration)) return false;
		let symbol;
		if (declaration.kind === K.VariableDeclaration) {
			if (declaration.type || !isVarConstLike(declaration)) return false;
			const initializer = declaration.initializer;
			if (!initializer || (initializer.kind !== K.ArrowFunction && initializer.kind !== K.FunctionExpression)) return false;
			symbol = declaration.symbol;
		} else {
			symbol = declaration.symbol;
		}
		if (!symbol) return false;
		for (const exported of symbol.exports?.values() ?? []) {
			if (exported.flags & SF.Value && exported.valueDeclaration && exported.valueDeclaration.kind === K.BinaryExpression) return true;
			if (exported.flags & SF.Value && exported.valueDeclaration && exported.valueDeclaration.kind === K.PropertyAccessExpression) return true;
		}
		return false;
	}

	getEnumMemberValue(member) {
		const value = this._checker._enumMemberValue(member);
		return value === undefined ? undefined : { value };
	}

	isInternalDeclaration() {
		throw unsupported("stripInternal");
	}

	isRangeOnOneLine(node) {
		const file = f.getParseTreeNode(node)?.getSourceFile?.();
		if (!file) return false;
		const start = file.getLineAndCharacterOfPosition(node.getStart ? node.getStart(file) : node.pos).line;
		return start === file.getLineAndCharacterOfPosition(node.end).line;
	}

	// --- literal constants -----------------------------------------------------

	_freshLiteralType(node) {
		const [file, index] = this._checker._node(node);
		const id = this._native.declarationFreshLiteral(file, index);
		return id === null || id === undefined ? undefined : this._checker._type(id);
	}

	/** tsc's `isLiteralConstDeclaration`. */
	isLiteralConstDeclaration(node) {
		if (isDeclarationReadonly(node) || (node.kind === K.VariableDeclaration && isVarConstLike(node))) return this._freshLiteralType(node) !== undefined;
		return false;
	}

	/** tsc's `createLiteralConstValue`. */
	createLiteralConstValue(node, tracker) {
		const type = this._freshLiteralType(node);
		if (type.flags & TF.EnumLike) throw unsupported("an enum member constant");
		if (type.flags & TF.BooleanLiteral) return this._checker.typeToString(type) === "true" ? f.createTrue() : f.createFalse();
		if (type.flags & TF.BigIntLiteral) return f.createBigIntLiteral(this._checker.typeToString(type));
		const value = type.value;
		if (typeof value === "string") return f.createStringLiteral(value);
		return value < 0 ? f.createPrefixUnaryExpression(K.MinusToken, f.createNumericLiteral(-value)) : f.createNumericLiteral(value);
	}

	// --- inferred types --------------------------------------------------------

	_context(enclosingDeclaration, tracker) {
		return { enclosingDeclaration, enclosingFile: enclosingDeclaration.getSourceFile?.() ?? f.getOriginalNode(enclosingDeclaration).getSourceFile(), tracker, multiline: true };
	}

	/** tsc's `createTypeOfDeclaration`. */
	createTypeOfDeclaration(declarationIn, enclosingDeclaration, tracker) {
		const declaration = f.getParseTreeNode(declarationIn);
		if (!declaration || !hasInferredType(declaration)) return f.createKeywordTypeNode(K.AnyKeyword);
		return this._serializeTypeOfDeclaration(declaration, declaration.symbol, this._context(enclosingDeclaration, tracker));
	}

	/** tsc's `createReturnTypeOfSignatureDeclaration`. */
	createReturnTypeOfSignatureDeclaration(signatureDeclarationIn, enclosingDeclaration, tracker) {
		const declaration = f.getParseTreeNode(signatureDeclarationIn);
		if (!declaration || !isFunctionLike(declaration)) return f.createKeywordTypeNode(K.AnyKeyword);
		const context = this._context(enclosingDeclaration, tracker);
		if (declaration.kind === K.GetAccessor) throw unsupported("an accessor's inferred type");
		return this._createReturnFromSignature(declaration, context);
	}

	/** expressionToTypeNode.ts `serializeTypeOfDeclaration`. */
	_serializeTypeOfDeclaration(node, symbol, context) {
		switch (node.kind) {
			case K.Parameter:
				return this._typeFromParameter(node, context);
			case K.VariableDeclaration:
				return this._typeFromVariable(node, symbol, context);
			case K.PropertySignature:
			case K.PropertyDeclaration:
				return this._typeFromProperty(node, context);
			case K.BindingElement:
				return this._inferTypeOfDeclaration(node, context);
		}
		throw unsupported(`the inferred type of a ${K[node.kind]}`);
	}

	_typeFromVariable(node, symbol, context) {
		let result;
		if (node.type) throw unsupported("reusing a written type annotation");
		if (node.initializer && ((symbol?.declarations?.length ?? 1) === 1 || symbol.declarations.filter((d) => d.kind === K.VariableDeclaration).length === 1)) {
			if (!this.isExpandoFunctionDeclaration(node) && !isContextuallyTyped(node)) {
				result = this._typeFromExpression(node.initializer, context, false, false, isVarConstLike(node));
			}
		}
		return result?.type ?? this._inferTypeOfDeclaration(node, context);
	}

	_typeFromParameter(node, context) {
		if (node.parent.kind === K.SetAccessor) throw unsupported("an accessor's inferred type");
		const addUndefined = this.requiresAddingImplicitUndefined(node, context.enclosingDeclaration);
		let result;
		if (node.type) throw unsupported("reusing a written type annotation");
		if (node.initializer && node.name.kind === K.Identifier && !isContextuallyTyped(node)) {
			result = this._typeFromExpression(node.initializer, context, false, addUndefined);
		}
		return result?.type ?? this._inferTypeOfDeclaration(node, context);
	}

	_typeFromProperty(node, context) {
		if (node.type) throw unsupported("reusing a written type annotation");
		let result;
		const initializer = node.kind === K.PropertyDeclaration ? node.initializer : undefined;
		if (initializer && !isContextuallyTyped(node)) {
			result = this._typeFromExpression(initializer, context, false, false, isDeclarationReadonly(node));
		}
		return result?.type ?? this._inferTypeOfDeclaration(node, context);
	}

	/** expressionToTypeNode.ts `typeFromExpression`: `undefined` where it falls back to the checker. */
	_typeFromExpression(node, context, isConstContext = false, requiresAddingUndefined = false, preserveLiterals = false) {
		switch (node.kind) {
			case K.ParenthesizedExpression:
				return this._typeFromExpression(node.expression, context, isConstContext, requiresAddingUndefined);
			case K.Identifier:
				if (this._checker.isUndefinedSymbol(this._checker.getSymbolAtLocation(node))) return { type: this._undefinedTypeNode() };
				return undefined;
			case K.NullKeyword:
				if (this._strictNullChecks) {
					return { type: this._addUndefinedIfNeeded(f.createLiteralTypeNode(f.createNull()), requiresAddingUndefined, node, context) };
				}
				return { type: f.createKeywordTypeNode(K.AnyKeyword) };
			case K.ArrowFunction:
			case K.FunctionExpression:
				return { type: this._typeFromFunctionLikeExpression(node, context) };
			case K.TypeAssertionExpression:
			case K.AsExpression:
				if (node.type.kind === K.TypeReference && node.type.typeName.kind === K.Identifier && node.type.typeName.escapedText === "const") {
					return this._typeFromExpression(node.expression, context, true, requiresAddingUndefined);
				}
				throw unsupported("reusing an asserted type");
			case K.PrefixUnaryExpression:
				if (isPrimitiveLiteralValue(node)) {
					return this._typeFromPrimitiveLiteral(
						node.operator === K.PlusToken ? node.operand : node,
						node.operand.kind === K.BigIntLiteral ? K.BigIntKeyword : K.NumberKeyword,
						context,
						isConstContext || preserveLiterals,
						requiresAddingUndefined,
					);
				}
				return undefined;
			case K.ArrayLiteralExpression:
			case K.ObjectLiteralExpression:
				// tsc builds these but answers with the checker's type
				// (`notImplemented`), except where the literal is not a
				// declaration's: there the expression's own type.
				if (!isConstContext && !requiresAddingUndefined && !isDeclaration(walkUpParenthesizedExpressions(node).parent)) {
					return { type: this._inferExpressionType(node, context) };
				}
				return undefined;
			case K.ClassExpression:
				return { type: this._inferExpressionType(node, context) };
			case K.TemplateExpression:
				if (!isConstContext && !preserveLiterals) return { type: f.createKeywordTypeNode(K.StringKeyword) };
				return undefined;
			case K.NumericLiteral:
				return this._typeFromPrimitiveLiteral(node, K.NumberKeyword, context, isConstContext || preserveLiterals, requiresAddingUndefined);
			case K.NoSubstitutionTemplateLiteral:
				return this._typeFromPrimitiveLiteral(f.createStringLiteral(node.text), K.StringKeyword, context, isConstContext || preserveLiterals, requiresAddingUndefined);
			case K.StringLiteral:
				return this._typeFromPrimitiveLiteral(node, K.StringKeyword, context, isConstContext || preserveLiterals, requiresAddingUndefined);
			case K.BigIntLiteral:
				return this._typeFromPrimitiveLiteral(node, K.BigIntKeyword, context, isConstContext || preserveLiterals, requiresAddingUndefined);
			case K.TrueKeyword:
			case K.FalseKeyword:
				return this._typeFromPrimitiveLiteral(node, K.BooleanKeyword, context, isConstContext || preserveLiterals, requiresAddingUndefined);
		}
		return undefined;
	}

	_typeFromPrimitiveLiteral(node, baseType, context, preserveLiterals, requiresAddingUndefined) {
		const result = preserveLiterals ? f.createLiteralTypeNode(this._reuseNode(context, node)) : f.createKeywordTypeNode(baseType);
		return { type: this._addUndefinedIfNeeded(result, requiresAddingUndefined, node, context) };
	}

	/** tsc's `reuseNode` and `setTextRange` (checker): a clone at the original's position when written in the file being emitted. */
	_reuseNode(context, node) {
		if (node === undefined) return undefined;
		const clone = f.cloneNode(node);
		if (node.flags & NodeFlags.Synthesized) return clone;
		f.setOriginalNode(clone, node);
		if (context.enclosingFile === node.getSourceFile?.()) f.setTextRange(clone, node);
		return clone;
	}

	_undefinedTypeNode() {
		return f.createKeywordTypeNode(this._strictNullChecks ? K.UndefinedKeyword : K.AnyKeyword);
	}

	_addUndefinedIfNeeded(node, addUndefined, owner) {
		const parentDeclaration = owner && walkUpParenthesizedExpressions(owner).parent;
		const optionalDeclaration = parentDeclaration && isDeclaration(parentDeclaration) && isOptionalDeclaration(parentDeclaration);
		if (!this._strictNullChecks || !(addUndefined || optionalDeclaration)) return node;
		if (node.kind === K.UnionType) return f.createUnionTypeNode([...node.types, f.createKeywordTypeNode(K.UndefinedKeyword)]);
		return f.createUnionTypeNode([node, f.createKeywordTypeNode(K.UndefinedKeyword)]);
	}

	_typeFromFunctionLikeExpression(node, context) {
		const returnType = this._createReturnFromSignature(node, context);
		if (node.typeParameters?.length) throw unsupported("reusing written type parameters");
		const parameters = node.parameters.map((parameter) => this._ensureParameter(parameter, context));
		return f.createFunctionTypeNode(undefined, parameters, returnType);
	}

	_ensureParameter(parameter, context) {
		if (parameter.name.kind !== K.Identifier) throw unsupported("a destructured parameter in an inferred function type");
		return f.updateNode(parameter, {
			modifiers: undefined,
			dotDotDotToken: this._reuseNode(context, parameter.dotDotDotToken),
			name: this._cloneName(parameter.name),
			questionToken: this.isOptionalParameter(parameter) ? f.createToken(K.QuestionToken) : undefined,
			type: this._typeFromParameter(parameter, context),
			initializer: undefined,
		});
	}

	_cloneName(name) {
		const clone = f.createIdentifier(unescape(name.escapedText));
		f.setOriginalNode(clone, name);
		return f.setEmitFlags(clone, f.EmitFlags.NoAsciiEscaping);
	}

	/** expressionToTypeNode.ts `createReturnFromSignature`. */
	_createReturnFromSignature(fn, context) {
		let result;
		if (fn.type) throw unsupported("reusing a written return type");
		if (isValueSignatureDeclaration(fn)) result = this._typeFromSingleReturnExpression(fn, context);
		return result?.type ?? this._inferReturnTypeOfSignature(fn, context);
	}

	_typeFromSingleReturnExpression(declaration, context) {
		let candidate;
		if (declaration.body) {
			if (declaration.asteriskToken || hasSyntacticModifier(declaration, ModifierFlags.Async)) return undefined;
			const body = declaration.body;
			if (body.kind === K.Block) {
				forEachReturnStatement(body, (statement) => {
					if (statement.parent !== body) {
						candidate = undefined;
						return true;
					}
					if (!candidate) candidate = statement.expression;
					else {
						candidate = undefined;
						return true;
					}
					return undefined;
				});
			} else {
				candidate = body;
			}
		}
		if (!candidate) return undefined;
		if (isContextuallyTyped(candidate)) {
			if (candidate.kind === K.AsExpression || candidate.kind === K.TypeAssertionExpression) throw unsupported("reusing an asserted type");
			return undefined;
		}
		return this._typeFromExpression(candidate, context);
	}

	/** The checker's `serializeTypeOfDeclaration`: the declaration's type, its fresh literals widened. */
	_inferTypeOfDeclaration(node, context) {
		const symbol = node.symbol;
		if (!symbol || symbol.flags & (SF.TypeLiteral | SF.Signature)) return f.createKeywordTypeNode(K.AnyKeyword);
		const [file, index] = this._checker._node(node);
		const id = this._native.declarationWidenedLiteralType(file, index);
		let type = id === null || id === undefined ? this._checker.getTypeOfSymbol(symbol) : this._checker._type(id);
		if (node.kind === K.Parameter && this.requiresAddingImplicitUndefined(node, context.enclosingDeclaration)) {
			type = this._checker.getUnionType([type, this._checker.getUndefinedType()]);
		}
		return this._typeToTypeNode(type, context);
	}

	/** The checker's `serializeTypeOfExpression`. */
	_inferExpressionType(node, context) {
		return this._typeToTypeNode(this._checker.getTypeAtLocation(node), context);
	}

	/** The checker's `serializeReturnTypeForSignature`. */
	_inferReturnTypeOfSignature(declaration, context) {
		const signature = this._signatureOfDeclaration(declaration);
		if (!signature) return f.createKeywordTypeNode(K.AnyKeyword);
		return this._typeToTypeNode(signature.getReturnType(), context);
	}

	_signatureOfDeclaration(declaration) {
		let type;
		if (declaration.kind === K.ArrowFunction || declaration.kind === K.FunctionExpression) type = this._checker.getTypeAtLocation(declaration);
		else if (declaration.symbol) type = this._checker.getTypeOfSymbol(declaration.symbol);
		if (!type) return undefined;
		const signatures = this._checker.getSignaturesOfType(type, declaration.kind === K.Constructor ? SignatureKind.Construct : SignatureKind.Call);
		return signatures.find((signature) => signature.declaration === declaration) ?? (signatures.length === 1 ? signatures[0] : undefined);
	}

	// --- typeToTypeNode --------------------------------------------------------

	_typeToTypeNode(type, context) {
		const flags = type.flags;
		if (flags & TF.Any) return f.createKeywordTypeNode(K.AnyKeyword);
		if (flags & TF.Unknown) return f.createKeywordTypeNode(K.UnknownKeyword);
		if (flags & TF.String) return f.createKeywordTypeNode(K.StringKeyword);
		if (flags & TF.Number) return f.createKeywordTypeNode(K.NumberKeyword);
		if (flags & TF.BigInt) return f.createKeywordTypeNode(K.BigIntKeyword);
		if (flags & TF.Boolean && !type.aliasSymbol) return f.createKeywordTypeNode(K.BooleanKeyword);
		if (flags & TF.EnumLike) throw unsupported("an enum type");
		if (flags & TF.StringLiteral) {
			return f.createLiteralTypeNode(f.setEmitFlags(f.createStringLiteral(type.value), f.EmitFlags.NoAsciiEscaping));
		}
		if (flags & TF.NumberLiteral) {
			const value = type.value;
			return f.createLiteralTypeNode(value < 0 ? f.createPrefixUnaryExpression(K.MinusToken, f.createNumericLiteral(-value)) : f.createNumericLiteral(value));
		}
		if (flags & TF.BigIntLiteral) return f.createLiteralTypeNode(f.createBigIntLiteral(this._checker.typeToString(type)));
		if (flags & TF.BooleanLiteral) return f.createLiteralTypeNode(this._checker.typeToString(type) === "true" ? f.createTrue() : f.createFalse());
		if (flags & TF.UniqueESSymbol) throw unsupported("a unique symbol type");
		if (flags & TF.Void) return f.createKeywordTypeNode(K.VoidKeyword);
		if (flags & TF.Undefined) return f.createKeywordTypeNode(K.UndefinedKeyword);
		if (flags & TF.Null) return f.createLiteralTypeNode(f.createNull());
		if (flags & TF.Never) return f.createKeywordTypeNode(K.NeverKeyword);
		if (flags & TF.ESSymbol) return f.createKeywordTypeNode(K.SymbolKeyword);
		if (flags & TF.NonPrimitive) return f.createKeywordTypeNode(K.ObjectKeyword);
		if (type.aliasSymbol && this._isTypeSymbolAccessible(type.aliasSymbol, context.enclosingDeclaration)) {
			const typeArguments = type.aliasTypeArguments?.map((argument) => this._typeToTypeNode(argument, context));
			if (type.aliasSymbol.escapedName === "Array" || type.aliasSymbol.escapedName === "ReadonlyArray") throw unsupported("an aliased array type");
			return this._symbolToTypeNode(type.aliasSymbol, context, SF.Type, typeArguments);
		}
		if (flags & TF.TypeParameter) {
			if (this._isThisType(type)) return f.createThisTypeNode();
			const name = type.symbol?.name;
			if (!name) throw unsupported("an anonymous type parameter");
			return f.createTypeReferenceNode(f.setEmitFlags(f.createIdentifier(name), f.EmitFlags.NoAsciiEscaping), undefined);
		}
		if (flags & TF.Union) {
			const types = this._formatUnionTypes(type.types);
			if (types.length === 1) return this._typeToTypeNode(types[0], context);
			return f.createUnionTypeNode(types.map((member) => this._typeToTypeNode(member, context)));
		}
		if (flags & TF.Intersection) {
			return f.createIntersectionTypeNode(type.types.map((member) => this._typeToTypeNode(member, context)));
		}
		if (flags & TF.Object) {
			const objectFlags = type.objectFlags;
			if (objectFlags & OF.Reference) return this._typeReferenceToTypeNode(type, context);
			if (objectFlags & OF.ClassOrInterface) {
				// tsc's `WriteClassExpressionAsTypeLiteral`: no name reaches a class expression's instance.
				const classExpression = this._classExpressionOfInstance(type);
				if (classExpression) return this._createTypeNodeFromObjectType(type, context, classExpression);
				return this._symbolToTypeNode(type.symbol, context, SF.Type, undefined);
			}
			return this._createAnonymousTypeNode(type, context);
		}
		throw unsupported(`the type ${this._checker.typeToString(type)}`);
	}

	_isThisType(type) {
		const declaration = type.symbol?.declarations?.[0];
		return !!declaration && (declaration.kind === K.ClassDeclaration || declaration.kind === K.InterfaceDeclaration);
	}

	_isTypeSymbolAccessible(symbol, enclosingDeclaration) {
		return this.isSymbolAccessible(symbol, enclosingDeclaration, SF.Type, false, true).accessibility === Accessibility.Accessible;
	}

	/** tsc's `formatUnionTypes`: `true | false` as `boolean`, `null` and `undefined` last. */
	_formatUnionTypes(types) {
		const result = [];
		let flags = 0;
		for (let i = 0; i < types.length; i++) {
			const member = types[i];
			flags |= member.flags;
			if (member.flags & TF.Nullable) continue;
			if (member.flags & TF.BooleanLiteral && i + 1 < types.length && types[i + 1].flags & TF.BooleanLiteral) {
				result.push(this._checker.getBooleanType());
				i += 1;
				continue;
			}
			if (member.flags & TF.EnumLike) throw unsupported("an enum type");
			result.push(member);
		}
		if (flags & TF.Null) result.push(this._checker.getNullType());
		if (flags & TF.Undefined) result.push(this._checker.getUndefinedType());
		return result;
	}

	_typeReferenceToTypeNode(type, context) {
		const checker = this._checker;
		const typeArguments = checker.getTypeArguments(type);
		if (checker.isArrayType(type)) {
			const elementType = this._typeToTypeNode(typeArguments[0], context);
			const arrayType = f.createArrayTypeNode(elementType);
			return type.symbol?.escapedName === "ReadonlyArray" ? f.createTypeOperatorNode(K.ReadonlyKeyword, arrayType) : arrayType;
		}
		if (checker.isTupleType(type)) {
			const target = type.target;
			const elements = typeArguments.map((argument, index) => {
				const flags = target.elementFlags[index];
				const labeled = target.labeledElementDeclarations?.[index];
				let node = this._typeToTypeNode(argument, context);
				if (labeled) {
					return f.createNamedTupleMember(
						flags & ElementFlags.Variable ? f.createToken(K.DotDotDotToken) : undefined,
						f.createIdentifier(unescape(labeled.name.escapedText)),
						flags & ElementFlags.Optional ? f.createToken(K.QuestionToken) : undefined,
						flags & ElementFlags.Rest ? f.createArrayTypeNode(node) : node,
					);
				}
				if (flags & ElementFlags.Optional) node = f.createOptionalTypeNode(node);
				else if (flags & ElementFlags.Rest) node = f.createRestTypeNode(f.createArrayTypeNode(node));
				else if (flags & ElementFlags.Variadic) node = f.createRestTypeNode(node);
				return node;
			});
			const tuple = f.setEmitFlags(f.createTupleTypeNode(elements), f.EmitFlags.SingleLine);
			return target.readonly ? f.createTypeOperatorNode(K.ReadonlyKeyword, tuple) : tuple;
		}
		const declared = type.symbol && checker.getDeclaredTypeOfSymbol(type.symbol);
		const parameterCount = declared ? checker.getTypeArguments(declared).filter((argument) => !this._isThisType(argument)).length : typeArguments.length;
		const typeArgumentNodes = parameterCount ? typeArguments.slice(0, parameterCount).map((argument) => this._typeToTypeNode(argument, context)) : undefined;
		return this._symbolToTypeNode(type.symbol, context, SF.Type, typeArgumentNodes);
	}

	_createAnonymousTypeNode(type, context) {
		const symbol = type.symbol;
		if (symbol) {
			if (symbol.flags & SF.Class && symbol.valueDeclaration?.kind === K.ClassDeclaration) return this._symbolToTypeNode(symbol, context, SF.Value, undefined);
			if (symbol.flags & (SF.Enum | SF.ValueModule)) return this._symbolToTypeNode(symbol, context, SF.Value, undefined);
			if (symbol.flags & SF.Function && (symbol.parent || symbol.declarations?.some((d) => d.parent.kind === K.SourceFile || d.parent.kind === K.ModuleBlock))) {
				if (this.isSymbolAccessible(symbol, context.enclosingDeclaration, SF.Value, false).accessibility === Accessibility.Accessible) {
					return this._symbolToTypeNode(symbol, context, SF.Value, undefined);
				}
			}
			if (symbol.flags & SF.Method && symbol.declarations?.some((d) => hasSyntacticModifier(d, ModifierFlags.Static))) throw unsupported("a static method's type");
		}
		return this._createTypeNodeFromObjectType(type, context);
	}

	/** The class expression `type` is the instance of: surge's types of one have no symbol, so it is found by its constructor's type. */
	_classExpressionOfInstance(type) {
		const declaration = type.symbol?.valueDeclaration;
		if (declaration) return declaration.kind === K.ClassExpression ? declaration : undefined;
		if (type.symbol) return undefined;
		if (!this._classExpressions) {
			this._classExpressions = [];
			const collect = (node) => {
				if (node.kind === K.ClassExpression) this._classExpressions.push(node);
				node.forEachChild(collect);
			};
			for (const file of this._program.getSourceFiles()) {
				if (!file.isDeclarationFile && !this._program.isSourceFileFromExternalLibrary(file)) collect(file);
			}
		}
		for (const classExpression of this._classExpressions) {
			const constructor = this._checker.getTypeAtLocation(classExpression);
			for (const signature of this._checker.getSignaturesOfType(constructor, SignatureKind.Construct)) {
				if (signature.getReturnType() === type) return classExpression;
			}
		}
		return undefined;
	}

	_createTypeNodeFromObjectType(type, context, classExpression) {
		const checker = this._checker;
		const properties = checker.getPropertiesOfType(type);
		const callSignatures = checker.getSignaturesOfType(type, SignatureKind.Call);
		const constructSignatures = checker.getSignaturesOfType(type, SignatureKind.Construct);
		const hasIndex = type.getStringIndexType() || type.getNumberIndexType();
		if (!properties.length && !hasIndex) {
			if (!callSignatures.length && !constructSignatures.length) {
				const literal = f.createTypeLiteralNode(undefined);
				if (!context.multiline) f.setEmitFlags(literal, f.EmitFlags.SingleLine);
				return literal;
			}
			if (callSignatures.length === 1 && !constructSignatures.length) return this._signatureToSignatureDeclaration(callSignatures[0], K.FunctionType, context);
			if (constructSignatures.length === 1 && !callSignatures.length) return this._signatureToSignatureDeclaration(constructSignatures[0], K.ConstructorType, context);
		}
		if (hasIndex) throw unsupported("an index signature in an inferred type");
		const members = [];
		for (const signature of callSignatures) members.push(this._signatureToSignatureDeclaration(signature, K.CallSignature, context));
		for (const signature of constructSignatures) members.push(this._signatureToSignatureDeclaration(signature, K.ConstructSignature, context));
		for (const property of properties) {
			// a class's constructor holds its `prototype`, which surge's types do not flag as one
			if (constructSignatures.length && property.escapedName === "prototype") continue;
			const member = classExpression?.members.find((each) => each.name && (each.name.text ?? each.name.escapedText) === property.name);
			if (member && modifierFlags(member) & (ModifierFlags.Private | ModifierFlags.Protected)) context.tracker.reportPrivateInBaseOfClassExpression(property.name);
			this._addPropertyToElementList(property, context, members, member);
		}
		const literal = f.createTypeLiteralNode(members);
		if (!context.multiline) f.setEmitFlags(literal, f.EmitFlags.SingleLine);
		return literal;
	}

	_addPropertyToElementList(property, context, members, memberDeclaration) {
		const checker = this._checker;
		if (property.valueDeclaration && (property.valueDeclaration.jsDoc?.length || hasLeadingJsDoc(property.valueDeclaration))) {
			throw unsupported("a property's documentation comment in an inferred type");
		}
		const propertyType = checker.getTypeOfSymbol(property);
		const name = this._propertyNameNode(property);
		const optional = property.flags & SF.Optional ? f.createToken(K.QuestionToken) : undefined;
		if (property.flags & SF.Accessor) throw unsupported("an accessor in an inferred type");
		const readonly = isReadonlySymbol(property, memberDeclaration);
		if (property.flags & (SF.Function | SF.Method) && !checker.getPropertiesOfType(propertyType).length && !readonly) {
			const signatures = checker.getSignaturesOfType(propertyType, SignatureKind.Call);
			for (const signature of signatures) members.push(this._signatureToSignatureDeclaration(signature, K.MethodSignature, context, { name, questionToken: optional }));
			if (signatures.length || !optional) return;
		}
		const declaration = property.valueDeclaration ?? memberDeclaration;
		if (declaration && hasInferredType(declaration) && declaration.type) throw unsupported("reusing a written type annotation");
		// the checker's `serializeTypeForDeclaration`: the syntactic builder first
		const typeNode =
			declaration && hasInferredType(declaration) && declaration.pos >= 0
				? this._serializeTypeOfDeclaration(declaration, property, { ...context, multiline: true })
				: this._typeToTypeNode(propertyType, { ...context, multiline: true });
		members.push(f.createPropertySignature(readonly ? [f.createToken(K.ReadonlyKeyword)] : undefined, name, optional, typeNode));
	}

	_propertyNameNode(property) {
		const name = unescape(property.escapedName);
		if (/^[A-Za-z_$][A-Za-z0-9_$]*$/.test(name)) return f.setEmitFlags(f.createIdentifier(name), f.EmitFlags.NoAsciiEscaping);
		if (/^\d+$/.test(name) && String(Number(name)) === name) return f.createNumericLiteral(name);
		return f.createStringLiteral(name);
	}

	_signatureToSignatureDeclaration(signature, kind, context, options) {
		const checker = this._checker;
		if (signature.typeParameters?.length) throw unsupported("a generic signature in an inferred type");
		const declaration = signature.declaration;
		if (declaration && declaration.type && declaration.kind !== K.JSDocSignature) throw unsupported("reusing a written return type");
		const parameters = signature.parameters.map((parameter) => {
			const parameterDeclaration = parameter.valueDeclaration?.kind === K.Parameter ? parameter.valueDeclaration : undefined;
			if (parameterDeclaration?.type) throw unsupported("reusing a written parameter type");
			const isRest = !!parameterDeclaration?.dotDotDotToken;
			const isOptional = parameterDeclaration ? this.isOptionalParameter(parameterDeclaration) : false;
			const name = parameterDeclaration && parameterDeclaration.name.kind === K.Identifier ? this._cloneName(parameterDeclaration.name) : f.createIdentifier(parameter.name);
			return f.createParameterDeclaration(
				undefined,
				isRest ? f.createToken(K.DotDotDotToken) : undefined,
				name,
				isOptional ? f.createToken(K.QuestionToken) : undefined,
				this._typeToTypeNode(checker.getTypeOfSymbol(parameter), context),
				undefined,
			);
		});
		const returnType = kind === K.Constructor ? undefined : this._typeToTypeNode(signature.getReturnType(), context);
		switch (kind) {
			case K.FunctionType:
				return f.createFunctionTypeNode(undefined, parameters, returnType);
			case K.ConstructorType:
				return f.createConstructorTypeNode(undefined, undefined, parameters, returnType);
			case K.CallSignature:
				return f.createCallSignature(undefined, parameters, returnType);
			case K.ConstructSignature:
				return f.createConstructSignature(undefined, parameters, returnType);
			case K.MethodSignature:
				return f.createMethodSignature(undefined, options.name, options.questionToken, undefined, parameters, returnType);
		}
		throw unsupported(`a ${K[kind]} signature`);
	}

	// --- symbol references -----------------------------------------------------

	/** tsc's `lookupSymbolChain`. */
	_lookupSymbolChain(symbol, context, meaning) {
		context.tracker.trackSymbol(symbol, context.enclosingDeclaration, meaning);
		if (symbol.flags & SF.TypeParameter) return [symbol];
		const chain = this._getSymbolChain(symbol, meaning, true, context);
		if (!chain || !chain.length) throw unsupported(`naming ${symbol.name} from where it is written`);
		return chain;
	}

	/** tsc's `getSymbolChain` (in `lookupSymbolChainWorker`). */
	_getSymbolChain(symbol, meaning, endOfChain, context) {
		let accessibleSymbolChain = this._getAccessibleSymbolChain(symbol, context.enclosingDeclaration, meaning, false);
		if (
			!accessibleSymbolChain ||
			this._needsQualification(accessibleSymbolChain[0], context.enclosingDeclaration, accessibleSymbolChain.length === 1 ? meaning : getQualifiedLeftMeaning(meaning))
		) {
			const parents = this._containersOfSymbol(accessibleSymbolChain ? accessibleSymbolChain[0] : symbol, context.enclosingDeclaration, meaning);
			if (parents && parents.length) {
				const specifiers = parents.map((parent) =>
					(parent.declarations ?? []).some(hasNonGlobalAugmentationExternalModuleSymbol) ? this._specifierForModuleSymbol(parent, context) : undefined,
				);
				const indices = parents.map((_, index) => index);
				indices.sort((a, b) => {
					const specifierA = specifiers[a];
					const specifierB = specifiers[b];
					if (specifierA && specifierB) {
						const isBRelative = pathIsRelative(specifierB);
						if (pathIsRelative(specifierA) === isBRelative) return countPathComponents(specifierA) - countPathComponents(specifierB);
						return isBRelative ? -1 : 1;
					}
					return 0;
				});
				for (const parent of indices.map((index) => parents[index])) {
					const parentChain = this._getSymbolChain(parent, getQualifiedLeftMeaning(meaning), false, context);
					if (parentChain) {
						const exportEquals = parent.exports?.get("export=");
						if (exportEquals && this._isSameReference(exportEquals, symbol)) {
							accessibleSymbolChain = parentChain;
							break;
						}
						accessibleSymbolChain = parentChain.concat(accessibleSymbolChain || [this._aliasForSymbolInContainer(parent, symbol) || symbol]);
						break;
					}
				}
			}
		}
		if (accessibleSymbolChain) return accessibleSymbolChain;
		if (endOfChain || !(symbol.flags & (SF.TypeLiteral | SF.ObjectLiteral))) return [symbol];
		return undefined;
	}

	/** tsc's `symbolToTypeNode`. */
	_symbolToTypeNode(symbol, context, meaning, typeArguments) {
		const chain = this._lookupSymbolChain(symbol, context, meaning);
		const isTypeOf = meaning === SF.Value;
		if ((chain[0].declarations ?? []).some(hasNonGlobalAugmentationExternalModuleSymbol)) {
			const nonRootParts = chain.length > 1 ? this._accessFromSymbolChain(chain, chain.length - 1, 1, typeArguments, context) : undefined;
			const specifier = this._specifierForModuleSymbol(chain[0], context);
			if (specifier.includes("/node_modules/")) throw unsupported("an import type through node_modules");
			const literal = f.createLiteralTypeNode(f.createStringLiteral(specifier));
			return f.createImportTypeNode(literal, undefined, nonRootParts, chain.length > 1 ? typeArguments : typeArguments, isTypeOf);
		}
		const entityName = this._accessFromSymbolChain(chain, chain.length - 1, 0, typeArguments, context);
		if (isTypeOf) return f.createTypeQueryNode(entityName, undefined);
		return f.createTypeReferenceNode(entityName, typeArguments);
	}

	_accessFromSymbolChain(chain, index, stopper, typeArguments, context) {
		const symbol = chain[index];
		const parent = chain[index - 1];
		let name;
		if (index === 0) name = symbol.name;
		else if (parent) {
			for (const [exportedName, exported] of this._exportsOf(parent)) {
				if (this._isSameReference(exported, symbol) && exportedName !== "export=") {
					name = unescape(exportedName);
					break;
				}
			}
		}
		if (name === undefined) name = symbol.name;
		if (name === "default" || name === "export=") throw unsupported("naming a default export");
		const identifier = f.setEmitFlags(f.createIdentifier(name), f.EmitFlags.NoAsciiEscaping);
		if (index > stopper) return f.createQualifiedName(this._accessFromSymbolChain(chain, index - 1, stopper, undefined, context), identifier);
		return identifier;
	}

	/** tsc's `getSpecifierForModuleSymbol`. */
	_specifierForModuleSymbol(symbol, context) {
		const file = symbol.declarations?.find((declaration) => declaration.kind === K.SourceFile);
		if (!file) {
			const name = symbol.escapedName;
			if (/^".+"$/.test(name)) return name.slice(1, -1);
			throw unsupported("a module symbol with no file");
		}
		const contextFile = context.enclosingFile;
		let byFile = this._specifiers.get(symbol);
		if (!byFile) this._specifiers.set(symbol, (byFile = new Map()));
		let specifier = byFile.get(contextFile);
		if (specifier === undefined) {
			specifier = this._moduleSpecifier(file, contextFile);
			byFile.set(contextFile, specifier);
		}
		return specifier;
	}

	/** moduleSpecifiers.ts `getModuleSpecifiers` for a file of the program: an import the file already writes, else the relative path. */
	_moduleSpecifier(moduleFile, importingFile) {
		for (const literal of moduleSpecifiersOf(importingFile)) {
			const resolved = this._program.getResolvedModule(importingFile, literal.text)?.resolvedModule;
			if (resolved && resolved.resolvedFileName === moduleFile.fileName) return literal.text;
		}
		const options = this._options;
		if (options.baseUrl || options.paths || options.rootDirs) throw unsupported("an import type under baseUrl, paths or rootDirs");
		// tsc names a referenced project's output by the source it stands for (`originalFileName`).
		const moduleFileName = this._referencedSources().get(moduleFile.fileName) ?? moduleFile.fileName;
		if (paths.isInsideNodeModules(moduleFileName)) throw unsupported("an import type through node_modules");
		const canonical = paths.createGetCanonicalFileName(this._useCaseSensitiveFileNames());
		let relative = paths.getRelativePathFromDirectory(paths.getDirectoryPath(importingFile.fileName), moduleFileName, canonical);
		if (!pathIsRelative(relative)) relative = `./${relative}`;
		return this._processEnding(relative, importingFile);
	}

	/** The source of each declaration file a referenced project outputs (tsc's `getOutputDeclarationFileName`). */
	_referencedSources() {
		if (this._sourcesOfOutputs) return this._sourcesOfOutputs;
		const sources = new Map();
		const getCanonicalFileName = paths.createGetCanonicalFileName(this._useCaseSensitiveFileNames());
		const seen = new Set();
		const visit = (references) => {
			for (const reference of references ?? []) {
				if (!reference || seen.has(reference)) continue;
				seen.add(reference);
				const { options, fileNames } = reference.commandLine;
				if (!options.outFile && options.configFilePath) {
					const currentDirectory = paths.getDirectoryPath(paths.normalizeSlashes(options.configFilePath));
					const emitted = fileNames.filter((fileName) => !paths.isDeclarationFileName(fileName) && !fileName.endsWith(".json"));
					let common;
					const host = {
						getCurrentDirectory: () => currentDirectory,
						getCanonicalFileName,
						getCommonSourceDirectory: () =>
							(common ??= emitPaths.commonSourceDirectoryOf(options, emitted, currentDirectory, getCanonicalFileName)),
					};
					for (const fileName of emitted) sources.set(emitPaths.getDeclarationOutputPaths(fileName, options, host).declarationFilePath, fileName);
				}
				visit(reference.references);
			}
		};
		visit(this._program.getResolvedProjectReferences());
		return (this._sourcesOfOutputs = sources);
	}

	_useCaseSensitiveFileNames() {
		return require("../sys.cjs").sys.useCaseSensitiveFileNames;
	}

	/** moduleSpecifiers.ts `processEnding` with the endings tsc prefers for the importing file. */
	_processEnding(fileName, importingFile) {
		if ([".json", ".mjs", ".cjs"].some((extension) => fileName.endsWith(extension))) return fileName;
		const noExtension = paths.removeFileExtension(fileName);
		if (noExtension === fileName) return fileName;
		if ([".d.mts", ".mts", ".d.cts", ".cts"].some((extension) => fileName.endsWith(extension))) throw unsupported("an import type of an .mts or .cts file");
		const resolution = this._options.moduleResolution;
		if (resolution === ModuleResolutionKind.Node16 || resolution === ModuleResolutionKind.NodeNext || resolution === ModuleResolutionKind.Bundler) {
			throw unsupported(`an import type under moduleResolution ${resolution}`);
		}
		if (usesExtensionsOnImports(importingFile)) throw unsupported("an import type in a file importing with extensions");
		return noExtension.endsWith("/index") ? noExtension.slice(0, -"/index".length) : noExtension;
	}
}

function isReadonlySymbol(symbol, fallbackDeclaration) {
	const declaration = symbol.valueDeclaration ?? fallbackDeclaration;
	if (!declaration) return false;
	if (modifierFlags(declaration) & ModifierFlags.Readonly) return true;
	if (declaration.kind === K.VariableDeclaration) return !!(combinedNodeFlags(declaration) & NodeFlags.Const);
	return !!(symbol.flags & SF.Accessor && !(symbol.flags & SF.SetAccessor));
}

function hasLeadingJsDoc(declaration) {
	const file = declaration.getSourceFile?.();
	if (!file) return false;
	const text = file.text;
	const leading = text.slice(declaration.pos, declaration.getStart(file));
	return leading.includes("/**");
}

const isExternalModuleImportEqualsDeclaration = (node) => node.kind === K.ImportEqualsDeclaration && node.moduleReference.kind === K.ExternalModuleReference;
const isNamespaceReexportDeclaration = (node) => node.kind === K.NamespaceExport && !!node.parent.moduleSpecifier;
const pathIsRelative = (path) => /^\.\.?(?:$|[\\/])/.test(path);
const countPathComponents = (path) => {
	let count = 0;
	for (let i = path.startsWith("./") ? 2 : 0; i < path.length; i++) if (path.charCodeAt(i) === 47) count++;
	return count;
};

function usesExtensionsOnImports(file) {
	for (const literal of moduleSpecifiersOf(file)) {
		const text = literal.text;
		if (!pathIsRelative(text) || [".d.ts", ".d.mts", ".d.cts"].some((extension) => text.endsWith(extension))) continue;
		return /\.[^/.]+$/.test(paths.getBaseFileName(text));
	}
	return false;
}

module.exports = { DeclarationEmitResolver, Accessibility };
