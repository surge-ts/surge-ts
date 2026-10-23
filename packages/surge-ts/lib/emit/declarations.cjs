"use strict";

// tsc's declaration transform (transformers/declarations.ts) for one
// TypeScript source file: what each statement and member becomes in the
// declaration file, which imports the result still needs (resolved late, once
// the declarations that reference them are known), and where it needs an
// `export {}`. The checker's answers come from `resolver` (resolver.cjs).
const { SyntaxKind: K, ModifierFlags, NodeFlags } = require("../enums.cjs");
const f = require("./factory.cjs");
const { visitNode, visitNodes, visitArray, visitEachChild } = require("./visitor.cjs");
const messages = require("../messages.cjs").diagnostics();
const { createFileDiagnostic } = require("../messages.cjs");

/** tsc's `createDiagnosticForNode` for a name: its span after leading trivia. */
function diagnosticForNode(node, message, ...args) {
	const sourceFile = f.getParseTreeNode(node).getSourceFile();
	const start = node.getStart(sourceFile);
	return createFileDiagnostic(sourceFile, start, node.end - start, message, ...args);
}

const isKind = (kind) => (node) => !!node && node.kind === kind;
const isIdentifier = isKind(K.Identifier);
const isPrivateIdentifier = isKind(K.PrivateIdentifier);
const isSourceFile = isKind(K.SourceFile);
const isComputedPropertyName = isKind(K.ComputedPropertyName);
const isOmittedExpression = isKind(K.OmittedExpression);
const isParameter = isKind(K.Parameter);
const isBindingElement = isKind(K.BindingElement);
const isExportAssignment = isKind(K.ExportAssignment);
const isTupleTypeNode = isKind(K.TupleType);
const isTypeQueryNode = isKind(K.TypeQuery);
const isBindingPattern = (node) => !!node && (node.kind === K.ObjectBindingPattern || node.kind === K.ArrayBindingPattern);
const isModifier = (node) => !!node && MODIFIER_KINDS.has(node.kind);

const MODIFIER_KINDS = new Set([
	K.AbstractKeyword,
	K.AccessorKeyword,
	K.AsyncKeyword,
	K.ConstKeyword,
	K.DeclareKeyword,
	K.DefaultKeyword,
	K.ExportKeyword,
	K.InKeyword,
	K.PublicKeyword,
	K.PrivateKeyword,
	K.ProtectedKeyword,
	K.ReadonlyKeyword,
	K.StaticKeyword,
	K.OutKeyword,
	K.OverrideKeyword,
]);

const MODIFIER_FLAG_OF_KIND = new Map([
	[K.ExportKeyword, ModifierFlags.Export],
	[K.DeclareKeyword, ModifierFlags.Ambient],
	[K.DefaultKeyword, ModifierFlags.Default],
	[K.ConstKeyword, ModifierFlags.Const],
	[K.PublicKeyword, ModifierFlags.Public],
	[K.PrivateKeyword, ModifierFlags.Private],
	[K.ProtectedKeyword, ModifierFlags.Protected],
	[K.AbstractKeyword, ModifierFlags.Abstract],
	[K.StaticKeyword, ModifierFlags.Static],
	[K.OverrideKeyword, ModifierFlags.Override],
	[K.ReadonlyKeyword, ModifierFlags.Readonly],
	[K.AccessorKeyword, ModifierFlags.Accessor],
	[K.AsyncKeyword, ModifierFlags.Async],
	[K.InKeyword, ModifierFlags.In],
	[K.OutKeyword, ModifierFlags.Out],
	[K.Decorator, ModifierFlags.Decorator],
]);

/** tsc's `getEffectiveModifierFlags` for a TypeScript file. */
function modifierFlags(node) {
	if (node.kind >= K.FirstToken && node.kind <= K.LastToken) return ModifierFlags.None;
	let flags = ModifierFlags.None;
	for (const modifier of node.modifiers ?? []) flags |= MODIFIER_FLAG_OF_KIND.get(modifier.kind) ?? 0;
	if (node.flags & NodeFlags.NestedNamespace) flags |= ModifierFlags.Export;
	return flags;
}
const hasEffectiveModifier = (node, flag) => (modifierFlags(node) & flag) !== 0;

