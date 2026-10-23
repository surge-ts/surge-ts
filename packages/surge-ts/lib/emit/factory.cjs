"use strict";

// The nodes declaration emit makes, TypeScript-shaped: a created node has no
// position (pos and end -1, as tsc's factory makes them), an updated one
// takes its original's text range and points at it as `original`, and an
// unchanged one is the original itself. Which of the three a node is decides
// the source map, so these follow tsc's factory (nodeFactory.ts) closely.
const { SyntaxKind: K, NodeFlags, ModifierFlags } = require("../enums.cjs");
const native = require("../native.cjs");

const schema = native.nodeSchema();

/** tsc's `EmitFlags`. */
const EmitFlags = {
	None: 0,
	SingleLine: 1,
	MultiLine: 2,
	NoLeadingSourceMap: 32,
	NoTrailingSourceMap: 64,
	NoSourceMap: 96,
	NoNestedSourceMaps: 128,
	NoTokenLeadingSourceMaps: 256,
	NoTokenTrailingSourceMaps: 512,
	NoLeadingComments: 1024,
	NoTrailingComments: 2048,
	NoComments: 3072,
	NoNestedComments: 4096,
	Indented: 131072,
	NoIndentation: 262144,
	NoAsciiEscaping: 16777216,
};

const SYNTHESIZED = NodeFlags.Synthesized;

// --- emit nodes and origins ---------------------------------------------------

function isParseTreeNode(node) {
	return (node.flags & SYNTHESIZED) === 0;
}

function getOriginalNode(node) {
	if (node) while (node.original !== undefined) node = node.original;
	return node;
}

function getParseTreeNode(node) {
	if (node === undefined || isParseTreeNode(node)) return node;
	for (node = node.original; node; node = node.original) {
		if (isParseTreeNode(node)) return node;
	}
	return undefined;
}

function getOrCreateEmitNode(node) {
	return (node.emitNode ??= {});
}

function mergeEmitNode(source, dest = {}) {
	if (source.flags) dest.flags = source.flags;
	if (source.commentRange) dest.commentRange = source.commentRange;
	if (source.sourceMapRange) dest.sourceMapRange = source.sourceMapRange;
	if (source.tokenSourceMapRanges) dest.tokenSourceMapRanges = { ...source.tokenSourceMapRanges, ...dest.tokenSourceMapRanges };
	if (source.startsOnNewLine !== undefined) dest.startsOnNewLine = source.startsOnNewLine;
	return dest;
}

function setOriginalNode(node, original) {
	if (node.original !== original) {
		node.original = original;
		if (original && original.emitNode) node.emitNode = mergeEmitNode(original.emitNode, node.emitNode);
	}
	return node;
}

function setTextRange(node, range) {
	if (range) {
		node.pos = range.pos;
		node.end = range.end;
	}
	return node;
}

const getEmitFlags = (node) => node.emitNode?.flags ?? 0;
function setEmitFlags(node, flags) {
	getOrCreateEmitNode(node).flags = flags;
	return node;
}
const getCommentRange = (node) => node.emitNode?.commentRange ?? node;
function setCommentRange(node, range) {
	getOrCreateEmitNode(node).commentRange = range;
	return node;
}
const getSourceMapRange = (node) => node.emitNode?.sourceMapRange ?? node;
const getStartsOnNewLine = (node) => node.emitNode?.startsOnNewLine;

/** tsc's `nodeIsSynthesized`. */
const nodeIsSynthesized = (node) => node.pos < 0;

// --- node arrays --------------------------------------------------------------

const isNodeArray = (array) => Array.isArray(array) && array.pos !== undefined;

function createNodeArray(elements, hasTrailingComma) {
	if (elements === undefined) elements = [];
	else if (isNodeArray(elements)) {
		if (hasTrailingComma === undefined || elements.hasTrailingComma === hasTrailingComma) return elements;
		const array = elements.slice();
		array.pos = elements.pos;
		array.end = elements.end;
		array.hasTrailingComma = hasTrailingComma;
		return array;
	}
	const array = elements.slice();
	array.pos = -1;
	array.end = -1;
	array.hasTrailingComma = !!hasTrailingComma;
	return array;
}

// --- nodes --------------------------------------------------------------------

function createNode(kind, properties, flags = 0) {
	return { kind, flags: SYNTHESIZED | flags, pos: -1, end: -1, parent: undefined, original: undefined, emitNode: undefined, ...properties };
}

