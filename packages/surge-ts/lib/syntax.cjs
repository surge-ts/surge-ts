"use strict";

// TypeScript's public syntax and text-span helpers: pure functions over nodes,
// kinds and spans, written as typescript@6.0.3 writes them.

const { SyntaxKind, NodeFlags } = require("./enums.cjs");
const { findAncestor, isFunctionLike, isClassLike } = require("./utilities.cjs");
const { isRootedDiskPath } = require("./paths.cjs");
const tokenKinds = require("./token-texts.json");

const tokenStrings = [];
for (const [text, kind] of Object.entries(tokenKinds)) tokenStrings[kind] = text;

// --- expression kinds --------------------------------------------------------

const LEFT_HAND_SIDE_EXPRESSION_KINDS = new Set([
	SyntaxKind.PropertyAccessExpression,
	SyntaxKind.ElementAccessExpression,
	SyntaxKind.NewExpression,
	SyntaxKind.CallExpression,
	SyntaxKind.JsxElement,
	SyntaxKind.JsxSelfClosingElement,
	SyntaxKind.JsxFragment,
	SyntaxKind.TaggedTemplateExpression,
	SyntaxKind.ArrayLiteralExpression,
	SyntaxKind.ParenthesizedExpression,
	SyntaxKind.ObjectLiteralExpression,
	SyntaxKind.ClassExpression,
	SyntaxKind.FunctionExpression,
	SyntaxKind.Identifier,
	SyntaxKind.PrivateIdentifier,
	SyntaxKind.RegularExpressionLiteral,
	SyntaxKind.NumericLiteral,
	SyntaxKind.BigIntLiteral,
	SyntaxKind.StringLiteral,
	SyntaxKind.NoSubstitutionTemplateLiteral,
	SyntaxKind.TemplateExpression,
	SyntaxKind.FalseKeyword,
	SyntaxKind.NullKeyword,
	SyntaxKind.ThisKeyword,
	SyntaxKind.TrueKeyword,
	SyntaxKind.SuperKeyword,
	SyntaxKind.NonNullExpression,
	SyntaxKind.ExpressionWithTypeArguments,
	SyntaxKind.MetaProperty,
	SyntaxKind.ImportKeyword,
	SyntaxKind.MissingDeclaration,
]);

const UNARY_EXPRESSION_KINDS = new Set([
	SyntaxKind.PrefixUnaryExpression,
	SyntaxKind.PostfixUnaryExpression,
	SyntaxKind.DeleteExpression,
	SyntaxKind.TypeOfExpression,
	SyntaxKind.VoidExpression,
	SyntaxKind.AwaitExpression,
	SyntaxKind.TypeAssertionExpression,
]);

const EXPRESSION_KINDS = new Set([
	SyntaxKind.ConditionalExpression,
	SyntaxKind.YieldExpression,
	SyntaxKind.ArrowFunction,
	SyntaxKind.BinaryExpression,
	SyntaxKind.SpreadElement,
	SyntaxKind.AsExpression,
	SyntaxKind.OmittedExpression,
	SyntaxKind.CommaListExpression,
	SyntaxKind.PartiallyEmittedExpression,
	SyntaxKind.SatisfiesExpression,
]);

function isLeftHandSideExpressionKind(kind) {
	return LEFT_HAND_SIDE_EXPRESSION_KINDS.has(kind);
}

function isUnaryExpressionKind(kind) {
	return UNARY_EXPRESSION_KINDS.has(kind) || isLeftHandSideExpressionKind(kind);
}

function isExpressionKind(kind) {
	return EXPRESSION_KINDS.has(kind) || isUnaryExpressionKind(kind);
}

function skipPartiallyEmittedExpressions(node) {
	while (node.kind === SyntaxKind.PartiallyEmittedExpression) node = node.expression;
	return node;
}

function isExpression(node) {
	return isExpressionKind(skipPartiallyEmittedExpressions(node).kind);
}

function isLeftHandSideExpression(node) {
	return isLeftHandSideExpressionKind(skipPartiallyEmittedExpressions(node).kind);
}

