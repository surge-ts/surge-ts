"use strict";

// TypeScript's syntax, scanner and option helpers, ported from typescript@6.0.3
// (src/compiler/utilities.ts, utilitiesPublic.ts, scanner.ts). Many are
// internal to TypeScript; surge-ts provides them because tools built on its
// compiler API call them. They read only the syntax tree, so they answer as
// TypeScript does over surge's nodes.

const { SyntaxKind, ModifierFlags, NodeFlags, SymbolFlags, ScriptKind } = require("./enums.cjs");
const { escapeLeadingUnderscores, forEachChild } = require("./nodes.cjs");
const paths = require("./paths.cjs");

const K = SyntaxKind;

// --- core -------------------------------------------------------------------

const identity = (value) => value;
const returnTrue = () => true;
const returnFalse = () => false;
const returnUndefined = () => undefined;
const noop = () => {};

function arrayFrom(iterator, map) {
	const result = [];
	for (const value of iterator) result.push(map ? map(value) : value);
	return result;
}

// --- modifiers --------------------------------------------------------------

function modifierToFlag(token) {
	switch (token) {
		case K.StaticKeyword:
			return ModifierFlags.Static;
		case K.PublicKeyword:
			return ModifierFlags.Public;
		case K.ProtectedKeyword:
			return ModifierFlags.Protected;
		case K.PrivateKeyword:
			return ModifierFlags.Private;
		case K.AbstractKeyword:
			return ModifierFlags.Abstract;
		case K.AccessorKeyword:
			return ModifierFlags.Accessor;
		case K.ExportKeyword:
			return ModifierFlags.Export;
		case K.DeclareKeyword:
			return ModifierFlags.Ambient;
		case K.ConstKeyword:
			return ModifierFlags.Const;
		case K.DefaultKeyword:
			return ModifierFlags.Default;
		case K.AsyncKeyword:
			return ModifierFlags.Async;
		case K.ReadonlyKeyword:
			return ModifierFlags.Readonly;
		case K.OverrideKeyword:
			return ModifierFlags.Override;
		case K.InKeyword:
			return ModifierFlags.In;
		case K.OutKeyword:
			return ModifierFlags.Out;
		case K.Decorator:
			return ModifierFlags.Decorator;
	}
	return ModifierFlags.None;
}

function modifiersToFlags(modifiers) {
	let flags = ModifierFlags.None;
	for (const modifier of modifiers ?? []) flags |= modifierToFlag(modifier.kind);
	return flags;
}

function canHaveModifiers(node) {
	switch (node.kind) {
		case K.TypeParameter:
		case K.Parameter:
		case K.PropertySignature:
		case K.PropertyDeclaration:
		case K.MethodSignature:
		case K.MethodDeclaration:
		case K.Constructor:
		case K.GetAccessor:
		case K.SetAccessor:
		case K.IndexSignature:
		case K.ConstructorType:
		case K.FunctionExpression:
		case K.ArrowFunction:
		case K.ClassExpression:
		case K.VariableStatement:
		case K.FunctionDeclaration:
		case K.ClassDeclaration:
		case K.InterfaceDeclaration:
		case K.TypeAliasDeclaration:
		case K.EnumDeclaration:
		case K.ModuleDeclaration:
		case K.ImportEqualsDeclaration:
		case K.ImportDeclaration:
		case K.ExportAssignment:
		case K.ExportDeclaration:
			return true;
	}
	return false;
}

function canHaveDecorators(node) {
	switch (node.kind) {
		case K.Parameter:
		case K.PropertyDeclaration:
		case K.MethodDeclaration:
		case K.GetAccessor:
		case K.SetAccessor:
		case K.ClassExpression:
		case K.ClassDeclaration:
			return true;
	}
	return false;
}

function getSyntacticModifierFlagsNoCache(node) {
	let flags = canHaveModifiers(node) ? modifiersToFlags(node.modifiers) : ModifierFlags.None;
	if (node.flags & NodeFlags.NestedNamespace || (node.kind === K.Identifier && node.flags & NodeFlags.IdentifierIsInJSDocNamespace)) {
		flags |= ModifierFlags.Export;
	}
	return flags;
}

const modifierFlagsCache = new WeakMap();