function maskModifierFlags(node, modifierMask = ModifierFlags.All ^ ModifierFlags.Public, modifierAdditions = ModifierFlags.None) {
	let flags = (modifierFlags(node) & modifierMask) | modifierAdditions;
	if (flags & ModifierFlags.Default && !(flags & ModifierFlags.Export)) flags ^= ModifierFlags.Export;
	if (flags & ModifierFlags.Default && flags & ModifierFlags.Ambient) flags ^= ModifierFlags.Ambient;
	return flags;
}
const maskModifiers = (node, mask, additions) => f.createModifiersFromModifierFlags(maskModifierFlags(node, mask, additions));

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
function isFunctionLike(node) {
	if (!node) return false;
	switch (node.kind) {
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
	return isFunctionLikeDeclarationKind(node.kind);
}

const DECLARATION_KINDS = new Set([
	K.ArrowFunction,
	K.BindingElement,
	K.ClassDeclaration,
	K.ClassExpression,
	K.ClassStaticBlockDeclaration,
	K.Constructor,
	K.EnumDeclaration,
	K.EnumMember,
	K.ExportSpecifier,
	K.FunctionDeclaration,
	K.FunctionExpression,
	K.GetAccessor,
	K.ImportClause,
	K.ImportEqualsDeclaration,
	K.ImportSpecifier,
	K.InterfaceDeclaration,
	K.JsxAttribute,
	K.MethodDeclaration,
	K.MethodSignature,
	K.ModuleDeclaration,
	K.NamespaceExportDeclaration,
	K.NamespaceImport,
	K.NamespaceExport,
	K.Parameter,
	K.PropertyAssignment,
	K.PropertyDeclaration,
	K.PropertySignature,
	K.SetAccessor,
	K.ShorthandPropertyAssignment,
	K.TypeAliasDeclaration,
	K.TypeParameter,
	K.VariableDeclaration,
	K.JSDocTypedefTag,
	K.JSDocCallbackTag,
	K.JSDocPropertyTag,
	K.NamedTupleMember,
]);
const isDeclaration = (node) => node.kind === K.TypeParameter || DECLARATION_KINDS.has(node.kind);

const isStringOrNumericLiteralLike = (node) =>
	node.kind === K.StringLiteral || node.kind === K.NoSubstitutionTemplateLiteral || node.kind === K.NumericLiteral;
const isSignedNumericLiteral = (node) =>
	node.kind === K.PrefixUnaryExpression && (node.operator === K.PlusToken || node.operator === K.MinusToken) && node.operand.kind === K.NumericLiteral;
function skipParentheses(node) {
	while (node.kind === K.ParenthesizedExpression) node = node.expression;
	return node;
}
function hasDynamicName(declaration) {
	const name = declaration.name;
	if (!name || !(name.kind === K.ComputedPropertyName || name.kind === K.ElementAccessExpression)) return false;
	const expression = name.kind === K.ElementAccessExpression ? skipParentheses(name.argumentExpression) : name.expression;
	return !isStringOrNumericLiteralLike(expression) && !isSignedNumericLiteral(expression);
}
function isEntityNameExpression(node) {
	return node.kind === K.Identifier || (node.kind === K.PropertyAccessExpression && isIdentifier(node.name) && isEntityNameExpression(node.expression));
}
const isEntityName = (node) => node.kind === K.Identifier || node.kind === K.QualifiedName;

function canProduceDiagnostics(node) {
	switch (node.kind) {
		case K.VariableDeclaration:
		case K.PropertyDeclaration:
		case K.PropertySignature:
		case K.BindingElement:
		case K.SetAccessor:
		case K.GetAccessor:
		case K.ConstructSignature:
		case K.CallSignature:
		case K.MethodDeclaration:
		case K.MethodSignature:
		case K.FunctionDeclaration:
		case K.Parameter:
		case K.TypeParameter:
		case K.ExpressionWithTypeArguments:
		case K.ImportEqualsDeclaration:
		case K.TypeAliasDeclaration:
		case K.Constructor:
		case K.IndexSignature:
		case K.PropertyAccessExpression:
		case K.ElementAccessExpression:
		case K.BinaryExpression:
			return true;
	}
	return false;
}

function hasInferredType(node) {
	switch (node.kind) {
		case K.Parameter:
		case K.PropertySignature:
		case K.PropertyDeclaration:
		case K.BindingElement:
		case K.PropertyAccessExpression:
		case K.ElementAccessExpression:
		case K.BinaryExpression:
		case K.VariableDeclaration:
		case K.ExportAssignment:
		case K.PropertyAssignment:
		case K.ShorthandPropertyAssignment:
			return true;
	}
	return false;
}

function isPrimitiveLiteralValue(node) {
	switch (node.kind) {
		case K.TrueKeyword:
		case K.FalseKeyword:
		case K.NumericLiteral:
		case K.StringLiteral:
		case K.NoSubstitutionTemplateLiteral:
		case K.BigIntLiteral:
			return true;
		case K.PrefixUnaryExpression:
			if (node.operator === K.MinusToken) return node.operand.kind === K.NumericLiteral || node.operand.kind === K.BigIntLiteral;
			if (node.operator === K.PlusToken) return node.operand.kind === K.NumericLiteral;
			return false;
	}
	return false;
}

function canHaveLiteralInitializer(node) {
	switch (node.kind) {
		case K.PropertyDeclaration:
		case K.PropertySignature:
			return !hasEffectiveModifier(node, ModifierFlags.Private);
		case K.Parameter:
		case K.VariableDeclaration:
			return true;
	}
	return false;
}

function isPreservedDeclarationStatement(node) {
	switch (node.kind) {
		case K.FunctionDeclaration:
		case K.ModuleDeclaration:
		case K.ImportEqualsDeclaration:
		case K.InterfaceDeclaration:
		case K.ClassDeclaration:
		case K.TypeAliasDeclaration:
		case K.EnumDeclaration:
		case K.VariableStatement:
		case K.ImportDeclaration:
		case K.ExportDeclaration:
		case K.ExportAssignment:
			return true;
	}
	return false;
}

function isProcessedComponent(node) {
	switch (node.kind) {
		case K.ConstructSignature:
		case K.Constructor:
		case K.MethodDeclaration:
		case K.GetAccessor:
		case K.SetAccessor:
		case K.PropertyDeclaration:
		case K.PropertySignature:
		case K.MethodSignature:
		case K.CallSignature:
		case K.IndexSignature:
		case K.VariableDeclaration:
		case K.TypeParameter:
		case K.ExpressionWithTypeArguments:
		case K.TypeReference:
		case K.ConditionalType:
		case K.FunctionType:
		case K.ConstructorType:
		case K.ImportType:
			return true;
	}
	return false;
}

function isLateVisibilityPaintedStatement(node) {
	switch (node.kind) {
		case K.ImportDeclaration:
		case K.ImportEqualsDeclaration:
		case K.VariableStatement:
		case K.ClassDeclaration:
		case K.FunctionDeclaration:
		case K.ModuleDeclaration:
		case K.TypeAliasDeclaration:
		case K.InterfaceDeclaration:
		case K.EnumDeclaration:
			return true;
	}
	return false;
}

const isGlobalScopeAugmentation = (module) => !!(module.flags & NodeFlags.GlobalAugmentation);
const isAmbientModule = (node) => node.kind === K.ModuleDeclaration && (node.name.kind === K.StringLiteral || isGlobalScopeAugmentation(node));
const isAnyImportOrReExport = (node) =>
	node.kind === K.ImportDeclaration || node.kind === K.ImportEqualsDeclaration || node.kind === K.ExportDeclaration;
const hasSyntacticExport = (node) => (modifierFlags(node) & ModifierFlags.Export) !== 0;
const needsScopeMarker = (result) => !isAnyImportOrReExport(result) && !isExportAssignment(result) && !hasSyntacticExport(result) && !isAmbientModule(result);
const isExternalModuleIndicator = (result) => isAnyImportOrReExport(result) || isExportAssignment(result) || hasSyntacticExport(result);
const isScopeMarker = (node) => isExportAssignment(node) || node.kind === K.ExportDeclaration;
const isAlwaysType = (node) => node.kind === K.InterfaceDeclaration;

function isEnclosingDeclaration(node) {
	switch (node.kind) {
		case K.SourceFile:
		case K.TypeAliasDeclaration:
		case K.ModuleDeclaration:
		case K.ClassDeclaration:
		case K.InterfaceDeclaration:
		case K.IndexSignature:
		case K.MappedType:
			return true;
	}
	return isFunctionLike(node);
}

/** The import clause's `type` (or `defer`) keyword, as tsc 5.9 records it. */
function phaseModifierOf(clause) {
	if (clause.phaseModifier !== undefined) return clause.phaseModifier;
	return clause.isTypeOnly ? K.TypeKeyword : undefined;
}

/** tsc's `filter`: `array` itself when every element passes. */
function filter(array, predicate) {
	if (!array) return array;
	let i = 0;
	while (i < array.length && predicate(array[i])) i++;
	if (i === array.length) return array;
	const result = array.slice(0, i);
	for (i++; i < array.length; i++) if (predicate(array[i])) result.push(array[i]);
	return result;
}

/** tsc's `concatenate`: one array when the other is empty. */
function concatenate(array1, array2) {
	if (!array2 || !array2.length) return array1;
	if (!array1 || !array1.length) return array2;
	return [...array1, ...array2];
}

const asNodeArray = (array) => (array === undefined ? undefined : f.createNodeArray(array));

/** tsc's `getResolutionModeOverride`: an import's `resolution-mode` attribute. */
function hasResolutionModeOverride(attributes) {
	return !!attributes && attributes.elements.some((element) => element.name && (element.name.text ?? element.name.escapedText) === "resolution-mode");
}

function parameterIsThisKeyword(parameter) {
	return isIdentifier(parameter.name) && parameter.name.escapedText === "this";
}

/**
 * The declaration file for `sourceFile`. `isExternalModule` is whether the
 * file is a module (tsc's `isExternalModule`).
 */
function transformDeclarations(sourceFile, { resolver, isExternalModule, stripInternal }) {
	const diagnostics = [];
	let needsDeclare = true;
	let resultHasExternalModuleIndicator = false;
	let needsScopeFixMarker = false;
	let resultHasScopeMarker = false;
	let enclosingDeclaration;
	let lateMarkedStatements;
	let lateStatementReplacementMap;
	let suppressNewDiagnosticContexts;
	let errorNameNode;
	const symbolTracker = { trackSymbol, reportPrivateInBaseOfClassExpression };

	function handleSymbolAccessibilityError(result) {
		if (result.accessibility === 0) {
			if (result.aliasesToMakeVisible) {
				if (!lateMarkedStatements) lateMarkedStatements = [...result.aliasesToMakeVisible];
				else for (const reference of result.aliasesToMakeVisible) if (!lateMarkedStatements.includes(reference)) lateMarkedStatements.push(reference);
			}
		} else if (result.accessibility !== 3) {
			throw new Error(`surge-ts: declaration emit naming the inaccessible ${result.errorSymbolName} (an error) is not supported yet`);
		}
		return false;
	}
	function trackSymbol(symbol, enclosing, meaning) {
		if (symbol.flags & 262144) return false;
		return handleSymbolAccessibilityError(resolver.isSymbolAccessible(symbol, enclosing, meaning, true));
	}
	function reportPrivateInBaseOfClassExpression(propertyName) {
		if (!errorNameNode) return;
		const diagnostic = diagnosticForNode(errorNameNode, messages.Property_0_of_exported_anonymous_class_type_may_not_be_private_or_protected, propertyName);
		if (errorNameNode.parent?.kind === K.VariableDeclaration) {
			diagnostic.relatedInformation = [diagnosticForNode(errorNameNode, messages.Add_a_type_annotation_to_the_variable_0, errorNameNode.text ?? errorNameNode.escapedText)];
		}
		diagnostics.push(diagnostic);
	}
	function checkEntityNameVisibility(entityName, enclosing) {
		handleSymbolAccessibilityError(resolver.isEntityNameVisible(entityName, enclosing));
	}

	return { transformed: transformRoot(sourceFile), diagnostics };

	function transformRoot(node) {
		needsDeclare = true;
		needsScopeFixMarker = false;
		resultHasScopeMarker = false;
		enclosingDeclaration = node;
		resultHasExternalModuleIndicator = false;
		suppressNewDiagnosticContexts = false;
		lateMarkedStatements = undefined;
		lateStatementReplacementMap = new Map();
		const statements = visitNodes(node.statements, visitDeclarationStatements);
		let combinedStatements = f.setTextRange(f.createNodeArray(transformAndReplaceLatePaintedStatements(statements)), node.statements);
		if (isExternalModule && (!resultHasExternalModuleIndicator || (needsScopeFixMarker && !resultHasScopeMarker))) {
			combinedStatements = f.setTextRange(f.createNodeArray([...combinedStatements, f.createEmptyExports()]), combinedStatements);
		}
		const preserved = (references) => (references ?? []).filter((reference) => reference.preserve).map((reference) => ({ ...reference, pos: -1, end: -1 }));
		return f.updateSourceFile(node, combinedStatements, true, [], preserved(node.typeReferenceDirectives), node.hasNoDefaultLib, preserved(node.libReferenceDirectives));
	}

	function filterBindingPatternInitializers(name) {
		if (name.kind === K.Identifier) return name;
		const visitBindingElement = (element) => {
			if (element.kind === K.OmittedExpression) return element;
			if (element.propertyName && isComputedPropertyName(element.propertyName) && isEntityNameExpression(element.propertyName.expression)) {
				checkEntityNameVisibility(element.propertyName.expression, enclosingDeclaration);
			}
			return f.updateNode(element, {
				dotDotDotToken: element.dotDotDotToken,
				propertyName: element.propertyName,
				name: filterBindingPatternInitializers(element.name),
				initializer: undefined,
			});
		};
		return f.updateNode(name, { elements: visitNodes(name.elements, visitBindingElement) });
	}

	function ensureParameter(parameter, modifierMask) {
		return f.updateNode(parameter, {
			modifiers: asNodeArray(maskModifiers(parameter, modifierMask)),
			dotDotDotToken: parameter.dotDotDotToken,
			name: filterBindingPatternInitializers(parameter.name),
			questionToken: resolver.isOptionalParameter(parameter) ? parameter.questionToken || f.createToken(K.QuestionToken) : undefined,
			type: ensureType(parameter, true),
			initializer: ensureNoInitializer(parameter),
		});
	}

	function shouldPrintWithInitializer(node) {
		return canHaveLiteralInitializer(node) && !!node.initializer && resolver.isLiteralConstDeclaration(f.getParseTreeNode(node));
	}

	function ensureNoInitializer(node) {
		if (shouldPrintWithInitializer(node)) return resolver.createLiteralConstValue(f.getParseTreeNode(node), symbolTracker);
		return undefined;
	}

	function ensureType(node, ignorePrivate) {
		if (!ignorePrivate && hasEffectiveModifier(node, ModifierFlags.Private)) return undefined;
		if (shouldPrintWithInitializer(node)) return undefined;
		if (!isExportAssignment(node) && !isBindingElement(node) && node.type && (!isParameter(node) || !resolver.requiresAddingImplicitUndefined(node, enclosingDeclaration))) {
			return visitNode(node.type, visitDeclarationSubtree);
		}
		const oldErrorNameNode = errorNameNode;
		errorNameNode = node.name;
		let typeNode;
		if (hasInferredType(node)) typeNode = resolver.createTypeOfDeclaration(node, enclosingDeclaration, symbolTracker);
		else if (isFunctionLike(node)) typeNode = resolver.createReturnTypeOfSignatureDeclaration(node, enclosingDeclaration, symbolTracker);
		errorNameNode = oldErrorNameNode;
		return typeNode ?? f.createKeywordTypeNode(K.AnyKeyword);
	}

	function isDeclarationAndNotVisible(node) {
		node = f.getParseTreeNode(node);
		switch (node.kind) {
			case K.FunctionDeclaration:
			case K.ModuleDeclaration:
			case K.InterfaceDeclaration:
			case K.ClassDeclaration:
			case K.TypeAliasDeclaration:
			case K.EnumDeclaration:
				return !resolver.isDeclarationVisible(node);
			case K.VariableDeclaration:
				return !getBindingNameVisible(node);
			case K.ImportEqualsDeclaration:
			case K.ImportDeclaration:
			case K.ExportDeclaration:
			case K.ExportAssignment:
				return false;
			case K.ClassStaticBlockDeclaration:
				return true;
		}
		return false;
	}

	function getBindingNameVisible(element) {
		if (isOmittedExpression(element)) return false;
		if (isBindingPattern(element.name)) return element.name.elements.some(getBindingNameVisible);
		return resolver.isDeclarationVisible(element);
	}

	function updateParamsList(node, params, modifierMask) {
		if (hasEffectiveModifier(node, ModifierFlags.Private)) return f.createNodeArray();
		if (!params) return f.createNodeArray();
		return f.createNodeArray(
			params.map((parameter) => ensureParameter(parameter, modifierMask)),
			params.hasTrailingComma,
		);
	}

	function updateAccessorParamsList(input, isPrivate) {
		let newParams;
		if (!isPrivate) {
			const thisParameter = input.parameters.length && parameterIsThisKeyword(input.parameters[0]) ? input.parameters[0] : undefined;
			if (thisParameter) newParams = [ensureParameter(thisParameter)];
		}
		if (input.kind === K.SetAccessor) {
			let newValueParameter;
			if (!isPrivate) {
				const hasThis = input.parameters.length === 2 && parameterIsThisKeyword(input.parameters[0]);
				const valueParameter = input.parameters[hasThis ? 1 : 0];
				if (valueParameter) newValueParameter = ensureParameter(valueParameter);
			}
			newValueParameter ??= f.createParameterDeclaration(undefined, undefined, "value");
			newParams = [...(newParams ?? []), newValueParameter];
		}
		return f.createNodeArray(newParams ?? []);
	}

	function ensureTypeParams(node, params) {
		return hasEffectiveModifier(node, ModifierFlags.Private) ? undefined : visitNodes(params, visitDeclarationSubtree);
	}

	function preserveJsDoc(updated, original) {
		return f.setCommentRange(updated, f.getCommentRange(original));
	}

	function transformImportEqualsDeclaration(declaration) {
		if (!resolver.isDeclarationVisible(declaration)) return undefined;
		if (declaration.moduleReference.kind === K.ExternalModuleReference) {
			resultHasExternalModuleIndicator = true;
			return declaration;
		}
		checkEntityNameVisibility(declaration.moduleReference, enclosingDeclaration);
		return declaration;
	}

	function updateImportDeclaration(declaration, importClause) {
		resultHasExternalModuleIndicator = true;
		return f.updateNode(declaration, {
			modifiers: declaration.modifiers,
			importClause,
			moduleSpecifier: declaration.moduleSpecifier,
			attributes: resolutionModeOverrideOf(declaration.attributes),
		});
	}

	function updateImportClause(clause, phaseModifier, name, namedBindings) {
		const updated = f.updateNode(clause, { name, namedBindings });
		if (updated !== clause) updated.phaseModifier = phaseModifier;
		return updated;
	}

	function transformImportDeclaration(declaration) {
		if (!declaration.importClause) return updateImportDeclaration(declaration, declaration.importClause);
		const clause = declaration.importClause;
		const phase = phaseModifierOf(clause);
		const phaseModifier = phase === K.DeferKeyword ? undefined : phase;
		const visibleDefaultBinding = clause.name && resolver.isDeclarationVisible(clause) ? clause.name : undefined;
		if (!clause.namedBindings) {
			return visibleDefaultBinding && updateImportDeclaration(declaration, updateImportClause(clause, phaseModifier, visibleDefaultBinding, undefined));
		}
		if (clause.namedBindings.kind === K.NamespaceImport) {
			const namedBindings = resolver.isDeclarationVisible(clause.namedBindings) ? clause.namedBindings : undefined;
			return visibleDefaultBinding || namedBindings
				? updateImportDeclaration(declaration, updateImportClause(clause, phaseModifier, visibleDefaultBinding, namedBindings))
				: undefined;
		}
		const bindingList = clause.namedBindings.elements.filter((binding) => resolver.isDeclarationVisible(binding));
		if (bindingList.length || visibleDefaultBinding) {
			return updateImportDeclaration(
				declaration,
				updateImportClause(
					clause,
					phaseModifier,
					visibleDefaultBinding,
					bindingList.length ? f.updateNode(clause.namedBindings, { elements: f.createNodeArray(bindingList) }) : undefined,
				),
			);
		}
		if (resolver.isImportRequiredByAugmentation(declaration)) return updateImportDeclaration(declaration, undefined);
		return undefined;
	}

	function resolutionModeOverrideOf(attributes) {
		return hasResolutionModeOverride(attributes) ? attributes : undefined;
	}

	function transformAndReplaceLatePaintedStatements(statements) {
		while (lateMarkedStatements && lateMarkedStatements.length) {
			const statement = lateMarkedStatements.shift();
			const priorNeedsDeclare = needsDeclare;
			needsDeclare = statement.parent && isSourceFile(statement.parent);
			const result = transformTopLevelDeclaration(statement);
			needsDeclare = priorNeedsDeclare;
			lateStatementReplacementMap.set(f.getOriginalNode(statement), result);
		}
		return visitNodes(statements, (statement) => {
			if (isLateVisibilityPaintedStatement(statement)) {
				const key = f.getOriginalNode(statement);
				if (lateStatementReplacementMap.has(key)) {
					const result = lateStatementReplacementMap.get(key);
					lateStatementReplacementMap.delete(key);
					if (result) {
						if (Array.isArray(result) ? result.some(needsScopeMarker) : needsScopeMarker(result)) needsScopeFixMarker = true;
						if (isSourceFile(statement.parent) && (Array.isArray(result) ? result.some(isExternalModuleIndicator) : isExternalModuleIndicator(result))) {
							resultHasExternalModuleIndicator = true;
						}
					}
					return result;
				}
			}
			return statement;
		});
	}

	function visitDeclarationSubtree(input) {
		if (shouldStripInternal(input)) return undefined;
		if (isDeclaration(input)) {
			if (isDeclarationAndNotVisible(input)) return undefined;
			if (hasDynamicName(input) && (!resolver.isLateBound(f.getParseTreeNode(input)) || !isEntityNameExpression(input.name.expression))) return undefined;
		}
		if (isFunctionLike(input) && resolver.isImplementationOfOverload(input)) return undefined;
		if (input.kind === K.SemicolonClassElement) return undefined;
		let previousEnclosingDeclaration;
		if (isEnclosingDeclaration(input)) {
			previousEnclosingDeclaration = enclosingDeclaration;
			enclosingDeclaration = input;
		}
		const canProduceDiagnostic = canProduceDiagnostics(input);
		const oldWithinObjectLiteralType = suppressNewDiagnosticContexts;
		let shouldEnterSuppressNewDiagnosticsContext =
			(input.kind === K.TypeLiteral || input.kind === K.MappedType) && input.parent && input.parent.kind !== K.TypeAliasDeclaration;
		if ((input.kind === K.MethodDeclaration || input.kind === K.MethodSignature) && hasEffectiveModifier(input, ModifierFlags.Private)) {
			if (!resolver.isFirstDeclarationOfSymbol(input)) return undefined;
			return cleanup(f.createPropertyDeclaration(ensureModifiers(input), input.name, undefined, undefined, undefined));
		}
		if (isTypeQueryNode(input)) checkEntityNameVisibility(input.exprName, enclosingDeclaration);
		if (shouldEnterSuppressNewDiagnosticsContext) suppressNewDiagnosticContexts = true;
		if (isProcessedComponent(input)) {
			switch (input.kind) {
				case K.ExpressionWithTypeArguments: {
					if (isEntityName(input.expression) || isEntityNameExpression(input.expression)) checkEntityNameVisibility(input.expression, enclosingDeclaration);
					return cleanup(visitEachChild(input, visitDeclarationSubtree));
				}
				case K.TypeReference: {
					checkEntityNameVisibility(input.typeName, enclosingDeclaration);
					return cleanup(visitEachChild(input, visitDeclarationSubtree));
				}
				case K.ConstructSignature:
					return cleanup(
						f.updateNode(input, {
							typeParameters: ensureTypeParams(input, input.typeParameters),
							parameters: updateParamsList(input, input.parameters),
							type: ensureType(input),
						}),
					);
				case K.Constructor:
					return cleanup(f.createConstructorDeclaration(ensureModifiers(input), updateParamsList(input, input.parameters, ModifierFlags.None), undefined));
				case K.MethodDeclaration:
					if (isPrivateIdentifier(input.name)) return cleanup(undefined);
					return cleanup(
						f.createMethodDeclaration(
							ensureModifiers(input),
							undefined,
							input.name,
							input.questionToken,
							ensureTypeParams(input, input.typeParameters),
							updateParamsList(input, input.parameters),
							ensureType(input),
							undefined,
						),
					);
				case K.GetAccessor:
					if (isPrivateIdentifier(input.name)) return cleanup(undefined);
					return cleanup(
						f.updateNode(input, {
							modifiers: asNodeArray(ensureModifiers(input)),
							name: input.name,
							parameters: updateAccessorParamsList(input, hasEffectiveModifier(input, ModifierFlags.Private)),
							type: ensureType(input),
							body: undefined,
						}),
					);
				case K.SetAccessor:
					if (isPrivateIdentifier(input.name)) return cleanup(undefined);
					return cleanup(
						f.updateNode(input, {
							modifiers: asNodeArray(ensureModifiers(input)),
							name: input.name,
							parameters: updateAccessorParamsList(input, hasEffectiveModifier(input, ModifierFlags.Private)),
							body: undefined,
						}),
					);
				case K.PropertyDeclaration:
					if (isPrivateIdentifier(input.name)) return cleanup(undefined);
					return cleanup(
						f.updateNode(input, {
							modifiers: asNodeArray(ensureModifiers(input)),
							name: input.name,
							questionToken: input.questionToken,
							exclamationToken: undefined,
							type: ensureType(input),
							initializer: ensureNoInitializer(input),
						}),
					);
				case K.PropertySignature:
					if (isPrivateIdentifier(input.name)) return cleanup(undefined);
					return cleanup(
						f.updateNode(input, {
							modifiers: asNodeArray(ensureModifiers(input)),
							name: input.name,
							questionToken: input.questionToken,
							type: ensureType(input),
						}),
					);
				case K.MethodSignature:
					if (isPrivateIdentifier(input.name)) return cleanup(undefined);
					return cleanup(
						f.updateNode(input, {
							modifiers: asNodeArray(ensureModifiers(input)),
							name: input.name,
							questionToken: input.questionToken,
							typeParameters: ensureTypeParams(input, input.typeParameters),
							parameters: updateParamsList(input, input.parameters),
							type: ensureType(input),
						}),
					);
				case K.CallSignature:
					return cleanup(
						f.updateNode(input, {
							typeParameters: ensureTypeParams(input, input.typeParameters),
							parameters: updateParamsList(input, input.parameters),
							type: ensureType(input),
						}),
					);
				case K.IndexSignature:
					return cleanup(
						f.updateNode(input, {
							modifiers: asNodeArray(ensureModifiers(input)),
							parameters: updateParamsList(input, input.parameters),
							type: visitNode(input.type, visitDeclarationSubtree) || f.createKeywordTypeNode(K.AnyKeyword),
						}),
					);
				case K.VariableDeclaration:
					if (isBindingPattern(input.name)) return recreateBindingPattern(input.name);
					shouldEnterSuppressNewDiagnosticsContext = true;
					suppressNewDiagnosticContexts = true;
					return cleanup(
						f.updateNode(input, {
							name: input.name,
							exclamationToken: undefined,
							type: ensureType(input),
							initializer: ensureNoInitializer(input),
						}),
					);
				case K.TypeParameter:
					if (input.parent && input.parent.kind === K.MethodDeclaration && hasEffectiveModifier(input.parent, ModifierFlags.Private) && (input.default || input.constraint)) {
						return cleanup(f.updateNode(input, { modifiers: input.modifiers, name: input.name, constraint: undefined, default: undefined }));
					}
					return cleanup(visitEachChild(input, visitDeclarationSubtree));
				case K.ConditionalType: {
					const checkType = visitNode(input.checkType, visitDeclarationSubtree);
					const extendsType = visitNode(input.extendsType, visitDeclarationSubtree);
					const oldEnclosingDeclaration = enclosingDeclaration;
					enclosingDeclaration = input.trueType;
					const trueType = visitNode(input.trueType, visitDeclarationSubtree);
					enclosingDeclaration = oldEnclosingDeclaration;
					const falseType = visitNode(input.falseType, visitDeclarationSubtree);
					return cleanup(f.updateNode(input, { checkType, extendsType, trueType, falseType }));
				}
				case K.FunctionType:
					return cleanup(
						f.updateNode(input, {
							typeParameters: visitNodes(input.typeParameters, visitDeclarationSubtree),
							parameters: updateParamsList(input, input.parameters),
							type: visitNode(input.type, visitDeclarationSubtree),
						}),
					);
				case K.ConstructorType:
					return cleanup(
						f.updateNode(input, {
							modifiers: asNodeArray(ensureModifiers(input)),
							typeParameters: visitNodes(input.typeParameters, visitDeclarationSubtree),
							parameters: updateParamsList(input, input.parameters),
							type: visitNode(input.type, visitDeclarationSubtree),
						}),
					);
				case K.ImportType: {
					const isLiteral = input.argument && input.argument.kind === K.LiteralType && input.argument.literal.kind === K.StringLiteral;
					if (!isLiteral) return cleanup(input);
					return cleanup(
						f.updateNode(input, {
							argument: input.argument,
							attributes: input.attributes,
							qualifier: input.qualifier,
							typeArguments: visitNodes(input.typeArguments, visitDeclarationSubtree),
						}),
					);
				}
			}
		}
		if (isTupleTypeNode(input) && resolver.isRangeOnOneLine(input)) f.setEmitFlags(input, f.EmitFlags.SingleLine);
		return cleanup(visitEachChild(input, visitDeclarationSubtree));

		function cleanup(returnValue) {
			if (returnValue && canProduceDiagnostic && hasDynamicName(input)) checkEntityNameVisibility(input.name.expression, enclosingDeclaration);
			if (isEnclosingDeclaration(input)) enclosingDeclaration = previousEnclosingDeclaration;
			if (shouldEnterSuppressNewDiagnosticsContext) suppressNewDiagnosticContexts = oldWithinObjectLiteralType;
			if (returnValue === input) return returnValue;
			return returnValue && f.setOriginalNode(preserveJsDoc(returnValue, input), input);
		}
	}

	function visitDeclarationStatements(input) {
		if (!isPreservedDeclarationStatement(input)) return undefined;
		if (shouldStripInternal(input)) return undefined;
		switch (input.kind) {
			case K.ExportDeclaration:
				if (isSourceFile(input.parent)) resultHasExternalModuleIndicator = true;
				resultHasScopeMarker = true;
				return f.updateNode(input, {
					modifiers: input.modifiers,
					exportClause: input.exportClause,
					moduleSpecifier: input.moduleSpecifier,
					attributes: resolutionModeOverrideOf(input.attributes),
				});
			case K.ExportAssignment:
				if (isSourceFile(input.parent)) resultHasExternalModuleIndicator = true;
				resultHasScopeMarker = true;
				if (input.expression.kind === K.Identifier) return input;
				throw new Error("surge-ts: declaration emit of an `export default` expression is not supported yet");
		}
		const result = transformTopLevelDeclaration(input);
		lateStatementReplacementMap.set(f.getOriginalNode(input), result);
		return input;
	}

	function stripExportModifiers(statement) {
		if (statement.kind === K.ImportEqualsDeclaration || hasEffectiveModifier(statement, ModifierFlags.Default) || !statement.modifiers) return statement;
		const modifiers = f.createModifiersFromModifierFlags(modifierFlags(statement) & (ModifierFlags.All ^ ModifierFlags.Export));
		return f.updateNode(statement, { modifiers: asNodeArray(modifiers) });
	}

	function transformTopLevelDeclaration(input) {
		if (lateMarkedStatements) {
			let index;
			while ((index = lateMarkedStatements.indexOf(input)) >= 0) lateMarkedStatements.splice(index, 1);
		}
		if (shouldStripInternal(input)) return undefined;
		switch (input.kind) {
			case K.ImportEqualsDeclaration:
				return transformImportEqualsDeclaration(input);
			case K.ImportDeclaration:
				return transformImportDeclaration(input);
		}
		if (isDeclaration(input) && isDeclarationAndNotVisible(input)) return undefined;
		if (isFunctionLike(input) && resolver.isImplementationOfOverload(input)) return undefined;
		let previousEnclosingDeclaration;
		if (isEnclosingDeclaration(input)) {
			previousEnclosingDeclaration = enclosingDeclaration;
			enclosingDeclaration = input;
		}
		const previousNeedsDeclare = needsDeclare;
		switch (input.kind) {
			case K.TypeAliasDeclaration: {
				needsDeclare = false;
				const result = cleanup(
					f.updateNode(input, {
						modifiers: asNodeArray(ensureModifiers(input)),
						name: input.name,
						typeParameters: visitNodes(input.typeParameters, visitDeclarationSubtree),
						type: visitNode(input.type, visitDeclarationSubtree),
					}),
				);
				needsDeclare = previousNeedsDeclare;
				return result;
			}
			case K.InterfaceDeclaration:
				return cleanup(
					f.updateNode(input, {
						modifiers: asNodeArray(ensureModifiers(input)),
						name: input.name,
						typeParameters: ensureTypeParams(input, input.typeParameters),
						heritageClauses: transformHeritageClauses(input.heritageClauses),
						members: visitNodes(input.members, visitDeclarationSubtree),
					}),
				);
			case K.FunctionDeclaration: {
				const result = cleanup(
					f.updateNode(input, {
						modifiers: asNodeArray(ensureModifiers(input)),
						asteriskToken: undefined,
						name: input.name,
						typeParameters: ensureTypeParams(input, input.typeParameters),
						parameters: updateParamsList(input, input.parameters),
						type: ensureType(input),
						body: undefined,
					}),
				);
				if (result && resolver.isExpandoFunctionDeclaration(input)) {
					throw new Error("surge-ts: declaration emit of a function with assigned properties is not supported yet");
				}
				return result;
			}
			case K.ModuleDeclaration: {
				needsDeclare = false;
				const inner = input.body;
				if (inner && inner.kind === K.ModuleBlock) {
					const oldNeedsScopeFix = needsScopeFixMarker;
					const oldHasScopeFix = resultHasScopeMarker;
					resultHasScopeMarker = false;
					needsScopeFixMarker = false;
					const statements = visitNodes(inner.statements, visitDeclarationStatements);
					let lateStatements = transformAndReplaceLatePaintedStatements(statements);
					if (input.flags & NodeFlags.Ambient) needsScopeFixMarker = false;
					if (!isGlobalScopeAugmentation(input) && !lateStatements.some(isScopeMarker) && !resultHasScopeMarker) {
						if (needsScopeFixMarker) lateStatements = f.createNodeArray([...lateStatements, f.createEmptyExports()]);
						else lateStatements = visitNodes(lateStatements, stripExportModifiers);
					}
					const body = f.updateNode(inner, { statements: lateStatements });
					needsDeclare = previousNeedsDeclare;
					needsScopeFixMarker = oldNeedsScopeFix;
					resultHasScopeMarker = oldHasScopeFix;
					const modifiers = asNodeArray(ensureModifiers(input));
					return cleanup(updateModuleDeclarationAndKeyword(input, modifiers, input.name, body));
				}
				needsDeclare = previousNeedsDeclare;
				const modifiers = asNodeArray(ensureModifiers(input));
				needsDeclare = false;
				visitNode(inner, visitDeclarationStatements);
				const key = f.getOriginalNode(inner);
				const body = lateStatementReplacementMap.get(key);
				lateStatementReplacementMap.delete(key);
				return cleanup(updateModuleDeclarationAndKeyword(input, modifiers, input.name, body));
			}
			case K.ClassDeclaration: {
				const modifiers = f.createNodeArray(ensureModifiers(input));
				const typeParameters = ensureTypeParams(input, input.typeParameters);
				const ctor = input.members.find((member) => member.kind === K.Constructor && member.body);
				let parameterProperties;
				if (ctor) {
					parameterProperties = [];
					for (const param of ctor.parameters) {
						if (!(modifierFlags(param) & ModifierFlags.ParameterPropertyModifier) || shouldStripInternal(param)) continue;
						if (param.name.kind !== K.Identifier) {
							throw new Error("surge-ts: declaration emit of a destructured parameter property is not supported yet");
						}
						parameterProperties.push(
							preserveJsDoc(f.createPropertyDeclaration(ensureModifiers(param), param.name, param.questionToken, ensureType(param), ensureNoInitializer(param)), param),
						);
					}
				}
				const privateIdentifier = input.members.some((member) => member.name && isPrivateIdentifier(member.name))
					? [f.createPropertyDeclaration(undefined, f.createPrivateIdentifier("#private"), undefined, undefined, undefined)]
					: undefined;
				const members = f.createNodeArray(concatenate(concatenate(privateIdentifier, parameterProperties), visitNodes(input.members, visitDeclarationSubtree)));
				const extendsClause = (input.heritageClauses ?? []).find((clause) => clause.token === K.ExtendsKeyword)?.types[0];
				if (extendsClause && !isEntityNameExpression(extendsClause.expression) && extendsClause.expression.kind !== K.NullKeyword) {
					throw new Error("surge-ts: declaration emit of a class extending an expression is not supported yet");
				}
				return cleanup(
					f.updateNode(input, {
						modifiers,
						name: input.name,
						typeParameters,
						heritageClauses: transformHeritageClauses(input.heritageClauses),
						members,
					}),
				);
			}
			case K.VariableStatement:
				return cleanup(transformVariableStatement(input));
			case K.EnumDeclaration:
				return cleanup(
					f.updateNode(input, {
						modifiers: f.createNodeArray(ensureModifiers(input)),
						name: input.name,
						members: f.createNodeArray(
							input.members
								.filter((member) => !shouldStripInternal(member))
								.map((member) => {
									const value = resolver.getEnumMemberValue(member);
									const constValue = value?.value;
									const initializer =
										constValue === undefined
											? undefined
											: typeof constValue === "string"
												? f.createStringLiteral(constValue)
												: constValue < 0
													? f.createPrefixUnaryExpression(K.MinusToken, f.createNumericLiteral(-constValue))
													: f.createNumericLiteral(constValue);
									return preserveJsDoc(f.updateNode(member, { name: member.name, initializer }), member);
								}),
						),
					}),
				);
		}
		throw new Error(`surge-ts: unhandled top-level declaration (kind ${input.kind})`);

		function cleanup(node) {
			if (isEnclosingDeclaration(input)) enclosingDeclaration = previousEnclosingDeclaration;
			if (input.kind === K.ModuleDeclaration) needsDeclare = previousNeedsDeclare;
			if (node === input) return node;
			return node && f.setOriginalNode(preserveJsDoc(node, input), input);
		}
	}

	function updateModuleDeclarationAndKeyword(node, modifiers, name, body) {
		const updated = f.updateNode(node, { modifiers, name, body });
		if (isAmbientModule(updated) || updated.flags & NodeFlags.Namespace) return updated;
		const fixed = f.createModuleDeclaration(updated.modifiers, updated.name, updated.body, updated.flags | NodeFlags.Namespace);
		f.setOriginalNode(fixed, updated);
		f.setTextRange(fixed, updated);
		return fixed;
	}

	function transformVariableStatement(input) {
		if (!input.declarationList.declarations.some(getBindingNameVisible)) return undefined;
		const nodes = visitNodes(input.declarationList.declarations, visitDeclarationSubtree);
		if (!nodes.length) return undefined;
		const modifiers = f.createNodeArray(ensureModifiers(input));
		const list = input.declarationList;
		let declarationList;
		if (list.flags & NodeFlags.Using) {
			declarationList = f.createVariableDeclarationList(nodes, NodeFlags.Const);
			f.setOriginalNode(declarationList, list);
			f.setTextRange(declarationList, list);
			f.setCommentRange(declarationList, list);
		} else {
			declarationList = f.updateNode(list, { declarations: nodes });
		}
		return f.updateNode(input, { modifiers, declarationList });
	}

	function recreateBindingPattern(pattern) {
		return pattern.elements.flatMap((element) => recreateBindingElement(element) ?? []);
	}

	function recreateBindingElement(element) {
		if (element.kind === K.OmittedExpression) return undefined;
		if (!element.name || !getBindingNameVisible(element)) return undefined;
		if (isBindingPattern(element.name)) return recreateBindingPattern(element.name);
		return f.createVariableDeclaration(element.name, undefined, ensureType(element), undefined);
	}

	function shouldStripInternal(node) {
		return !!stripInternal && !!node && resolver.isInternalDeclaration(node);
	}

	function ensureModifiers(node) {
		const currentFlags = modifierFlags(node);
		const newFlags = ensureModifierFlags(node);
		if (currentFlags === newFlags) return visitArray(node.modifiers, (modifier) => (isModifier(modifier) ? modifier : undefined));
		return f.createModifiersFromModifierFlags(newFlags);
	}

	function ensureModifierFlags(node) {
		let mask = ModifierFlags.All ^ (ModifierFlags.Public | ModifierFlags.Async | ModifierFlags.Override);
		let additions = needsDeclare && !isAlwaysType(node) ? ModifierFlags.Ambient : ModifierFlags.None;
		if (node.parent.kind !== K.SourceFile) {
			mask ^= ModifierFlags.Ambient;
			additions = ModifierFlags.None;
		}
		return maskModifierFlags(node, mask, additions);
	}

	function transformHeritageClauses(nodes) {
		return f.createNodeArray(
			(nodes ?? [])
				.map((clause) =>
					f.updateNode(clause, {
						types: visitNodes(
							f.createNodeArray(
								filter(clause.types, (type) => isEntityNameExpression(type.expression) || (clause.token === K.ExtendsKeyword && type.expression.kind === K.NullKeyword)),
							),
							visitDeclarationSubtree,
						),
					}),
				)
				.filter((clause) => clause.types && clause.types.length),
		);
	}
}

module.exports = {
	transformDeclarations,
	phaseModifierOf,
	modifierFlags,
	isFunctionLike,
	isFunctionLikeDeclarationKind,
	isDeclaration,
	hasInferredType,
	isPrimitiveLiteralValue,
	isAmbientModule,
	isLateVisibilityPaintedStatement,
};