function isOptionalChainOf(kind) {
	return (node) => node.kind === kind && (node.flags & NodeFlags.OptionalChain) !== 0;
}

const isCallChain = isOptionalChainOf(SyntaxKind.CallExpression);
const isPropertyAccessChain = isOptionalChainOf(SyntaxKind.PropertyAccessExpression);
const isElementAccessChain = isOptionalChainOf(SyntaxKind.ElementAccessExpression);
const isNonNullChain = isOptionalChainOf(SyntaxKind.NonNullExpression);

function isNullishCoalesce(node) {
	return node.kind === SyntaxKind.BinaryExpression && node.operatorToken.kind === SyntaxKind.QuestionQuestionToken;
}

function isCallLikeExpression(node) {
	switch (node.kind) {
		case SyntaxKind.CallExpression:
		case SyntaxKind.NewExpression:
		case SyntaxKind.TaggedTemplateExpression:
		case SyntaxKind.Decorator:
		case SyntaxKind.JsxOpeningElement:
		case SyntaxKind.JsxSelfClosingElement:
		case SyntaxKind.JsxOpeningFragment:
			return true;
		case SyntaxKind.BinaryExpression:
			return node.operatorToken.kind === SyntaxKind.InstanceOfKeyword;
		default:
			return false;
	}
}

function isConciseBody(node) {
	return node.kind === SyntaxKind.Block || isExpression(node);
}

function isForInitializer(node) {
	return node.kind === SyntaxKind.VariableDeclarationList || isExpression(node);
}

function isConstTypeReference(node) {
	return (
		node.kind === SyntaxKind.TypeReference &&
		node.typeName.kind === SyntaxKind.Identifier &&
		node.typeName.escapedText === "const" &&
		!node.typeArguments
	);
}

// --- bindings and declarations ----------------------------------------------

function isBindingPatternKind(kind) {
	return kind === SyntaxKind.ObjectBindingPattern || kind === SyntaxKind.ArrayBindingPattern;
}

function isEmptyBindingElement(node) {
	return node.kind === SyntaxKind.OmittedExpression || isEmptyBindingPattern(node.name);
}

function isEmptyBindingPattern(node) {
	return isBindingPatternKind(node.kind) && node.elements.every(isEmptyBindingElement);
}

function walkUpBindingElementsAndPatterns(binding) {
	let node = binding.parent;
	while (node.parent.kind === SyntaxKind.BindingElement) node = node.parent.parent;
	return node.parent;
}

function isRestParameter(node) {
	const type = node.kind === SyntaxKind.JSDocParameterTag ? node.typeExpression && node.typeExpression.type : node.type;
	return node.dotDotDotToken !== undefined || (!!type && type.kind === SyntaxKind.JSDocVariadicType);
}

function hasRestParameter(signature) {
	const parameters = signature.parameters;
	return !!parameters?.length && isRestParameter(parameters[parameters.length - 1]);
}

function hasOnlyExpressionInitializer(node) {
	switch (node.kind) {
		case SyntaxKind.VariableDeclaration:
		case SyntaxKind.Parameter:
		case SyntaxKind.BindingElement:
		case SyntaxKind.PropertyDeclaration:
		case SyntaxKind.PropertyAssignment:
		case SyntaxKind.EnumMember:
			return true;
		default:
			return false;
	}
}

/** `getNameOfDeclaration`, for the declarations TypeScript files write. */
function getNameOfDeclaration(declaration) {
	if (declaration === undefined) return undefined;
	switch (declaration.kind) {
		case SyntaxKind.Identifier:
			return declaration;
		case SyntaxKind.JSDocPropertyTag:
		case SyntaxKind.JSDocParameterTag:
			return declaration.name.kind === SyntaxKind.QualifiedName ? declaration.name.right : declaration.name;
		case SyntaxKind.JSDocTypedefTag:
			return getNameOfJSDocTypedef(declaration);
		case SyntaxKind.ExportAssignment:
			return declaration.expression.kind === SyntaxKind.Identifier ? declaration.expression : undefined;
		case SyntaxKind.ElementAccessExpression:
			if (isBindableStaticElementAccessExpression(declaration)) return declaration.argumentExpression;
	}
	if (declaration.name) return declaration.name;
	switch (declaration.kind) {
		case SyntaxKind.FunctionExpression:
		case SyntaxKind.ArrowFunction:
		case SyntaxKind.ClassExpression:
			return getAssignedName(declaration);
	}
	return undefined;
}