function getSyntacticModifierFlags(node) {
	if (node.kind >= K.FirstToken && node.kind <= K.LastToken) return ModifierFlags.None;
	let flags = modifierFlagsCache.get(node);
	if (flags === undefined) {
		flags = getSyntacticModifierFlagsNoCache(node);
		modifierFlagsCache.set(node, flags);
	}
	return flags;
}

// TypeScript adds JSDoc modifiers (`@private`, `@readonly`, …) for a node in a
// JavaScript file; surge-ts reads the modifiers written in the syntax only.
const getEffectiveModifierFlags = getSyntacticModifierFlags;

function hasSyntacticModifier(node, flags) {
	return (getSyntacticModifierFlags(node) & flags) !== 0;
}

function hasEffectiveModifier(node, flags) {
	return (getEffectiveModifierFlags(node) & flags) !== 0;
}

const hasStaticModifier = (node) => hasSyntacticModifier(node, ModifierFlags.Static);
const hasAbstractModifier = (node) => hasSyntacticModifier(node, ModifierFlags.Abstract);
const hasAccessorModifier = (node) => hasSyntacticModifier(node, ModifierFlags.Accessor);
const hasDecorators = (node) => hasSyntacticModifier(node, ModifierFlags.Decorator);

function getModifiers(node) {
	if (hasSyntacticModifier(node, ModifierFlags.Modifier)) return node.modifiers?.filter((modifier) => modifier.kind !== K.Decorator);
	return undefined;
}

function getDecorators(node) {
	if (hasDecorators(node)) return node.modifiers?.filter((modifier) => modifier.kind === K.Decorator);
	return undefined;
}

function walkUpBindingElementsAndPatterns(binding) {
	let node = binding.parent;
	while (node.parent.kind === K.BindingElement) node = node.parent.parent;
	return node.parent;
}

function getCombinedFlags(node, getFlags) {
	if (node.kind === K.BindingElement) node = walkUpBindingElementsAndPatterns(node);
	let flags = getFlags(node);
	if (node.kind === K.VariableDeclaration) node = node.parent;
	if (node && node.kind === K.VariableDeclarationList) {
		flags |= getFlags(node);
		node = node.parent;
	}
	if (node && node.kind === K.VariableStatement) flags |= getFlags(node);
	return flags;
}

function getCombinedModifierFlags(node) {
	return getCombinedFlags(node, getEffectiveModifierFlags);
}

function getCombinedNodeFlags(node) {
	return getCombinedFlags(node, (current) => current.flags);
}

// --- syntax predicates ------------------------------------------------------

function isFunctionLikeDeclarationKind(kind) {
	switch (kind) {
		case K.FunctionDeclaration:
		case K.MethodDeclaration:
		case K.Constructor:
		case K.GetAccessor:
		case K.SetAccessor:
		case K.FunctionExpression:
		case K.ArrowFunction:
			return true;
	}
	return false;
}

function isFunctionLikeKind(kind) {
	switch (kind) {
		case K.MethodSignature:
		case K.CallSignature:
		case K.JSDocSignature:
		case K.ConstructSignature:
		case K.IndexSignature:
		case K.FunctionType:
		case K.JSDocFunctionType:
		case K.ConstructorType:
			return true;
	}
	return isFunctionLikeDeclarationKind(kind);
}

const isFunctionLike = (node) => !!node && isFunctionLikeKind(node.kind);
const isFunctionLikeDeclaration = (node) => !!node && isFunctionLikeDeclarationKind(node.kind);
const isFunctionBody = (node) => node.kind === K.Block;
const isNodeKind = (kind) => kind >= K.FirstNode;
const isStringLiteralLike = (node) => node.kind === K.StringLiteral || node.kind === K.NoSubstitutionTemplateLiteral;
const isStringOrNumericLiteralLike = (node) => isStringLiteralLike(node) || node.kind === K.NumericLiteral;
const isAnyImportSyntax = (node) => node.kind === K.ImportDeclaration || node.kind === K.ImportEqualsDeclaration;
const isJsonSourceFile = (file) => file.scriptKind === ScriptKind.JSON;
const positionIsSynthesized = (pos) => !(pos >= 0);

function isIterationStatement(node, lookInLabeledStatements) {
	switch (node.kind) {
		case K.ForStatement:
		case K.ForInStatement:
		case K.ForOfStatement:
		case K.DoStatement:
		case K.WhileStatement:
			return true;
		case K.LabeledStatement:
			return !!lookInLabeledStatements && isIterationStatement(node.statement, lookInLabeledStatements);
	}
	return false;
}