/** tsc's `update`: the new node stands where the original did. */
function update(updated, original) {
	if (updated !== original) {
		setOriginalNode(updated, original);
		setTextRange(updated, original);
	}
	return updated;
}

/**
 * `node` with `changes` (named children and scalars) applied, or `node`
 * itself when nothing changes, as each of tsc's `updateX` does.
 */
function updateNode(node, changes) {
	let changed = false;
	for (const [name, value] of Object.entries(changes)) {
		if (node[name] !== value) {
			changed = true;
			break;
		}
	}
	if (!changed) return node;
	const properties = {};
	for (const [name] of schema[node.kind] ?? []) properties[name] = node[name];
	for (const name of SCALAR_PROPERTIES[node.kind] ?? []) properties[name] = node[name];
	Object.assign(properties, changes);
	// surge's parsed import clauses say `isTypeOnly`, tsc 5.9's say `phaseModifier`.
	if (node.kind === K.ImportClause && properties.phaseModifier === undefined && node.isTypeOnly) properties.phaseModifier = K.TypeKeyword;
	return update(createNode(node.kind, properties, node.flags & ~SYNTHESIZED & KEPT_FLAGS), node);
}

/** tsc's `cloneNode`: a positionless copy of `node` that points at it as its original. */
function cloneNode(node) {
	const properties = {};
	for (const [name] of schema[node.kind] ?? []) properties[name] = node[name];
	for (const name of SCALAR_PROPERTIES[node.kind] ?? []) properties[name] = node[name];
	return setOriginalNode(createNode(node.kind, properties, node.flags & ~SYNTHESIZED), node);
}

// The node flags a node made from another keeps: its declaration list's
// `let`/`const`, a module's `namespace`/`global`.
const KEPT_FLAGS = NodeFlags.Let | NodeFlags.Const | NodeFlags.Using | NodeFlags.Namespace | NodeFlags.GlobalAugmentation | NodeFlags.NestedNamespace;

const SCALAR_PROPERTIES = {
	[K.Identifier]: ["escapedText"],
	[K.PrivateIdentifier]: ["escapedText"],
	[K.StringLiteral]: ["text", "singleQuote"],
	[K.NumericLiteral]: ["text"],
	[K.BigIntLiteral]: ["text"],
	[K.NoSubstitutionTemplateLiteral]: ["text", "rawText"],
	[K.TemplateHead]: ["text", "rawText"],
	[K.TemplateMiddle]: ["text", "rawText"],
	[K.TemplateTail]: ["text", "rawText"],
	[K.PrefixUnaryExpression]: ["operator"],
	[K.TypeOperator]: ["operator"],
	[K.HeritageClause]: ["token"],
	[K.ImportClause]: ["phaseModifier"],
	[K.ImportEqualsDeclaration]: ["isTypeOnly"],
	[K.ImportSpecifier]: ["isTypeOnly"],
	[K.ExportSpecifier]: ["isTypeOnly"],
	[K.ExportDeclaration]: ["isTypeOnly"],
	[K.ImportType]: ["isTypeOf"],
	[K.ExportAssignment]: ["isExportEquals"],
	[K.MetaProperty]: ["keywordToken"],
	[K.ImportAttributes]: ["token"],
};

// --- type parenthesizer rules (parenthesizerRules.ts) -------------------------