/** tsc's `getAssignedName`: what a nameless function or class expression is assigned to. */
function getAssignedName(node) {
	const parent = node.parent;
	if (!parent) return undefined;
	switch (parent.kind) {
		case SyntaxKind.PropertyAssignment:
		case SyntaxKind.BindingElement:
			return parent.name;
		case SyntaxKind.BinaryExpression:
			if (node !== parent.right) return undefined;
			if (parent.left.kind === SyntaxKind.Identifier) return parent.left;
			return isAccessExpression(parent.left) ? getElementOrPropertyAccessArgumentExpressionOrName(parent.left) : undefined;
		case SyntaxKind.VariableDeclaration:
			return parent.name.kind === SyntaxKind.Identifier ? parent.name : undefined;
	}
	return undefined;
}

function isAccessExpression(node) {
	return node.kind === SyntaxKind.PropertyAccessExpression || node.kind === SyntaxKind.ElementAccessExpression;
}

function isStringOrNumericLiteralLike(node) {
	return node.kind === SyntaxKind.StringLiteral || node.kind === SyntaxKind.NoSubstitutionTemplateLiteral || node.kind === SyntaxKind.NumericLiteral;
}

function getElementOrPropertyAccessArgumentExpressionOrName(node) {
	if (node.kind === SyntaxKind.PropertyAccessExpression) return node.name;
	let argument = node.argumentExpression;
	while (argument.kind === SyntaxKind.ParenthesizedExpression) argument = argument.expression;
	return isStringOrNumericLiteralLike(argument) ? argument : node;
}

function isEntityNameExpression(node) {
	return (
		node.kind === SyntaxKind.Identifier ||
		(node.kind === SyntaxKind.PropertyAccessExpression && node.name.kind === SyntaxKind.Identifier && isEntityNameExpression(node.expression))
	);
}

function isBindableStaticElementAccessExpression(node, excludeThisKeyword) {
	return (
		node.kind === SyntaxKind.ElementAccessExpression &&
		isStringOrNumericLiteralLike(node.argumentExpression) &&
		((!excludeThisKeyword && node.expression.kind === SyntaxKind.ThisKeyword) ||
			isEntityNameExpression(node.expression) ||
			isBindableStaticAccessExpression(node.expression, true))
	);
}

function isBindableStaticAccessExpression(node, excludeThisKeyword) {
	return (
		(node.kind === SyntaxKind.PropertyAccessExpression &&
			((!excludeThisKeyword && node.expression.kind === SyntaxKind.ThisKeyword) ||
				(node.name.kind === SyntaxKind.Identifier &&
					(isEntityNameExpression(node.expression) || isBindableStaticAccessExpression(node.expression, true))))) ||
		isBindableStaticElementAccessExpression(node, excludeThisKeyword)
	);
}

function getNameOfJSDocTypedef(declaration) {
	if (declaration.name) return declaration.name;
	// A nameless `@typedef` is named by the variable its comment documents.
	const host = declaration.parent?.parent;
	if (host?.kind === SyntaxKind.VariableStatement) {
		const name = host.declarationList.declarations[0]?.name;
		return name?.kind === SyntaxKind.Identifier ? name : undefined;
	}
	return undefined;
}

function getTypeParameterOwner(declaration) {
	if (declaration && declaration.kind === SyntaxKind.TypeParameter) {
		for (let current = declaration; current; current = current.parent) {
			if (isFunctionLike(current) || isClassLike(current) || current.kind === SyntaxKind.InterfaceDeclaration) return current;
		}
	}
	return undefined;
}

function getEffectiveTypeParameterDeclarations(node) {
	if (node.kind === SyntaxKind.JSDocTypedefTag || node.kind === SyntaxKind.JSDocCallbackTag) {
		return (node.parent.tags ?? []).flatMap((tag) => (tag.kind === SyntaxKind.JSDocTemplateTag ? tag.typeParameters : []));
	}
	return node.typeParameters ?? [];
}