function isOptionalChain(node) {
	const kind = node.kind;
	return (
		!!(node.flags & NodeFlags.OptionalChain) &&
		(kind === K.PropertyAccessExpression || kind === K.ElementAccessExpression || kind === K.CallExpression || kind === K.NonNullExpression)
	);
}

function isSignedNumericLiteral(node) {
	return (
		node.kind === K.PrefixUnaryExpression &&
		(node.operator === K.PlusToken || node.operator === K.MinusToken) &&
		node.operand.kind === K.NumericLiteral
	);
}

function isParameterPropertyDeclaration(node, parent) {
	return node.kind === K.Parameter && hasSyntacticModifier(node, ModifierFlags.ParameterPropertyModifier) && parent.kind === K.Constructor;
}

function isAutoAccessorPropertyDeclaration(node) {
	return node.kind === K.PropertyDeclaration && hasAccessorModifier(node);
}

function isTypeOnlyExportDeclaration(node) {
	switch (node.kind) {
		case K.ExportSpecifier:
			return node.isTypeOnly || node.parent.parent.isTypeOnly;
		case K.ExportDeclaration:
			return node.isTypeOnly && !!node.moduleSpecifier && !node.exportClause;
		case K.NamespaceExport:
			return node.parent.isTypeOnly;
	}
	return false;
}

function isImportCall(node) {
	if (node.kind !== K.CallExpression) return false;
	const expression = node.expression;
	return (
		expression.kind === K.ImportKeyword ||
		(expression.kind === K.MetaProperty && expression.keywordToken === K.ImportKeyword && expression.name.escapedText === "defer")
	);
}

const isSuperCall = (node) => node.kind === K.CallExpression && node.expression.kind === K.SuperKeyword;

function isSuperProperty(node) {
	return (node.kind === K.PropertyAccessExpression || node.kind === K.ElementAccessExpression) && node.expression.kind === K.SuperKeyword;
}

const isThisIdentifier = (node) => !!node && node.kind === K.Identifier && node.escapedText === "this";

function skipParentheses(node) {
	while (node.kind === K.ParenthesizedExpression) node = node.expression;
	return node;
}

function isPropertyAccessEntityNameExpression(node) {
	return node.kind === K.PropertyAccessExpression && node.name.kind === K.Identifier && isEntityNameExpression(node.expression);
}

function isEntityNameExpression(node) {
	return node.kind === K.Identifier || isPropertyAccessEntityNameExpression(node);
}

function isLiteralLikeElementAccess(node) {
	return node.kind === K.ElementAccessExpression && isStringOrNumericLiteralLike(node.argumentExpression);
}

function isBindableStaticNameExpression(node, excludeThisKeyword) {
	return isEntityNameExpression(node) || isBindableStaticAccessExpression(node, excludeThisKeyword);
}

function isBindableStaticElementAccessExpression(node, excludeThisKeyword) {
	return (
		isLiteralLikeElementAccess(node) &&
		((!excludeThisKeyword && node.expression.kind === K.ThisKeyword) ||
			isEntityNameExpression(node.expression) ||
			isBindableStaticAccessExpression(node.expression, true))
	);
}

function isBindableStaticAccessExpression(node, excludeThisKeyword) {
	return (
		(node.kind === K.PropertyAccessExpression &&
			((!excludeThisKeyword && node.expression.kind === K.ThisKeyword) ||
				(node.name.kind === K.Identifier && isBindableStaticNameExpression(node.expression, true)))) ||
		isBindableStaticElementAccessExpression(node, excludeThisKeyword)
	);
}

function getElementOrPropertyAccessName(node) {
	let name;
	if (node.kind === K.PropertyAccessExpression) name = node.name;
	else {
		const argument = skipParentheses(node.argumentExpression);
		name = argument.kind === K.NumericLiteral || isStringLiteralLike(argument) ? argument : node;
	}
	if (name.kind === K.Identifier) return name.escapedText;
	if (isStringLiteralLike(name) || name.kind === K.NumericLiteral) return escapeLeadingUnderscores(name.text);
	return undefined;
}