function parenthesizeCheckTypeOfConditionalType(type) {
	switch (type.kind) {
		case K.FunctionType:
		case K.ConstructorType:
		case K.ConditionalType:
			return createParenthesizedType(type);
	}
	return type;
}
function parenthesizeExtendsTypeOfConditionalType(type) {
	return type.kind === K.ConditionalType ? createParenthesizedType(type) : type;
}
function parenthesizeConstituentTypeOfUnionType(type) {
	switch (type.kind) {
		case K.UnionType:
		case K.IntersectionType:
			return createParenthesizedType(type);
	}
	return parenthesizeCheckTypeOfConditionalType(type);
}
function parenthesizeConstituentTypeOfIntersectionType(type) {
	switch (type.kind) {
		case K.UnionType:
		case K.IntersectionType:
			return createParenthesizedType(type);
	}
	return parenthesizeConstituentTypeOfUnionType(type);
}
function parenthesizeOperandOfTypeOperator(type) {
	return type.kind === K.IntersectionType ? createParenthesizedType(type) : parenthesizeConstituentTypeOfIntersectionType(type);
}
function parenthesizeOperandOfReadonlyTypeOperator(type) {
	return type.kind === K.TypeOperator ? createParenthesizedType(type) : parenthesizeOperandOfTypeOperator(type);
}
function parenthesizeNonArrayTypeOfPostfixType(type) {
	switch (type.kind) {
		case K.InferType:
		case K.TypeOperator:
		case K.TypeQuery:
			return createParenthesizedType(type);
	}
	return parenthesizeOperandOfTypeOperator(type);
}
function hasJSDocPostfixQuestion(type) {
	if (type.kind === K.JSDocNullableType) return type.postfix;
	if (type.kind === K.NamedTupleMember) return hasJSDocPostfixQuestion(type.type);
	if (type.kind === K.FunctionType || type.kind === K.ConstructorType || type.kind === K.TypeOperator) return hasJSDocPostfixQuestion(type.type);
	if (type.kind === K.ConditionalType) return hasJSDocPostfixQuestion(type.falseType);
	if (type.kind === K.UnionType || type.kind === K.IntersectionType) return hasJSDocPostfixQuestion(type.types[type.types.length - 1]);
	if (type.kind === K.InferType) return !!type.typeParameter.constraint && hasJSDocPostfixQuestion(type.typeParameter.constraint);
	return false;
}
function parenthesizeElementTypeOfTupleType(type) {
	return hasJSDocPostfixQuestion(type) ? createParenthesizedType(type) : type;
}
function parenthesizeTypeOfOptionalType(type) {
	return hasJSDocPostfixQuestion(type) ? createParenthesizedType(type) : parenthesizeNonArrayTypeOfPostfixType(type);
}
function parenthesizeLeadingTypeArgument(node) {
	return (node.kind === K.FunctionType || node.kind === K.ConstructorType) && node.typeParameters ? createParenthesizedType(node) : node;
}
function sameMap(array, map) {
	let result;
	for (let i = 0; i < array.length; i++) {
		const mapped = map(array[i], i);
		if (result) result.push(mapped);
		else if (mapped !== array[i]) {
			result = array.slice(0, i);
			result.push(mapped);
		}
	}
	return result ?? array;
}
function parenthesizeTypeArguments(typeArguments) {
	if (typeArguments && typeArguments.length) {
		return createNodeArray(sameMap(typeArguments, (node, i) => (i === 0 ? parenthesizeLeadingTypeArgument(node) : node)));
	}
	return undefined;
}

const parenthesizer = {
	parenthesizeCheckTypeOfConditionalType,
	parenthesizeExtendsTypeOfConditionalType,
	parenthesizeConstituentTypeOfUnionType,
	parenthesizeConstituentTypeOfIntersectionType,
	parenthesizeOperandOfTypeOperator,
	parenthesizeOperandOfReadonlyTypeOperator,
	parenthesizeNonArrayTypeOfPostfixType,
	parenthesizeElementTypeOfTupleType,
	parenthesizeTypeOfOptionalType,
	parenthesizeLeadingTypeArgument,
};

// --- creating ----------------------------------------------------------------

const asNodeArray = (array) => (array ? createNodeArray(array) : undefined);
const asName = (name) => (typeof name === "string" ? createIdentifier(name) : name);

function escapeLeadingUnderscores(text) {
	return text.length >= 2 && text.charCodeAt(0) === 95 && text.charCodeAt(1) === 95 ? `_${text}` : text;
}