function getEffectiveConstraintOfTypeParameter(node) {
	if (node.constraint) return node.constraint;
	return node.parent?.kind === SyntaxKind.JSDocTemplateTag && node === node.parent.typeParameters[0] ? node.parent.constraint : undefined;
}

// --- imports and exports -----------------------------------------------------

function isTypeOnlyImportDeclaration(node) {
	switch (node.kind) {
		case SyntaxKind.ImportSpecifier:
			return node.isTypeOnly || node.parent.parent.isTypeOnly;
		case SyntaxKind.NamespaceImport:
			return node.parent.isTypeOnly;
		case SyntaxKind.ImportClause:
		case SyntaxKind.ImportEqualsDeclaration:
			return node.isTypeOnly;
	}
	return false;
}

function isTypeOnlyExportDeclaration(node) {
	switch (node.kind) {
		case SyntaxKind.ExportSpecifier:
			return node.isTypeOnly || node.parent.parent.isTypeOnly;
		case SyntaxKind.ExportDeclaration:
			return node.isTypeOnly && !!node.moduleSpecifier && !node.exportClause;
		case SyntaxKind.NamespaceExport:
			return node.parent.isTypeOnly;
	}
	return false;
}

function isTypeOnlyImportOrExportDeclaration(node) {
	return isTypeOnlyImportDeclaration(node) || isTypeOnlyExportDeclaration(node);
}

function isPartOfTypeOnlyImportOrExportDeclaration(node) {
	return findAncestor(node, isTypeOnlyImportOrExportDeclaration) !== undefined;
}

function isExternalModule(file) {
	return file.externalModuleIndicator !== undefined;
}

function isExternalModuleNameRelative(moduleName) {
	return /^\.\.?($|[\\/])/.test(moduleName) || isRootedDiskPath(moduleName);
}

// --- tokens and identifiers --------------------------------------------------

function isTokenKind(kind) {
	return kind >= SyntaxKind.FirstToken && kind <= SyntaxKind.LastToken;
}

function isToken(node) {
	return isTokenKind(node.kind);
}

function tokenToString(kind) {
	return tokenStrings[kind];
}

function identifierToKeywordKind(node) {
	const kind = tokenKinds[node.escapedText];
	return kind !== undefined && kind >= SyntaxKind.FirstKeyword && kind <= SyntaxKind.LastKeyword ? kind : undefined;
}

function isIdentifierStart(ch, _languageVersion) {
	return ch === 0x24 || ch === 0x5f || /\p{ID_Start}/u.test(String.fromCodePoint(ch));
}

function isIdentifierPart(ch, _languageVersion, identifierVariant) {
	return (
		ch === 0x24 ||
		ch === 0x200c ||
		ch === 0x200d ||
		(identifierVariant === 1 /* LanguageVariant.JSX */ && (ch === 0x2d || ch === 0x3a)) ||
		/\p{ID_Continue}/u.test(String.fromCodePoint(ch))
	);
}

function couldStartTrivia(text, pos) {
	const ch = text.charCodeAt(pos);
	switch (ch) {
		case 13:
		case 10:
		case 9:
		case 11:
		case 12:
		case 32:
		case 47:
		case 60:
		case 124:
		case 61:
		case 62:
			return true;
		case 35:
			return pos === 0;
		default:
			return ch > 127;
	}
}

// --- parse tree --------------------------------------------------------------

function isParseTreeNode(node) {
	return (node.flags & NodeFlags.Synthesized) === 0;
}

function getParseTreeNode(node, nodeTest) {
	if (node === undefined || isParseTreeNode(node)) return node;
	for (node = node.original; node; node = node.original) {
		if (isParseTreeNode(node)) return !nodeTest || nodeTest(node) ? node : undefined;
	}
	return undefined;
}

function getOriginalNode(node, nodeTest) {
	if (node) while (node.original !== undefined) node = node.original;
	if (!node || !nodeTest) return node;
	return nodeTest(node) ? node : undefined;
}