function isPrototypeAccess(node) {
	return isBindableStaticAccessExpression(node) && getElementOrPropertyAccessName(node) === "prototype";
}

const isAssignmentOperator = (token) => token >= K.FirstAssignment && token <= K.LastAssignment;

function isLogicalOrCoalescingAssignmentOperator(token) {
	return token === K.BarBarEqualsToken || token === K.AmpersandAmpersandEqualsToken || token === K.QuestionQuestionEqualsToken;
}

function isLogicalOrCoalescingAssignmentExpression(node) {
	return node.kind === K.BinaryExpression && isLogicalOrCoalescingAssignmentOperator(node.operatorToken.kind);
}

function isLeftHandSideExpressionKind(kind) {
	switch (kind) {
		case K.PropertyAccessExpression:
		case K.ElementAccessExpression:
		case K.NewExpression:
		case K.CallExpression:
		case K.JsxElement:
		case K.JsxSelfClosingElement:
		case K.JsxFragment:
		case K.TaggedTemplateExpression:
		case K.ArrayLiteralExpression:
		case K.ParenthesizedExpression:
		case K.ObjectLiteralExpression:
		case K.ClassExpression:
		case K.FunctionExpression:
		case K.Identifier:
		case K.PrivateIdentifier:
		case K.RegularExpressionLiteral:
		case K.NumericLiteral:
		case K.BigIntLiteral:
		case K.StringLiteral:
		case K.NoSubstitutionTemplateLiteral:
		case K.TemplateExpression:
		case K.FalseKeyword:
		case K.NullKeyword:
		case K.ThisKeyword:
		case K.TrueKeyword:
		case K.SuperKeyword:
		case K.NonNullExpression:
		case K.ExpressionWithTypeArguments:
		case K.MetaProperty:
		case K.ImportKeyword:
		case K.MissingDeclaration:
			return true;
	}
	return false;
}

function isAssignmentExpression(node, excludeCompoundAssignment) {
	return (
		node.kind === K.BinaryExpression &&
		(excludeCompoundAssignment ? node.operatorToken.kind === K.EqualsToken : isAssignmentOperator(node.operatorToken.kind)) &&
		isLeftHandSideExpressionKind(skipPartiallyEmittedExpressions(node.left).kind)
	);
}

function skipPartiallyEmittedExpressions(node) {
	while (node.kind === K.PartiallyEmittedExpression) node = node.expression;
	return node;
}

function getAssignmentTarget(node) {
	let parent = node.parent;
	while (true) {
		switch (parent.kind) {
			case K.BinaryExpression:
				return isAssignmentOperator(parent.operatorToken.kind) && parent.left === node ? parent : undefined;
			case K.PrefixUnaryExpression:
			case K.PostfixUnaryExpression:
				return parent.operator === K.PlusPlusToken || parent.operator === K.MinusMinusToken ? parent : undefined;
			case K.ForInStatement:
			case K.ForOfStatement:
				return parent.initializer === node ? parent : undefined;
			case K.ParenthesizedExpression:
			case K.ArrayLiteralExpression:
			case K.SpreadElement:
			case K.NonNullExpression:
				node = parent;
				break;
			case K.SpreadAssignment:
				node = parent.parent;
				break;
			case K.ShorthandPropertyAssignment:
				if (parent.name !== node) return undefined;
				node = parent.parent;
				break;
			case K.PropertyAssignment:
				if (parent.name === node) return undefined;
				node = parent.parent;
				break;
			default:
				return undefined;
		}
		parent = node.parent;
	}
}

const AssignmentKind = { None: 0, Definite: 1, Compound: 2 };

function getAssignmentTargetKind(node) {
	const target = getAssignmentTarget(node);
	if (!target) return AssignmentKind.None;
	switch (target.kind) {
		case K.BinaryExpression: {
			const operator = target.operatorToken.kind;
			return operator === K.EqualsToken || isLogicalOrCoalescingAssignmentOperator(operator) ? AssignmentKind.Definite : AssignmentKind.Compound;
		}
		case K.PrefixUnaryExpression:
		case K.PostfixUnaryExpression:
			return AssignmentKind.Compound;
		case K.ForInStatement:
		case K.ForOfStatement:
			return AssignmentKind.Definite;
	}
	return AssignmentKind.None;
}

const isAssignmentTarget = (node) => !!getAssignmentTarget(node);