function createIdentifier(text) {
	return createNode(K.Identifier, { escapedText: escapeLeadingUnderscores(text) });
}
function createPrivateIdentifier(text) {
	return createNode(K.PrivateIdentifier, { escapedText: escapeLeadingUnderscores(text) });
}
function createToken(kind) {
	return createNode(kind, {});
}
const createModifier = createToken;
const createKeywordTypeNode = createToken;
function createModifiersFromModifierFlags(flags) {
	const result = [];
	if (flags & ModifierFlags.Export) result.push(createModifier(K.ExportKeyword));
	if (flags & ModifierFlags.Ambient) result.push(createModifier(K.DeclareKeyword));
	if (flags & ModifierFlags.Default) result.push(createModifier(K.DefaultKeyword));
	if (flags & ModifierFlags.Const) result.push(createModifier(K.ConstKeyword));
	if (flags & ModifierFlags.Public) result.push(createModifier(K.PublicKeyword));
	if (flags & ModifierFlags.Private) result.push(createModifier(K.PrivateKeyword));
	if (flags & ModifierFlags.Protected) result.push(createModifier(K.ProtectedKeyword));
	if (flags & ModifierFlags.Abstract) result.push(createModifier(K.AbstractKeyword));
	if (flags & ModifierFlags.Static) result.push(createModifier(K.StaticKeyword));
	if (flags & ModifierFlags.Override) result.push(createModifier(K.OverrideKeyword));
	if (flags & ModifierFlags.Readonly) result.push(createModifier(K.ReadonlyKeyword));
	if (flags & ModifierFlags.Accessor) result.push(createModifier(K.AccessorKeyword));
	if (flags & ModifierFlags.Async) result.push(createModifier(K.AsyncKeyword));
	if (flags & ModifierFlags.In) result.push(createModifier(K.InKeyword));
	if (flags & ModifierFlags.Out) result.push(createModifier(K.OutKeyword));
	return result.length ? result : undefined;
}
function createStringLiteral(text, isSingleQuote) {
	return createNode(K.StringLiteral, { text, singleQuote: isSingleQuote });
}
function createNumericLiteral(value) {
	return createNode(K.NumericLiteral, { text: typeof value === "number" ? `${value}` : value });
}
function createBigIntLiteral(value) {
	return createNode(K.BigIntLiteral, { text: typeof value === "string" ? value : `${value.negative ? "-" : ""}${value.base10Value}n` });
}
const createTrue = () => createToken(K.TrueKeyword);
const createFalse = () => createToken(K.FalseKeyword);
const createNull = () => createToken(K.NullKeyword);
function createPrefixUnaryExpression(operator, operand) {
	return createNode(K.PrefixUnaryExpression, { operator, operand });
}
function createQualifiedName(left, right) {
	return createNode(K.QualifiedName, { left, right: asName(right) });
}
function createPropertyAccessExpression(expression, name) {
	return createNode(K.PropertyAccessExpression, { expression, questionDotToken: undefined, name: asName(name) });
}
function createTypeReferenceNode(typeName, typeArguments) {
	return createNode(K.TypeReference, { typeName: asName(typeName), typeArguments: typeArguments && parenthesizeTypeArguments(createNodeArray(typeArguments)) });
}
function createArrayTypeNode(elementType) {
	return createNode(K.ArrayType, { elementType: parenthesizeNonArrayTypeOfPostfixType(elementType) });
}
function createTupleTypeNode(elements) {
	return createNode(K.TupleType, { elements: createNodeArray(sameMap(elements ?? [], parenthesizeElementTypeOfTupleType)) });
}
function createNamedTupleMember(dotDotDotToken, name, questionToken, type) {
	return createNode(K.NamedTupleMember, { dotDotDotToken, name, questionToken, type });
}
function createOptionalTypeNode(type) {
	return createNode(K.OptionalType, { type: parenthesizeTypeOfOptionalType(type) });
}
function createRestTypeNode(type) {
	return createNode(K.RestType, { type });
}
function createUnionTypeNode(types) {
	return createNode(K.UnionType, { types: createNodeArray(sameMap(types, parenthesizeConstituentTypeOfUnionType)) });
}
function createIntersectionTypeNode(types) {
	return createNode(K.IntersectionType, { types: createNodeArray(sameMap(types, parenthesizeConstituentTypeOfIntersectionType)) });
}
function createParenthesizedType(type) {
	return createNode(K.ParenthesizedType, { type });
}
function createTypeOperatorNode(operator, type) {
	const rule = operator === K.ReadonlyKeyword ? parenthesizeOperandOfReadonlyTypeOperator : parenthesizeOperandOfTypeOperator;
	return createNode(K.TypeOperator, { operator, type: rule(type) });
}
function createIndexedAccessTypeNode(objectType, indexType) {
	return createNode(K.IndexedAccessType, { objectType: parenthesizeNonArrayTypeOfPostfixType(objectType), indexType });
}
function createLiteralTypeNode(literal) {
	return createNode(K.LiteralType, { literal });
}
function createTypeQueryNode(exprName, typeArguments) {
	return createNode(K.TypeQuery, { exprName, typeArguments: typeArguments && parenthesizeTypeArguments(typeArguments) });
}
function createThisTypeNode() {
	return createToken(K.ThisType);
}
function createImportTypeNode(argument, attributes, qualifier, typeArguments, isTypeOf = false) {
	return createNode(K.ImportType, { argument, attributes, qualifier, typeArguments: typeArguments && parenthesizeTypeArguments(typeArguments), isTypeOf });
}
function createTypeLiteralNode(members) {
	return createNode(K.TypeLiteral, { members: createNodeArray(members) });
}
function createPropertySignature(modifiers, name, questionToken, type) {
	return createNode(K.PropertySignature, { modifiers: asNodeArray(modifiers), name: asName(name), questionToken, type, initializer: undefined });
}
function createMethodSignature(modifiers, name, questionToken, typeParameters, parameters, type) {
	return createNode(K.MethodSignature, {
		modifiers: asNodeArray(modifiers),
		name: asName(name),
		questionToken,
		typeParameters: asNodeArray(typeParameters),
		parameters: createNodeArray(parameters),
		type,
	});
}
function createCallSignature(typeParameters, parameters, type) {
	return createNode(K.CallSignature, { typeParameters: asNodeArray(typeParameters), parameters: createNodeArray(parameters), type });
}
function createConstructSignature(typeParameters, parameters, type) {
	return createNode(K.ConstructSignature, { typeParameters: asNodeArray(typeParameters), parameters: createNodeArray(parameters), type });
}
function createIndexSignature(modifiers, parameters, type) {
	return createNode(K.IndexSignature, { modifiers: asNodeArray(modifiers), parameters: createNodeArray(parameters), type });
}
function createFunctionTypeNode(typeParameters, parameters, type) {
	return createNode(K.FunctionType, { typeParameters: asNodeArray(typeParameters), parameters: createNodeArray(parameters), type });
}
function createConstructorTypeNode(modifiers, typeParameters, parameters, type) {
	return createNode(K.ConstructorType, { modifiers: asNodeArray(modifiers), typeParameters: asNodeArray(typeParameters), parameters: createNodeArray(parameters), type });
}
function createTypeParameterDeclaration(modifiers, name, constraint, defaultType) {
	return createNode(K.TypeParameter, { modifiers: asNodeArray(modifiers), name: asName(name), constraint, default: defaultType, expression: undefined });
}
function createTypePredicateNode(assertsModifier, parameterName, type) {
	return createNode(K.TypePredicate, { assertsModifier, parameterName: asName(parameterName), type });
}
function createParameterDeclaration(modifiers, dotDotDotToken, name, questionToken, type, initializer) {
	return createNode(K.Parameter, { modifiers: asNodeArray(modifiers), dotDotDotToken, name: asName(name), questionToken, type, initializer });
}
function createPropertyDeclaration(modifiers, name, questionOrExclamationToken, type, initializer) {
	return createNode(K.PropertyDeclaration, {
		modifiers: asNodeArray(modifiers),
		name: asName(name),
		questionToken: questionOrExclamationToken && questionOrExclamationToken.kind === K.QuestionToken ? questionOrExclamationToken : undefined,
		exclamationToken: questionOrExclamationToken && questionOrExclamationToken.kind === K.ExclamationToken ? questionOrExclamationToken : undefined,
		type,
		initializer,
	});
}
function createMethodDeclaration(modifiers, asteriskToken, name, questionToken, typeParameters, parameters, type, body) {
	return createNode(K.MethodDeclaration, {
		modifiers: asNodeArray(modifiers),
		asteriskToken,
		name: asName(name),
		questionToken,
		exclamationToken: undefined,
		typeParameters: asNodeArray(typeParameters),
		parameters: createNodeArray(parameters),
		type,
		body,
	});
}
function createConstructorDeclaration(modifiers, parameters, body) {
	return createNode(K.Constructor, { modifiers: asNodeArray(modifiers), typeParameters: undefined, parameters: createNodeArray(parameters), type: undefined, body });
}
function createVariableDeclaration(name, exclamationToken, type, initializer) {
	return createNode(K.VariableDeclaration, { name: asName(name), exclamationToken, type, initializer });
}
function createVariableDeclarationList(declarations, flags = NodeFlags.None) {
	return createNode(K.VariableDeclarationList, { declarations: createNodeArray(declarations) }, flags & NodeFlags.BlockScoped);
}
function createVariableStatement(modifiers, declarationList) {
	return createNode(K.VariableStatement, {
		modifiers: asNodeArray(modifiers),
		declarationList: Array.isArray(declarationList) ? createVariableDeclarationList(declarationList) : declarationList,
	});
}
function createModuleDeclaration(modifiers, name, body, flags = NodeFlags.None) {
	return createNode(K.ModuleDeclaration, { modifiers: asNodeArray(modifiers), name, body }, flags & (NodeFlags.Namespace | NodeFlags.NestedNamespace | NodeFlags.GlobalAugmentation));
}
function createModuleBlock(statements) {
	return createNode(K.ModuleBlock, { statements: createNodeArray(statements) });
}
function createExportDeclaration(modifiers, isTypeOnly, exportClause, moduleSpecifier, attributes) {
	return createNode(K.ExportDeclaration, { modifiers: asNodeArray(modifiers), isTypeOnly, exportClause, moduleSpecifier, attributes });
}
function createNamedExports(elements) {
	return createNode(K.NamedExports, { elements: createNodeArray(elements) });
}
function createExportSpecifier(isTypeOnly, propertyName, name) {
	return createNode(K.ExportSpecifier, { isTypeOnly, propertyName: asName(propertyName), name: asName(name) });
}
function createExportAssignment(modifiers, isExportEquals, expression) {
	return createNode(K.ExportAssignment, { modifiers: asNodeArray(modifiers), isExportEquals, expression });
}
function createEmptyExports() {
	return createExportDeclaration(undefined, false, createNamedExports([]), undefined);
}

