"use strict";

// The `TransformationContext` a declaration file's `afterDeclarations`
// transformers run with. Its `factory` takes tsc's `NodeFactory` arguments, so
// typescript's own `visitEachChild` can walk and update surge's declaration
// nodes: an `updateX` returns the node itself when nothing it is given
// differs, else a new node standing where the old one did.
const { SyntaxKind: K } = require("../enums.cjs");
const f = require("./factory.cjs");

const unsupported = (what) => new Error(`surge-ts: ${what} in an afterDeclarations transformer is not supported yet`);

// tsc's `updateX(node, ...)` parameters, as the properties they set.
const UPDATES = {
	updateQualifiedName: ["left", "right"],
	updateComputedPropertyName: ["expression"],
	updateTypeParameterDeclaration: ["modifiers", "name", "constraint", "default"],
	updateParameterDeclaration: ["modifiers", "dotDotDotToken", "name", "questionToken", "type", "initializer"],
	updateDecorator: ["expression"],
	updatePropertySignature: ["modifiers", "name", "questionToken", "type"],
	updateMethodSignature: ["modifiers", "name", "questionToken", "typeParameters", "parameters", "type"],
	updateMethodDeclaration: ["modifiers", "asteriskToken", "name", "questionToken", "typeParameters", "parameters", "type", "body"],
	updateConstructorDeclaration: ["modifiers", "parameters", "body"],
	updateGetAccessorDeclaration: ["modifiers", "name", "parameters", "type", "body"],
	updateSetAccessorDeclaration: ["modifiers", "name", "parameters", "body"],
	updateCallSignature: ["typeParameters", "parameters", "type"],
	updateConstructSignature: ["typeParameters", "parameters", "type"],
	updateIndexSignature: ["modifiers", "parameters", "type"],
	updateTemplateLiteralTypeSpan: ["type", "literal"],
	updateTypePredicateNode: ["assertsModifier", "parameterName", "type"],
	updateTypeReferenceNode: ["typeName", "typeArguments"],
	updateFunctionTypeNode: ["typeParameters", "parameters", "type"],
	updateTypeQueryNode: ["exprName", "typeArguments"],
	updateTypeLiteralNode: ["members"],
	updateArrayTypeNode: ["elementType"],
	updateTupleTypeNode: ["elements"],
	updateNamedTupleMember: ["dotDotDotToken", "name", "questionToken", "type"],
	updateOptionalTypeNode: ["type"],
	updateRestTypeNode: ["type"],
	updateUnionTypeNode: ["types"],
	updateIntersectionTypeNode: ["types"],
	updateConditionalTypeNode: ["checkType", "extendsType", "trueType", "falseType"],
	updateInferTypeNode: ["typeParameter"],
	updateTemplateLiteralType: ["head", "templateSpans"],
	updateImportTypeNode: ["argument", "attributes", "qualifier", "typeArguments", "isTypeOf"],
	updateParenthesizedType: ["type"],
	updateTypeOperatorNode: ["type"],
	updateIndexedAccessTypeNode: ["objectType", "indexType"],
	updateMappedTypeNode: ["readonlyToken", "typeParameter", "nameType", "questionToken", "type", "members"],
	updateLiteralTypeNode: ["literal"],
	updateObjectBindingPattern: ["elements"],
	updateArrayBindingPattern: ["elements"],
	updateBindingElement: ["dotDotDotToken", "propertyName", "name", "initializer"],
	updateArrayLiteralExpression: ["elements"],
	updateObjectLiteralExpression: ["properties"],
	updatePropertyAccessExpression: ["expression", "name"],
	updateElementAccessExpression: ["expression", "argumentExpression"],
	updateCallExpression: ["expression", "typeArguments", "arguments"],
	updateNewExpression: ["expression", "typeArguments", "arguments"],
	updateParenthesizedExpression: ["expression"],
	updatePrefixUnaryExpression: ["operand"],
	updateExpressionWithTypeArguments: ["expression", "typeArguments"],
	updateHeritageClause: ["types"],
	updateVariableStatement: ["modifiers", "declarationList"],
	updateVariableDeclaration: ["name", "exclamationToken", "type", "initializer"],
	updateVariableDeclarationList: ["declarations"],
	updateFunctionDeclaration: ["modifiers", "asteriskToken", "name", "typeParameters", "parameters", "type", "body"],
	updateClassDeclaration: ["modifiers", "name", "typeParameters", "heritageClauses", "members"],
	updateInterfaceDeclaration: ["modifiers", "name", "typeParameters", "heritageClauses", "members"],
	updateTypeAliasDeclaration: ["modifiers", "name", "typeParameters", "type"],
	updateEnumDeclaration: ["modifiers", "name", "members"],
	updateEnumMember: ["name", "initializer"],
	updateModuleDeclaration: ["modifiers", "name", "body"],
	updateModuleBlock: ["statements"],
	updateNamespaceExportDeclaration: ["name"],
	updateImportEqualsDeclaration: ["modifiers", "isTypeOnly", "name", "moduleReference"],
	updateImportDeclaration: ["modifiers", "importClause", "moduleSpecifier", "attributes"],
	updateImportClause: ["phaseModifier", "name", "namedBindings"],
	updateImportAttributes: ["elements", "multiLine"],
	updateImportAttribute: ["name", "value"],
	updateNamespaceImport: ["name"],
	updateNamespaceExport: ["name"],
	updateNamedImports: ["elements"],
	updateImportSpecifier: ["isTypeOnly", "propertyName", "name"],
	updateExportAssignment: ["modifiers", "expression"],
	updateExportDeclaration: ["modifiers", "isTypeOnly", "exportClause", "moduleSpecifier", "attributes"],
	updateNamedExports: ["elements"],
	updateExportSpecifier: ["isTypeOnly", "propertyName", "name"],
	updateExternalModuleReference: ["expression"],
};