const AccessKind = { Read: 0, Write: 1, ReadWrite: 2 };

function reverseAccessKind(kind) {
	return kind === AccessKind.Read ? AccessKind.Write : kind === AccessKind.Write ? AccessKind.Read : AccessKind.ReadWrite;
}

function accessKind(node) {
	const parent = node.parent;
	switch (parent?.kind) {
		case K.ParenthesizedExpression:
			return accessKind(parent);
		case K.PostfixUnaryExpression:
		case K.PrefixUnaryExpression:
			return parent.operator === K.PlusPlusToken || parent.operator === K.MinusMinusToken ? AccessKind.ReadWrite : AccessKind.Read;
		case K.BinaryExpression:
			if (parent.left !== node || !isAssignmentOperator(parent.operatorToken.kind)) return AccessKind.Read;
			return parent.operatorToken.kind === K.EqualsToken ? AccessKind.Write : AccessKind.ReadWrite;
		case K.PropertyAccessExpression:
			return parent.name !== node ? AccessKind.Read : accessKind(parent);
		case K.PropertyAssignment: {
			const parentAccess = accessKind(parent.parent);
			return node === parent.name ? reverseAccessKind(parentAccess) : parentAccess;
		}
		case K.ShorthandPropertyAssignment:
			return node === parent.objectAssignmentInitializer ? AccessKind.Read : accessKind(parent.parent);
		case K.ArrayLiteralExpression:
			return accessKind(parent);
		case K.ForInStatement:
		case K.ForOfStatement:
			return node === parent.initializer ? AccessKind.Write : AccessKind.Read;
	}
	return AccessKind.Read;
}

const isWriteAccess = (node) => accessKind(node) !== AccessKind.Read;
const isWriteOnlyAccess = (node) => accessKind(node) === AccessKind.Write;

function findAncestor(node, callback) {
	while (node) {
		const result = callback(node);
		if (result === "quit") return undefined;
		if (result) return node;
		node = node.parent;
	}
	return undefined;
}

function isNodeDescendantOf(node, ancestor) {
	while (node) {
		if (node === ancestor) return true;
		node = node.parent;
	}
	return false;
}

const isClassLikeKind = (kind) => kind === K.ClassDeclaration || kind === K.ClassExpression;

function isClassElementKind(kind) {
	switch (kind) {
		case K.Constructor:
		case K.PropertyDeclaration:
		case K.MethodDeclaration:
		case K.GetAccessor:
		case K.SetAccessor:
		case K.IndexSignature:
		case K.ClassStaticBlockDeclaration:
		case K.SemicolonClassElement:
			return true;
	}
	return false;
}

function getThisContainer(node, includeArrowFunctions, includeClassComputedPropertyName) {
	if (node.kind === K.SourceFile) throw new Error("surge-ts: getThisContainer of a source file");
	while (true) {
		node = node.parent;
		if (!node) throw new Error("surge-ts: getThisContainer reached no container");
		switch (node.kind) {
			case K.ComputedPropertyName:
				if (includeClassComputedPropertyName && isClassLikeKind(node.parent.parent.kind)) return node;
				node = node.parent.parent;
				break;
			case K.Decorator:
				if (node.parent.kind === K.Parameter && isClassElementKind(node.parent.parent.kind)) node = node.parent.parent;
				else if (isClassElementKind(node.parent.kind)) node = node.parent;
				break;
			case K.ArrowFunction:
				if (!includeArrowFunctions) continue;
			// falls through
			case K.FunctionDeclaration:
			case K.FunctionExpression:
			case K.ModuleDeclaration:
			case K.ClassStaticBlockDeclaration:
			case K.PropertyDeclaration:
			case K.PropertySignature:
			case K.MethodDeclaration:
			case K.MethodSignature:
			case K.Constructor:
			case K.GetAccessor:
			case K.SetAccessor:
			case K.CallSignature:
			case K.ConstructSignature:
			case K.IndexSignature:
			case K.EnumDeclaration:
			case K.SourceFile:
				return node;
		}
	}
}

function getHeritageClause(clauses, kind) {
	return clauses?.find((clause) => clause.token === kind);
}

function getInterfaceBaseTypeNodes(node) {
	return getHeritageClause(node.heritageClauses, K.ExtendsKeyword)?.types;
}