// --- updating ----------------------------------------------------------------

/** tsc's `updateSourceFile`: a declaration file standing for `node`. */
function updateSourceFile(node, statements, isDeclarationFile, referencedFiles, typeReferenceDirectives, hasNoDefaultLib, libReferenceDirectives) {
	const updated = {
		kind: K.SourceFile,
		flags: SYNTHESIZED | (node.flags & ~SYNTHESIZED),
		pos: node.pos,
		end: node.end,
		parent: undefined,
		original: undefined,
		emitNode: undefined,
		statements: createNodeArray(statements),
		endOfFileToken: node.endOfFileToken,
		fileName: node.fileName,
		text: node.text,
		languageVersion: node.languageVersion,
		isDeclarationFile,
		referencedFiles,
		typeReferenceDirectives,
		libReferenceDirectives,
		hasNoDefaultLib,
	};
	setOriginalNode(updated, node);
	return updated;
}

module.exports = {
	EmitFlags,
	isParseTreeNode,
	getOriginalNode,
	getParseTreeNode,
	getOrCreateEmitNode,
	setOriginalNode,
	setTextRange,
	getEmitFlags,
	setEmitFlags,
	getCommentRange,
	setCommentRange,
	getSourceMapRange,
	getStartsOnNewLine,
	nodeIsSynthesized,
	isNodeArray,
	createNodeArray,
	createNode,
	update,
	updateNode,
	cloneNode,
	parenthesizer,
	sameMap,
	createIdentifier,
	createPrivateIdentifier,
	createToken,
	createModifier,
	createKeywordTypeNode,
	createModifiersFromModifierFlags,
	createStringLiteral,
	createNumericLiteral,
	createBigIntLiteral,
	createTrue,
	createFalse,
	createNull,
	createPrefixUnaryExpression,
	createQualifiedName,
	createPropertyAccessExpression,
	createTypeReferenceNode,
	createArrayTypeNode,
	createTupleTypeNode,
	createNamedTupleMember,
	createOptionalTypeNode,
	createRestTypeNode,
	createUnionTypeNode,
	createIntersectionTypeNode,
	createParenthesizedType,
	createTypeOperatorNode,
	createIndexedAccessTypeNode,
	createLiteralTypeNode,
	createTypeQueryNode,
	createThisTypeNode,
	createImportTypeNode,
	createTypeLiteralNode,
	createPropertySignature,
	createMethodSignature,
	createCallSignature,
	createConstructSignature,
	createIndexSignature,
	createFunctionTypeNode,
	createConstructorTypeNode,
	createTypeParameterDeclaration,
	createTypePredicateNode,
	createParameterDeclaration,
	createPropertyDeclaration,
	createMethodDeclaration,
	createConstructorDeclaration,
	createVariableDeclaration,
	createVariableDeclarationList,
	createVariableStatement,
	createModuleDeclaration,
	createModuleBlock,
	createExportDeclaration,
	createNamedExports,
	createExportSpecifier,
	createExportAssignment,
	createEmptyExports,
	updateSourceFile,
};