/** tsc's `update` for one of `UPDATES`: `node` unless an argument differs. */
function updateWith(node, names, args) {
	let changed = false;
	for (let i = 0; i < names.length; i++) {
		if (node[names[i]] !== args[i]) {
			changed = true;
			break;
		}
	}
	if (!changed) return node;
	const changes = {};
	for (let i = 0; i < names.length; i++) {
		const value = args[i];
		changes[names[i]] = Array.isArray(value) ? f.createNodeArray(value) : value;
	}
	return f.updateNode(node, changes);
}

function createNodeFactory() {
	const factory = {
		createNodeArray: (elements, hasTrailingComma) => f.createNodeArray(elements, hasTrailingComma),
		createIdentifier: (text) => f.createIdentifier(text),
		createPrivateIdentifier: (text) => f.createPrivateIdentifier(text),
		createStringLiteral: (text, isSingleQuote) => f.createStringLiteral(text, isSingleQuote),
		createNumericLiteral: (value) => f.createNumericLiteral(value),
		createBigIntLiteral: (value) => f.createBigIntLiteral(value),
		createToken: (kind) => f.createToken(kind),
		createModifier: (kind) => f.createModifier(kind),
		createModifiersFromModifierFlags: (flags) => f.createModifiersFromModifierFlags(flags),
		createKeywordTypeNode: (kind) => f.createKeywordTypeNode(kind),
		createTrue: f.createTrue,
		createFalse: f.createFalse,
		createNull: f.createNull,
		createQualifiedName: (left, right) => f.createQualifiedName(left, right),
		createTypeReferenceNode: (typeName, typeArguments) => f.createTypeReferenceNode(typeName, typeArguments),
		createLiteralTypeNode: (literal) => f.createLiteralTypeNode(literal),
		createArrayTypeNode: (elementType) => f.createArrayTypeNode(elementType),
		createUnionTypeNode: (types) => f.createUnionTypeNode(types),
		createIntersectionTypeNode: (types) => f.createIntersectionTypeNode(types),
		createParenthesizedType: (type) => f.createParenthesizedType(type),
		createTypeQueryNode: (exprName, typeArguments) => f.createTypeQueryNode(exprName, typeArguments),
		createImportTypeNode: (argument, attributes, qualifier, typeArguments, isTypeOf) =>
			f.createImportTypeNode(argument, attributes, qualifier, typeArguments, isTypeOf),
		createTypeLiteralNode: (members) => f.createTypeLiteralNode(members),
		createPropertySignature: (modifiers, name, questionToken, type) => f.createPropertySignature(modifiers, name, questionToken, type),
		createExportDeclaration: (modifiers, isTypeOnly, exportClause, moduleSpecifier, attributes) =>
			f.createExportDeclaration(modifiers, isTypeOnly, exportClause, moduleSpecifier, attributes),
		createNamedExports: (elements) => f.createNamedExports(elements),
		createExportSpecifier: (isTypeOnly, propertyName, name) => f.createExportSpecifier(isTypeOnly, propertyName, name),
		updateSourceFile(node, statements, isDeclarationFile = node.isDeclarationFile, referencedFiles = node.referencedFiles, typeReferenceDirectives = node.typeReferenceDirectives, hasNoDefaultLib = node.hasNoDefaultLib, libReferenceDirectives = node.libReferenceDirectives) {
			if (
				node.statements === statements &&
				node.isDeclarationFile === isDeclarationFile &&
				node.referencedFiles === referencedFiles &&
				node.typeReferenceDirectives === typeReferenceDirectives &&
				node.hasNoDefaultLib === hasNoDefaultLib &&
				node.libReferenceDirectives === libReferenceDirectives
			) {
				return node;
			}
			return f.updateSourceFile(node, statements, isDeclarationFile, referencedFiles, typeReferenceDirectives, hasNoDefaultLib, libReferenceDirectives);
		},
		updatePropertyDeclaration(node, modifiers, name, questionOrExclamationToken, type, initializer) {
			const questionToken = questionOrExclamationToken?.kind === K.QuestionToken ? questionOrExclamationToken : undefined;
			const exclamationToken = questionOrExclamationToken?.kind === K.ExclamationToken ? questionOrExclamationToken : undefined;
			return updateWith(node, ["modifiers", "name", "questionToken", "exclamationToken", "type", "initializer"], [modifiers, name, questionToken, exclamationToken, type, initializer]);
		},
		updateConstructorTypeNode(node, ...args) {
			const [modifiers, typeParameters, parameters, type] = args.length === 4 ? args : [node.modifiers, ...args];
			return updateWith(node, ["modifiers", "typeParameters", "parameters", "type"], [modifiers, typeParameters, parameters, type]);
		},
		updateImportTypeNode(node, argument, attributes, qualifier, typeArguments, isTypeOf = node.isTypeOf) {
			return updateWith(node, UPDATES.updateImportTypeNode, [argument, attributes, qualifier, typeArguments, isTypeOf]);
		},
		mergeLexicalEnvironment: (statements, declarations) => {
			if (declarations && declarations.length) throw unsupported("a hoisted declaration");
			return statements;
		},
	};
	for (const [method, names] of Object.entries(UPDATES)) {
		if (!factory[method]) factory[method] = (node, ...args) => updateWith(node, names, args);
	}
	return new Proxy(factory, {
		get(target, name) {
			if (name in target || typeof name !== "string") return target[name];
			return () => {
				throw unsupported(`factory.${name}`);
			};
		},
	});
}