// TypeScript reads `@augments` / `@implements` for a JavaScript class; surge-ts
// reads the heritage clauses written in the syntax only.
function getEffectiveBaseTypeNode(node) {
	const clause = getHeritageClause(node.heritageClauses, K.ExtendsKeyword);
	return clause && clause.types.length > 0 ? clause.types[0] : undefined;
}

function getEffectiveImplementsTypeNodes(node) {
	return getHeritageClause(node.heritageClauses, K.ImplementsKeyword)?.types;
}

function getAllSuperTypeNodes(node) {
	if (node.kind === K.InterfaceDeclaration) return getInterfaceBaseTypeNodes(node) ?? [];
	if (isClassLikeKind(node.kind)) {
		const base = getEffectiveBaseTypeNode(node);
		return [...(base ? [base] : []), ...(getEffectiveImplementsTypeNodes(node) ?? [])];
	}
	return [];
}

const getTextOfJsxNamespacedName = (node) => `${node.namespace.text}:${node.name.text}`;
const getEscapedTextOfJsxNamespacedName = (node) => `${node.namespace.escapedText}:${node.name.text}`;

function getPropertyNameForPropertyNameNode(name) {
	switch (name.kind) {
		case K.Identifier:
		case K.PrivateIdentifier:
			return name.escapedText;
		case K.StringLiteral:
		case K.NoSubstitutionTemplateLiteral:
		case K.NumericLiteral:
		case K.BigIntLiteral:
			return escapeLeadingUnderscores(name.text);
		case K.ComputedPropertyName: {
			const expression = name.expression;
			if (isStringOrNumericLiteralLike(expression)) return escapeLeadingUnderscores(expression.text);
			if (isSignedNumericLiteral(expression)) {
				return expression.operator === K.MinusToken ? `-${expression.operand.text}` : expression.operand.text;
			}
			return undefined;
		}
		case K.JsxNamespacedName:
			return getEscapedTextOfJsxNamespacedName(name);
	}
	throw new Error(`surge-ts: getPropertyNameForPropertyNameNode of a ${K[name.kind]}`);
}

function skipAlias(symbol, checker) {
	return symbol.flags & SymbolFlags.Alias ? checker.getAliasedSymbol(symbol) : symbol;
}

// --- module instance state (the binder's getModuleInstanceState) -------------

const ModuleInstanceState = { NonInstantiated: 0, Instantiated: 1, ConstEnumOnly: 2 };

function nodeHasName(statement, name) {
	if (statement.name?.kind === K.Identifier && statement.name.escapedText === name.escapedText) return true;
	if (statement.kind === K.VariableStatement) {
		return statement.declarationList.declarations.some((declaration) => nodeHasName(declaration, name));
	}
	return false;
}

function getModuleInstanceStateForAliasTarget(specifier, visited) {
	const name = specifier.propertyName || specifier.name;
	if (name.kind !== K.Identifier) return ModuleInstanceState.Instantiated;
	for (let scope = specifier.parent; scope; scope = scope.parent) {
		if (scope.kind !== K.Block && scope.kind !== K.ModuleBlock && scope.kind !== K.SourceFile) continue;
		let found;
		for (const statement of scope.statements) {
			if (!nodeHasName(statement, name)) continue;
			const state = getModuleInstanceStateCached(statement, visited);
			if (found === undefined || state > found) found = state;
			if (found === ModuleInstanceState.Instantiated) return found;
			if (statement.kind === K.ImportEqualsDeclaration) found = ModuleInstanceState.Instantiated;
		}
		if (found !== undefined) return found;
	}
	return ModuleInstanceState.Instantiated;
}