// --- text spans --------------------------------------------------------------

function createTextSpan(start, length) {
	if (start < 0) throw new Error("start < 0");
	if (length < 0) throw new Error("length < 0");
	return { start, length };
}

function createTextSpanFromBounds(start, end) {
	return createTextSpan(start, end - start);
}

function textSpanEnd(span) {
	return span.start + span.length;
}

function textSpanIsEmpty(span) {
	return span.length === 0;
}

function textSpanContainsPosition(span, position) {
	return position >= span.start && position < textSpanEnd(span);
}

function textSpanContainsTextSpan(span, other) {
	return other.start >= span.start && textSpanEnd(other) <= textSpanEnd(span);
}

function textSpanIntersection(span1, span2) {
	const start = Math.max(span1.start, span2.start);
	const end = Math.min(textSpanEnd(span1), textSpanEnd(span2));
	return start <= end ? createTextSpanFromBounds(start, end) : undefined;
}

function textSpanOverlap(span1, span2) {
	const overlap = textSpanIntersection(span1, span2);
	return overlap && overlap.length === 0 ? undefined : overlap;
}

function textSpanOverlapsWith(span, other) {
	return textSpanOverlap(span, other) !== undefined;
}

function decodedTextSpanIntersectsWith(start1, length1, start2, length2) {
	return start2 <= start1 + length1 && start2 + length2 >= start1;
}

function textSpanIntersectsWithTextSpan(span, other) {
	return decodedTextSpanIntersectsWith(span.start, span.length, other.start, other.length);
}

function textSpanIntersectsWith(span, start, length) {
	return decodedTextSpanIntersectsWith(span.start, span.length, start, length);
}

function textSpanIntersectsWithPosition(span, position) {
	return position <= textSpanEnd(span) && position >= span.start;
}

function createTextChangeRange(span, newLength) {
	if (newLength < 0) throw new Error("newLength < 0");
	return { span, newLength };
}

function textChangeRangeNewSpan(range) {
	return createTextSpan(range.span.start, range.newLength);
}

function textChangeRangeIsUnchanged(range) {
	return textSpanIsEmpty(range.span) && range.newLength === 0;
}

function displayPartsToString(displayParts) {
	return displayParts ? displayParts.map((part) => part.text).join("") : "";
}

module.exports = {
	isExpression,
	isLeftHandSideExpression,
	skipPartiallyEmittedExpressions,
	isCallChain,
	isPropertyAccessChain,
	isElementAccessChain,
	isNonNullChain,
	isNullishCoalesce,
	isCallLikeExpression,
	isConciseBody,
	isForInitializer,
	isConstTypeReference,
	isEmptyBindingElement,
	isEmptyBindingPattern,
	walkUpBindingElementsAndPatterns,
	isRestParameter,
	hasRestParameter,
	hasOnlyExpressionInitializer,
	getNameOfDeclaration,
	getNameOfJSDocTypedef,
	getTypeParameterOwner,
	getEffectiveTypeParameterDeclarations,
	getEffectiveConstraintOfTypeParameter,
	isTypeOnlyImportDeclaration,
	isTypeOnlyImportOrExportDeclaration,
	isPartOfTypeOnlyImportOrExportDeclaration,
	isExternalModule,
	isExternalModuleNameRelative,
	isTokenKind,
	isToken,
	tokenToString,
	identifierToKeywordKind,
	isIdentifierStart,
	isIdentifierPart,
	couldStartTrivia,
	isParseTreeNode,
	getParseTreeNode,
	getOriginalNode,
	createTextSpan,
	createTextSpanFromBounds,
	textSpanEnd,
	textSpanIsEmpty,
	textSpanContainsPosition,
	textSpanContainsTextSpan,
	textSpanIntersection,
	textSpanOverlap,
	textSpanOverlapsWith,
	decodedTextSpanIntersectsWith,
	textSpanIntersectsWithTextSpan,
	textSpanIntersectsWith,
	textSpanIntersectsWithPosition,
	createTextChangeRange,
	textChangeRangeNewSpan,
	textChangeRangeIsUnchanged,
	displayPartsToString,
};