let nodeFactory;

/** The context `afterDeclarations` transformers are created with. */
function createTransformationContext(program, diagnostics) {
	nodeFactory ??= createNodeFactory();
	let lexicalEnvironmentFlags = 0;
	const fail = (what) => () => {
		throw unsupported(what);
	};
	return {
		factory: nodeFactory,
		getCompilerOptions: () => program.getCompilerOptions(),
		getEmitResolver: fail("context.getEmitResolver"),
		getEmitHost: fail("context.getEmitHost"),
		getEmitHelperFactory: fail("context.getEmitHelperFactory"),
		startLexicalEnvironment() {},
		suspendLexicalEnvironment() {},
		resumeLexicalEnvironment() {},
		endLexicalEnvironment: () => undefined,
		setLexicalEnvironmentFlags(flags, value) {
			lexicalEnvironmentFlags = value ? lexicalEnvironmentFlags | flags : lexicalEnvironmentFlags & ~flags;
		},
		getLexicalEnvironmentFlags: () => lexicalEnvironmentFlags,
		hoistVariableDeclaration: fail("context.hoistVariableDeclaration"),
		hoistFunctionDeclaration: fail("context.hoistFunctionDeclaration"),
		addInitializationStatement: fail("context.addInitializationStatement"),
		startBlockScope() {},
		endBlockScope: () => undefined,
		addBlockScopedVariable: fail("context.addBlockScopedVariable"),
		requestEmitHelper: fail("context.requestEmitHelper"),
		readEmitHelpers: () => undefined,
		enableSubstitution() {},
		isSubstitutionEnabled: () => false,
		onSubstituteNode: (_hint, node) => node,
		enableEmitNotification() {},
		isEmitNotificationEnabled: () => false,
		onEmitNode: (hint, node, emitCallback) => emitCallback(hint, node),
		addDiagnostic(diagnostic) {
			diagnostics.push(diagnostic);
		},
	};
}

module.exports = { createTransformationContext };