function getModuleInstanceStateWorker(node, visited) {
	switch (node.kind) {
		case K.InterfaceDeclaration:
		case K.TypeAliasDeclaration:
			return ModuleInstanceState.NonInstantiated;
		case K.EnumDeclaration:
			if (hasSyntacticModifier(node, ModifierFlags.Const)) return ModuleInstanceState.ConstEnumOnly;
			break;
		case K.ImportDeclaration:
		case K.ImportEqualsDeclaration:
			if (!hasSyntacticModifier(node, ModifierFlags.Export)) return ModuleInstanceState.NonInstantiated;
			break;
		case K.ExportDeclaration:
			if (!node.moduleSpecifier && node.exportClause?.kind === K.NamedExports) {
				let state = ModuleInstanceState.NonInstantiated;
				for (const specifier of node.exportClause.elements) {
					state = Math.max(state, getModuleInstanceStateForAliasTarget(specifier, visited));
					if (state === ModuleInstanceState.Instantiated) return state;
				}
				return state;
			}
			break;
		case K.ModuleBlock: {
			let state = ModuleInstanceState.NonInstantiated;
			forEachChild(node, (child) => {
				const childState = getModuleInstanceStateCached(child, visited);
				if (childState === ModuleInstanceState.ConstEnumOnly) state = childState;
				else if (childState === ModuleInstanceState.Instantiated) {
					state = childState;
					return true;
				}
			});
			return state;
		}
		case K.ModuleDeclaration:
			return getModuleInstanceState(node, visited);
		case K.Identifier:
			if (node.flags & NodeFlags.IdentifierIsInJSDocNamespace) return ModuleInstanceState.NonInstantiated;
	}
	return ModuleInstanceState.Instantiated;
}

function getModuleInstanceStateCached(node, visited = new Map()) {
	if (visited.has(node)) return visited.get(node) ?? ModuleInstanceState.NonInstantiated;
	visited.set(node, undefined);
	const result = getModuleInstanceStateWorker(node, visited);
	visited.set(node, result);
	return result;
}

function getModuleInstanceState(node, visited) {
	return node.body ? getModuleInstanceStateCached(node.body, visited) : ModuleInstanceState.Instantiated;
}

function isInstantiatedModule(node, preserveConstEnums) {
	const state = getModuleInstanceState(node);
	return state === ModuleInstanceState.Instantiated || (!!preserveConstEnums && state === ModuleInstanceState.ConstEnumOnly);
}

// --- scanner ----------------------------------------------------------------

function isLineBreak(ch) {
	return ch === 10 || ch === 13 || ch === 0x2028 || ch === 0x2029;
}

function isWhiteSpaceSingleLine(ch) {
	return (
		ch === 32 ||
		ch === 9 ||
		ch === 11 ||
		ch === 12 ||
		ch === 160 ||
		ch === 133 ||
		ch === 5760 ||
		(ch >= 8192 && ch <= 8203) ||
		ch === 8239 ||
		ch === 8287 ||
		ch === 12288 ||
		ch === 65279
	);
}

const isWhiteSpaceLike = (ch) => isWhiteSpaceSingleLine(ch) || isLineBreak(ch);

const utf16EncodeAsString = (codePoint) => String.fromCodePoint(codePoint);

function getShebang(text) {
	return /^#!.*/.exec(text)?.[0];
}

/** TypeScript's `iterateCommentRanges`, collecting into an array. */
function collectCommentRanges(text, pos, trailing) {
	let comments;
	let pending;
	let collecting = trailing;
	if (pos === 0) {
		collecting = true;
		const shebang = getShebang(text);
		if (shebang) pos = shebang.length;
	}
	scan: while (pos >= 0 && pos < text.length) {
		const ch = text.charCodeAt(pos);
		switch (ch) {
			case 13:
				if (text.charCodeAt(pos + 1) === 10) pos++;
			// falls through
			case 10:
				pos++;
				if (trailing) break scan;
				collecting = true;
				if (pending) pending.hasTrailingNewLine = true;
				continue;
			case 9:
			case 11:
			case 12:
			case 32:
				pos++;
				continue;
			case 47: {
				const next = text.charCodeAt(pos + 1);
				if (next !== 47 && next !== 42) break scan;
				const kind = next === 47 ? K.SingleLineCommentTrivia : K.MultiLineCommentTrivia;
				const start = pos;
				let hasTrailingNewLine = false;
				pos += 2;
				if (next === 47) {
					while (pos < text.length) {
						if (isLineBreak(text.charCodeAt(pos))) {
							hasTrailingNewLine = true;
							break;
						}
						pos++;
					}
				} else {
					while (pos < text.length) {
						if (text.charCodeAt(pos) === 42 && text.charCodeAt(pos + 1) === 47) {
							pos += 2;
							break;
						}
						pos++;
					}
				}
				if (collecting) {
					if (pending) (comments ??= []).push(pending);
					pending = { kind, pos: start, end: pos, hasTrailingNewLine };
				}
				continue;
			}
			default:
				if (ch > 127 && isWhiteSpaceLike(ch)) {
					if (pending && isLineBreak(ch)) pending.hasTrailingNewLine = true;
					pos++;
					continue;
				}
				break scan;
		}
	}
	if (pending) (comments ??= []).push(pending);
	return comments;
}

const getLeadingCommentRanges = (text, pos) => collectCommentRanges(text, pos, false);
const getTrailingCommentRanges = (text, pos) => collectCommentRanges(text, pos, true);

// --- compiler options -------------------------------------------------------

const getEmitDeclarations = (options) => !!(options.declaration || options.composite);
const isIncrementalCompilation = (options) => !!(options.incremental || options.composite);

function getTsBuildInfoEmitOutputFilePath(options) {
	if (!isIncrementalCompilation(options) && !options.tscBuild) return undefined;
	if (options.tsBuildInfoFile) return options.tsBuildInfoFile;
	let extensionless;
	if (options.outFile) {
		extensionless = paths.removeFileExtension(options.outFile);
	} else {
		const configFile = options.configFilePath;
		if (!configFile) return undefined;
		const configExtensionless = paths.removeFileExtension(configFile);
		extensionless = options.outDir
			? options.rootDir
				? paths.resolvePath(options.outDir, paths.getRelativePathFromDirectory(options.rootDir, configExtensionless, true))
				: paths.combinePaths(options.outDir, paths.getBaseFileName(configExtensionless))
			: configExtensionless;
	}
	return `${extensionless}.tsbuildinfo`;
}

function isSolutionConfig(config) {
	return !config.fileNames.length && Object.prototype.hasOwnProperty.call(config.raw ?? {}, "references");
}

module.exports = {
	identity,
	returnTrue,
	returnFalse,
	returnUndefined,
	noop,
	arrayFrom,
	modifierToFlag,
	modifiersToFlags,
	canHaveModifiers,
	canHaveDecorators,
	getSyntacticModifierFlags,
	getSyntacticModifierFlagsNoCache,
	getEffectiveModifierFlags,
	hasSyntacticModifier,
	hasEffectiveModifier,
	hasStaticModifier,
	hasAbstractModifier,
	hasAccessorModifier,
	hasDecorators,
	getModifiers,
	getDecorators,
	getCombinedModifierFlags,
	getCombinedNodeFlags,
	isFunctionLike,
	isFunctionLikeKind,
	isFunctionLikeDeclaration,
	isFunctionBody,
	isNodeKind,
	isIterationStatement,
	isOptionalChain,
	isStringLiteralLike,
	isStringOrNumericLiteralLike,
	isSignedNumericLiteral,
	isParameterPropertyDeclaration,
	isAutoAccessorPropertyDeclaration,
	isTypeOnlyExportDeclaration,
	isAnyImportSyntax,
	isImportCall,
	isSuperCall,
	isSuperProperty,
	isThisIdentifier,
	isJsonSourceFile,
	positionIsSynthesized,
	skipParentheses,
	isEntityNameExpression,
	isPropertyAccessEntityNameExpression,
	isBindableStaticAccessExpression,
	isPrototypeAccess,
	isAssignmentOperator,
	isAssignmentExpression,
	isLogicalOrCoalescingAssignmentOperator,
	isLogicalOrCoalescingAssignmentExpression,
	isAssignmentTarget,
	getAssignmentTargetKind,
	isWriteAccess,
	isWriteOnlyAccess,
	findAncestor,
	isNodeDescendantOf,
	getThisContainer,
	getInterfaceBaseTypeNodes,
	getEffectiveBaseTypeNode,
	getEffectiveImplementsTypeNodes,
	getAllSuperTypeNodes,
	getTextOfJsxNamespacedName,
	getEscapedTextOfJsxNamespacedName,
	getPropertyNameForPropertyNameNode,
	skipAlias,
	isInstantiatedModule,
	isLineBreak,
	isWhiteSpaceSingleLine,
	isWhiteSpaceLike,
	utf16EncodeAsString,
	getShebang,
	getLeadingCommentRanges,
	getTrailingCommentRanges,
	getEmitDeclarations,
	isIncrementalCompilation,
	getTsBuildInfoEmitOutputFilePath,
	isSolutionConfig,
};
