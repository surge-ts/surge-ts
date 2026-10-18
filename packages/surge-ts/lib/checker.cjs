"use strict";

// TypeScript's `TypeChecker`, `Type`, `Symbol` and `Signature` over the
// native checker's handles. A wrapper is created once per handle and cached
// by its program, so asking twice for the same type, symbol or signature
// gives the same object.
const { TypeFlags, ObjectFlags, SymbolFlags, SignatureKind, SyntaxKind, ModifierFlags, NodeFlags, ElementFlags } = require("./enums.cjs");
const { ownNode, escapeLeadingUnderscores, unescapeLeadingUnderscores } = require("./nodes.cjs");
const { documentationOf, jsDocTagsOf } = require("./jsdoc.cjs");
const {
	isOptionalChain,
	hasSyntacticModifier,
	isStringLiteralLike,
	isFunctionLikeDeclaration,
	getPropertyNameForPropertyNameNode,
} = require("./utilities.cjs");
const { EmitResolver } = require("./resolver.cjs");

function hasUndefined(type) {
	return !!(type.flags & TypeFlags.Undefined) || (type.isUnion() && type.types.some((member) => member.flags & TypeFlags.Undefined));
}

function isOptionalChainRoot(node) {
	return isOptionalChain(node) && node.kind !== SyntaxKind.NonNullExpression && !!node.questionDotToken;
}

/** tsc's `isOutermostOptionalChain`. */
function isOutermostOptionalChain(node) {
	return !isOptionalChain(node.parent) || isOptionalChainRoot(node.parent) || node !== node.parent.expression;
}

/** tsc's `isInPropertyInitializerOrClassStaticBlock(node, ignoreArrowFunctions: true)`. */
function isInPropertyInitializerOrClassStaticBlock(node) {
	for (let current = node; current; current = current.parent) {
		switch (current.kind) {
			case SyntaxKind.PropertyDeclaration:
			case SyntaxKind.ClassStaticBlockDeclaration:
				return true;
			case SyntaxKind.TypeQuery:
			case SyntaxKind.JsxClosingElement:
				return false;
			case SyntaxKind.Block:
				if (current.parent && isFunctionLikeDeclaration(current.parent) && current.parent.kind !== SyntaxKind.ArrowFunction) return false;
		}
	}
	return false;
}

/** `as const`: tsc's `isConstTypeReference`. */
function isConstTypeReference(node) {
	return node.kind === SyntaxKind.TypeReference && node.typeName.kind === SyntaxKind.Identifier && node.typeName.text === "const" && !node.typeArguments;
}

function isTypeOnlyImportOrExportDeclaration(node) {
	switch (node.kind) {
		case SyntaxKind.ImportClause:
		case SyntaxKind.ImportEqualsDeclaration:
			return node.isTypeOnly;
		case SyntaxKind.ImportSpecifier:
		case SyntaxKind.ExportSpecifier:
			return node.isTypeOnly || node.parent.parent.isTypeOnly;
		case SyntaxKind.NamespaceImport:
		case SyntaxKind.NamespaceExport:
			return node.parent.isTypeOnly;
	}
	return false;
}

class TypeObject {
	constructor(checker, id) {
		this.checker = checker;
		this._id = id;
		this._flags = undefined;
	}
	get flags() {
		if (this._flags === undefined) this._flags = this.checker._native.typeFlags(this._id);
		return this._flags;
	}
	get objectFlags() {
		if (this._objectFlags === undefined) this._objectFlags = this.flags & TypeFlags.Object ? this._computeObjectFlags() : 0;
		return this._objectFlags;
	}
	_computeObjectFlags() {
		const flags = this.checker._native.objectFlags(this._id);
		if (!(flags & ObjectFlags.Reference) || flags & ObjectFlags.ClassOrInterface) return flags;
		// A generic class's or interface's own declared type (`Box<T>`) is
		// the class or interface itself, which references it.
		const symbol = this.symbol;
		if (!symbol || !(symbol.flags & (SymbolFlags.Class | SymbolFlags.Interface))) return flags;
		if (this.checker.getDeclaredTypeOfSymbol(symbol) !== this) return flags;
		return flags | (symbol.flags & SymbolFlags.Class ? ObjectFlags.Class : ObjectFlags.Interface);
	}
	get symbol() {
		return this.checker._symbolOrUndefined(this.checker._native.typeSymbol(this._id));
	}
	get aliasSymbol() {
		return this.checker._symbolOrUndefined(this.checker._native.aliasSymbol(this._id));
	}
	get aliasTypeArguments() {
		const alias = this.aliasSymbol;
		const declaration = alias?.declarations?.[0];
		if (!declaration?.typeParameters?.length) return undefined;
		throw new Error("surge-ts: Type.aliasTypeArguments is not supported yet");
	}
	/**
	 * `TypeReference.target`: a tuple's `TupleType` (its element flags and
	 * lengths); a generic instantiation's declared type.
	 */
	get target() {
		if (this._target === undefined) this._target = this._computeTarget() ?? null;
		return this._target ?? undefined;
	}
	_computeTarget() {
		if (!(this.objectFlags & ObjectFlags.Reference)) return undefined;
		const shape = this.checker._native.tupleShape(this._id);
		if (shape) return new TupleTarget(shape);
		const symbol = this.symbol;
		return symbol && symbol.flags & (SymbolFlags.Class | SymbolFlags.Interface) ? this.checker.getDeclaredTypeOfSymbol(symbol) : undefined;
	}
	get value() {
		return this.flags & (TypeFlags.StringLiteral | TypeFlags.NumberLiteral) ? this.checker._native.literalValue(this._id) : undefined;
	}
	get types() {
		return this.flags & (TypeFlags.Union | TypeFlags.Intersection)
			? this.checker._native.typeConstituents(this._id).map((id) => this.checker._type(id))
			: undefined;
	}
	get intrinsicName() {
		return this.flags & TypeFlags.Intrinsic ? this.checker.typeToString(this) : undefined;
	}
	getFlags() {
		return this.flags;
	}
	getSymbol() {
		return this.symbol;
	}
	getProperties() {
		return this.checker.getPropertiesOfType(this);
	}
	getProperty(name) {
		return this.checker.getPropertyOfType(this, name);
	}
	getApparentProperties() {
		return this.checker.getPropertiesOfType(this.checker.getApparentType(this));
	}
	getCallSignatures() {
		return this.checker.getSignaturesOfType(this, SignatureKind.Call);
	}
	getConstructSignatures() {
		return this.checker.getSignaturesOfType(this, SignatureKind.Construct);
	}
	getStringIndexType() {
		return this.checker._typeOrUndefined(this.checker._native.indexTypeOfType(this._id, false));
	}
	getNumberIndexType() {
		return this.checker._typeOrUndefined(this.checker._native.indexTypeOfType(this._id, true));
	}
	getNonNullableType() {
		return this.checker.getNonNullableType(this);
	}
	getBaseTypes() {
		return this.isClassOrInterface() ? this.checker.getBaseTypes(this) : undefined;
	}
	getConstraint() {
		return this.checker.getBaseConstraintOfType(this);
	}
	getDefault() {
		if (!this.isTypeParameter()) return undefined;
		throw new Error("surge-ts: Type.getDefault is not supported yet");
	}
	isUnion() {
		return (this.flags & TypeFlags.Union) !== 0;
	}
	isIntersection() {
		return (this.flags & TypeFlags.Intersection) !== 0;
	}
	isUnionOrIntersection() {
		return (this.flags & TypeFlags.UnionOrIntersection) !== 0;
	}
	isLiteral() {
		return (this.flags & (TypeFlags.StringLiteral | TypeFlags.NumberLiteral | TypeFlags.BigIntLiteral)) !== 0;
	}
	isStringLiteral() {
		return (this.flags & TypeFlags.StringLiteral) !== 0;
	}
	isNumberLiteral() {
		return (this.flags & TypeFlags.NumberLiteral) !== 0;
	}
	isTypeParameter() {
		return (this.flags & TypeFlags.TypeParameter) !== 0;
	}
	isClassOrInterface() {
		return (this.objectFlags & ObjectFlags.ClassOrInterface) !== 0;
	}
	isClass() {
		return (this.objectFlags & ObjectFlags.Class) !== 0;
	}
	isIndexType() {
		return (this.flags & TypeFlags.Index) !== 0;
	}
}

/** tsc's `TupleType`, the target a tuple reference instantiates. */
class TupleTarget {
	constructor(shape) {
		const flags = shape.elementFlags;
		this.elementFlags = flags;
		this.readonly = shape.readonly;
		this.objectFlags = ObjectFlags.Tuple | ObjectFlags.Reference;
		this.flags = TypeFlags.Object;
		this.minLength = flags.reduce((length, flag, index) => (flag & ElementFlags.Required ? index + 1 : length), 0);
		const firstVariable = flags.findIndex((flag) => flag & ElementFlags.Variable);
		this.fixedLength = firstVariable < 0 ? flags.length : firstVariable;
		this.hasRestElement = flags.some((flag) => flag & ElementFlags.Variable);
		this.combinedFlags = flags.reduce((combined, flag) => combined | flag, 0);
		// surge's tuples keep no element labels yet: every slot is unlabeled.
		this.labeledElementDeclarations = flags.map(() => undefined);
	}
}

class SymbolObject {
	constructor(checker, id) {
		this._checker = checker;
		this._id = id;
		this._info = undefined;
	}
	_details() {
		if (this._info === undefined) this._info = this._checker._native.symbolInfo(this._id);
		return this._info;
	}
	get name() {
		return this._details().name;
	}
	get escapedName() {
		return this._details().escapedName;
	}
	get flags() {
		return this._details().flags;
	}
	get declarations() {
		const declarations = this._checker._native.symbolDeclarations(this._id);
		return declarations.length ? declarations.map((reference) => this._checker._program._nodeOf(reference)) : undefined;
	}
	get valueDeclaration() {
		const reference = this._checker._native.symbolValueDeclaration(this._id);
		return reference ? this._checker._program._nodeOf(reference) : undefined;
	}
	get parent() {
		return this._checker._symbolOrUndefined(this._checker._native.symbolParent(this._id));
	}
	get members() {
		return this._table(this._checker._native.symbolMembers(this._id));
	}
	get exports() {
		return this._table(this._checker._native.symbolExports(this._id));
	}
	_table(ids) {
		if (ids.length === 0) return undefined;
		const table = new Map();
		for (const id of ids) {
			const symbol = this._checker._symbol(id);
			table.set(symbol.escapedName, symbol);
		}
		return table;
	}
	getName() {
		return this.name;
	}
	getEscapedName() {
		return this.escapedName;
	}
	getFlags() {
		return this.flags;
	}
	getDeclarations() {
		return this.declarations;
	}
	getDocumentationComment() {
		return documentationOf(this.declarations ?? []);
	}
	getJsDocTags() {
		return jsDocTagsOf(this.declarations ?? []);
	}
}

class SignatureObject {
	constructor(checker, id) {
		this._checker = checker;
		this._id = id;
	}
	get declaration() {
		const reference = this._checker._native.signatureDeclaration(this._id);
		return reference ? this._checker._program._nodeOf(reference) : undefined;
	}
	get parameters() {
		return this.getParameters();
	}
	get typeParameters() {
		const typeParameters = this.getTypeParameters();
		return typeParameters.length ? typeParameters : undefined;
	}
	get thisParameter() {
		return this._checker._symbolOrUndefined(this._checker._native.signatureThisParameter(this._id));
	}
	getDeclaration() {
		return this.declaration;
	}
	getTypeParameters() {
		return this._checker._native.signatureTypeParameters(this._id).map((id) => this._checker._type(id));
	}
	getParameters() {
		return this._checker._native.signatureParameters(this._id).map((id) => this._checker._symbol(id));
	}
	getReturnType() {
		return this._checker.getReturnTypeOfSignature(this);
	}
	getDocumentationComment() {
		const declaration = this.declaration;
		return declaration ? documentationOf([declaration]) : [];
	}
	getJsDocTags() {
		const declaration = this.declaration;
		return declaration ? jsDocTagsOf([declaration]) : [];
	}
}

/**
 * A generic signature as a call instantiated it: its return type is the
 * call's, and a parameter an argument fills has the type the checker
 * instantiated it to there.
 */
class InstantiatedSignature extends SignatureObject {
	constructor(checker, id, returnType, call) {
		super(checker, id);
		this._returnType = returnType;
		this._call = call;
	}
	/** The parameter's type in this instantiation, where an argument says. */
	_parameterType(index) {
		const argument = this._call.arguments?.[index];
		return (argument && argument.kind !== SyntaxKind.SpreadElement && this._checker.getContextualType(argument)) || this._checker.getTypeOfSymbol(this.getParameters()[index]);
	}
	_toString() {
		const checker = this._checker;
		const parameters = this.getParameters().map((parameter, index) => {
			const declaration = parameter.valueDeclaration;
			const rest = !!declaration?.dotDotDotToken;
			const optional = !rest && !!(declaration?.questionToken || declaration?.initializer);
			const type = rest ? checker.getTypeOfSymbol(parameter) : this._parameterType(index);
			return `${rest ? "..." : ""}${parameter.name}${optional ? "?" : ""}: ${checker.typeToString(type)}`;
		});
		return `(${parameters.join(", ")}): ${checker.typeToString(this._returnType)}`;
	}
	get typeParameters() {
		return undefined;
	}
	getTypeParameters() {
		return [];
	}
	getReturnType() {
		return this._returnType;
	}
}

function hasRestParameter(signature) {
	const parameters = signature.getParameters();
	return parameters.length > 0 && !!parameters[parameters.length - 1].valueDeclaration?.dotDotDotToken;
}

/** Whether a call passing `args` fits the signature's parameter count. */
function signatureArityFits(signature, args) {
	const parameters = signature.getParameters();
	const rest = hasRestParameter(signature);
	if (args.some((argument) => argument.kind === SyntaxKind.SpreadElement)) return rest || args.length <= parameters.length;
	const required = parameters.filter((parameter) => {
		const declaration = parameter.valueDeclaration;
		return declaration && !declaration.questionToken && !declaration.initializer && !declaration.dotDotDotToken;
	}).length;
	return args.length >= required && (rest || args.length <= parameters.length);
}

class TypeChecker {
	constructor(program) {
		this._program = program;
		this._native = program._native;
		this._types = new Map();
		this._symbols = new Map();
		this._signatures = new Map();
	}
	_type(id) {
		let type = this._types.get(id);
		if (type === undefined) {
			type = new TypeObject(this, id);
			this._types.set(id, type);
		}
		return type;
	}
	_typeOrUndefined(id) {
		return id === null || id === undefined ? undefined : this._type(id);
	}
	_symbol(id) {
		let symbol = this._symbols.get(id);
		if (symbol === undefined) {
			symbol = new SymbolObject(this, id);
			this._symbols.set(id, symbol);
		}
		return symbol;
	}
	_symbolOrUndefined(id) {
		return id === null || id === undefined ? undefined : this._symbol(id);
	}
	_signature(id) {
		let signature = this._signatures.get(id);
		if (signature === undefined) {
			signature = new SignatureObject(this, id);
			this._signatures.set(id, signature);
		}
		return signature;
	}
	_node(node) {
		ownNode(node);
		const sourceFile = node._sourceFile;
		if (sourceFile._program !== this._program) {
			throw new TypeError("surge-ts: the node belongs to another program");
		}
		return [sourceFile._fileIndex, node._index];
	}
	_typeId(type) {
		if (!(type instanceof TypeObject) || type.checker !== this) throw new TypeError("surge-ts: the type belongs to another program");
		return type._id;
	}
	_symbolId(symbol) {
		if (!(symbol instanceof SymbolObject) || symbol._checker !== this) {
			throw new TypeError("surge-ts: the symbol belongs to another program");
		}
		return symbol._id;
	}
	_signatureId(signature) {
		if (!(signature instanceof SignatureObject) || signature._checker !== this) {
			throw new TypeError("surge-ts: the signature belongs to another program");
		}
		return signature._id;
	}

	getSymbolAtLocation(node) {
		const [file, index] = this._node(node);
		return this._symbolOrUndefined(this._native.symbolAtLocation(file, index));
	}
	getTypeAtLocation(node) {
		const [file, index] = this._node(node);
		return this._withOptionalMarker(node, this._type(this._native.typeAtLocation(file, index)));
	}
	/**
	 * tsc types a link inside an optional chain with an `undefined` the chain
	 * adds (its optional type marker), which `getNonOptionalType` removes.
	 * surge's types do not carry the marker, so the link's type is wrapped
	 * with the type it has without it.
	 */
	_withOptionalMarker(node, type) {
		if (!isOptionalChain(node) || isOutermostOptionalChain(node) || !hasUndefined(type) || this._ownTypeHasUndefined(node)) return type;
		const nonOptional = this.getUnionType(type.isUnion() ? type.types.filter((member) => !(member.flags & TypeFlags.Undefined)) : []);
		const key = `${type._id}:${nonOptional._id}`;
		let marked = this._markedTypes?.get(key);
		if (marked === undefined) {
			marked = new TypeObject(this, type._id);
			marked._nonOptional = nonOptional;
			(this._markedTypes ??= new Map()).set(key, marked);
		}
		return marked;
	}
	/** Whether a chain link's member, element or return type itself includes `undefined`. */
	_ownTypeHasUndefined(node) {
		switch (node.kind) {
			case SyntaxKind.PropertyAccessExpression:
			case SyntaxKind.ElementAccessExpression: {
				// The member as read off the receiver's non-nullable part.
				const receiver = this.getNonNullableType(this.getNonOptionalType(this.getTypeAtLocation(node.expression)));
				const name = node.kind === SyntaxKind.PropertyAccessExpression ? node.name.text : node.argumentExpression.text;
				const symbol = name === undefined ? undefined : this.getPropertyOfType(receiver, name);
				if (!symbol) return node.kind === SyntaxKind.PropertyAccessExpression || !!this._program.getCompilerOptions().noUncheckedIndexedAccess;
				return !!(symbol.flags & SymbolFlags.Optional) || hasUndefined(this.getTypeOfSymbol(symbol));
			}
			case SyntaxKind.CallExpression: {
				const callee = this.getNonNullableType(this.getNonOptionalType(this.getTypeAtLocation(node.expression)));
				const signatures = callee.getCallSignatures();
				return signatures.length === 0 || signatures.some((signature) => hasUndefined(signature.getReturnType()));
			}
		}
		return false;
	}

	// --- TypeScript-internal queries -------------------------------------------

	/**
	 * `getContextualType`: the type the checker checked the expression
	 * against, or where it recorded none, tsc's rule for the position.
	 */
	getContextualType(node) {
		// A property's value: the literal's contextual property (tsc's
		// `getContextualTypeForObjectLiteralElement`), a member's own type.
		const parent = node.parent;
		if (parent?.kind === SyntaxKind.PropertyAssignment && parent.initializer === node && parent.parent?.kind === SyntaxKind.ObjectLiteralExpression) {
			const element = this.getContextualTypeForObjectLiteralElement(parent);
			if (element) return element;
		}
		const [file, index] = this._node(node);
		const recorded = this._typeOrUndefined(this._native.contextualType(file, index));
		// surge checks some library methods (`Array#map`) against signatures of
		// its own, whose parameters have no names; tsc's contextual type is the
		// declared parameter's.
		if (recorded && !this._isSyntheticSignatureType(recorded)) return this._declaredArgumentContextualType(node) ?? recorded;
		return this._argumentContextualType(node) ?? recorded ?? this._syntacticContextualType(node);
	}
	/**
	 * A non-generic call's argument: its parameter in the signature the call
	 * resolves to. The checker records an overloaded call's arguments against
	 * the overloads' fold.
	 */
	_declaredArgumentContextualType(node) {
		const call = node.parent;
		if (!call || (call.kind !== SyntaxKind.CallExpression && call.kind !== SyntaxKind.NewExpression)) return undefined;
		const index = call.arguments?.indexOf(node) ?? -1;
		if (index < 0) return undefined;
		const callee = this.getNonNullableType(this.getNonOptionalType(this.getTypeAtLocation(call.expression)));
		const signatures = call.kind === SyntaxKind.NewExpression ? callee.getConstructSignatures() : callee.getCallSignatures();
		// A lone non-generic signature is the one the call resolves to; the
		// checker's record of the argument's context can lose its `this`.
		if (signatures.length === 1) return signatures[0].getTypeParameters().length ? undefined : this._parameterTypeAt(signatures[0], index);
		const resolved = this.getResolvedSignature(call);
		if (!resolved || resolved instanceof InstantiatedSignature || resolved.getTypeParameters().length > 0) return undefined;
		return this._parameterTypeAt(resolved, index);
	}
	_isSyntheticSignatureType(type) {
		const signatures = type.getCallSignatures();
		return signatures.length > 0 && signatures.every((signature) => {
			const parameters = signature.getParameters();
			return parameters.length > 0 && parameters.every((parameter) => /^arg\d+$/.test(parameter.name));
		});
	}
	/** An argument's parameter, from the callee's declared signatures (tsc's `getContextualTypeForArgument`). */
	_argumentContextualType(node) {
		const call = node.parent;
		if (!call || (call.kind !== SyntaxKind.CallExpression && call.kind !== SyntaxKind.NewExpression)) return undefined;
		const index = call.arguments?.indexOf(node) ?? -1;
		if (index < 0) return undefined;
		const callee = this.getNonNullableType(this.getNonOptionalType(this.getTypeAtLocation(call.expression)));
		const signatures = call.kind === SyntaxKind.NewExpression ? callee.getConstructSignatures() : callee.getCallSignatures();
		const signature = signatures.find((candidate) => signatureArityFits(candidate, call.arguments));
		if (!signature) return undefined;
		const parameters = signature.getParameters();
		if (hasRestParameter(signature) && index >= parameters.length - 1) {
			const restType = this.getTypeOfSymbol(parameters[parameters.length - 1]);
			return this.getTypeOfPropertyOfContextualType(restType, String(index - (parameters.length - 1)));
		}
		return index < parameters.length ? this.getTypeOfSymbol(parameters[index]) : undefined;
	}
	/**
	 * `getResolvedSignature`: the callee's signature the call's arguments fit,
	 * by arity and then by their types. A generic one stands for its
	 * instantiation: its return type is the call's type, as the checker
	 * resolved it.
	 */
	getResolvedSignature(node) {
		let callee;
		let construct = false;
		let args = node.arguments ?? [];
		switch (node.kind) {
			case SyntaxKind.CallExpression:
				callee = node.expression;
				break;
			case SyntaxKind.NewExpression:
				callee = node.expression;
				construct = true;
				break;
			case SyntaxKind.TaggedTemplateExpression:
				callee = node.tag;
				args = [];
				break;
			case SyntaxKind.Decorator:
				callee = node.expression;
				args = [];
				break;
			default:
				return undefined;
		}
		if (callee.kind === SyntaxKind.SuperKeyword) {
			construct = true;
		}
		const calleeType = this.getNonNullableType(this.getNonOptionalType(this.getTypeAtLocation(callee)));
		const signatures = construct ? calleeType.getConstructSignatures() : calleeType.getCallSignatures();
		if (signatures.length === 0) return undefined;
		const fitting = signatures.filter((signature) => signatureArityFits(signature, args));
		const accepts = (signature) =>
			signature.getTypeParameters().length > 0 ||
			args.every((argument, index) => {
				if (argument.kind === SyntaxKind.SpreadElement) return true;
				const parameter = this._parameterTypeAt(signature, index);
				return !parameter || this.isTypeAssignableTo(this.getTypeAtLocation(argument), parameter);
			});
		const chosen = fitting.find(accepts) ?? fitting[0] ?? signatures[signatures.length - 1];
		const generic = chosen.getTypeParameters().length > 0 && !construct && node.kind !== SyntaxKind.Decorator;
		// A call in an optional chain returns `undefined` too (tsc's
		// `createOptionalCallSignature`).
		const chained = node.kind === SyntaxKind.CallExpression && (node.flags & NodeFlags.OptionalChain) !== 0;
		if (generic) return new InstantiatedSignature(this, chosen._id, this.getTypeAtLocation(node), node);
		if (chained) return new InstantiatedSignature(this, chosen._id, this.getUnionType([chosen.getReturnType(), this.getUndefinedType()]), node);
		return chosen;
	}
	/**
	 * `getTypeOfAssignmentPattern`: the type an object or array literal
	 * destructures, read off what is assigned to it, as tsc's checker does.
	 */
	getTypeOfAssignmentPattern(pattern) {
		const parent = pattern.parent;
		switch (parent.kind) {
			case SyntaxKind.ForOfStatement:
				return this._iteratedElementType(this.getTypeAtLocation(parent.expression)) ?? this._errorType();
			case SyntaxKind.BinaryExpression:
				return this.getTypeAtLocation(parent.right);
			case SyntaxKind.PropertyAssignment: {
				const source = this.getTypeOfAssignmentPattern(parent.parent);
				const name = getPropertyNameForPropertyNameNode(parent.name);
				const property = name === undefined ? undefined : this.getPropertyOfType(source, unescapeLeadingUnderscores(name));
				return (property && this.getTypeOfSymbol(property)) ?? source.getStringIndexType() ?? this._errorType();
			}
			case SyntaxKind.ArrayLiteralExpression: {
				const source = this.getTypeOfAssignmentPattern(parent);
				const index = parent.elements.indexOf(pattern);
				if (this.isTupleType(source)) {
					const element = this.getTypeArguments(source)[index];
					if (element) return element;
				}
				return this._iteratedElementType(source) ?? this._errorType();
			}
		}
		return this._errorType();
	}
	/** The element a `for…of` or array destructuring reads off `type`. */
	_iteratedElementType(type) {
		if (type.flags & TypeFlags.StringLike) return this.getStringType();
		if (type.isUnion()) {
			const elements = type.types.map((member) => this._iteratedElementType(member));
			return elements.every(Boolean) ? this.getUnionType(elements) : undefined;
		}
		return type.getNumberIndexType();
	}
	/** The type a signature's parameter at `index` takes, through its rest. */
	_parameterTypeAt(signature, index) {
		const parameters = signature.getParameters();
		if (hasRestParameter(signature) && index >= parameters.length - 1) {
			return this.getTypeOfPropertyOfContextualType(this.getTypeOfSymbol(parameters[parameters.length - 1]), String(index - (parameters.length - 1)));
		}
		return index < parameters.length ? this.getTypeOfSymbol(parameters[index]) : undefined;
	}
	/** tsc's `getContextualType` cases that follow from the syntax and other contextual types. */
	_syntacticContextualType(node) {
		const parent = node.parent;
		if (!parent) return undefined;
		switch (parent.kind) {
			case SyntaxKind.ParenthesizedExpression:
				return this.getContextualType(parent);
			case SyntaxKind.AsExpression:
			case SyntaxKind.TypeAssertionExpression:
				return isConstTypeReference(parent.type) ? this.getContextualType(parent) : this.getTypeFromTypeNode(parent.type);
			case SyntaxKind.SatisfiesExpression:
				return this.getTypeFromTypeNode(parent.type);
			case SyntaxKind.PropertyAssignment:
			case SyntaxKind.ShorthandPropertyAssignment:
				return this.getContextualTypeForObjectLiteralElement(parent);
			case SyntaxKind.ArrayLiteralExpression: {
				const array = this.getContextualType(parent);
				return array && this.getTypeOfPropertyOfContextualType(array, String(parent.elements.indexOf(node)));
			}
			case SyntaxKind.VariableDeclaration:
			case SyntaxKind.PropertyDeclaration:
			case SyntaxKind.Parameter:
				return parent.initializer === node && parent.type ? this.getTypeFromTypeNode(parent.type) : undefined;
			case SyntaxKind.ConditionalExpression:
				return node !== parent.condition ? this.getContextualType(parent) : undefined;
			case SyntaxKind.BinaryExpression: {
				const operator = parent.operatorToken.kind;
				if (operator === SyntaxKind.EqualsToken) return node === parent.right ? this.getTypeAtLocation(parent.left) : undefined;
				if (operator === SyntaxKind.BarBarToken || operator === SyntaxKind.QuestionQuestionToken) return this.getContextualType(parent);
				if (operator === SyntaxKind.AmpersandAmpersandToken || operator === SyntaxKind.CommaToken) {
					return node === parent.right ? this.getContextualType(parent) : undefined;
				}
				return undefined;
			}
		}
		return undefined;
	}
	/**
	 * tsc's `getContextualTypeForObjectLiteralElement`: the property the
	 * element's name names on its object literal's contextual type.
	 */
	getContextualTypeForObjectLiteralElement(element) {
		const literal = element.parent;
		if (!literal || literal.kind !== SyntaxKind.ObjectLiteralExpression || !element.name) return undefined;
		const contextual = this.getContextualType(literal);
		if (!contextual) return undefined;
		const name = getPropertyNameForPropertyNameNode(element.name);
		if (name === undefined) return undefined;
		// A declared member has its symbol's type (a method's, with its
		// declaration); an index signature or tuple element answers otherwise.
		const property = contextual.isUnion() ? undefined : this.getPropertyOfType(contextual, unescapeLeadingUnderscores(name));
		return (property && this.getTypeOfSymbol(property)) ?? this.getTypeOfPropertyOfContextualType(contextual, unescapeLeadingUnderscores(name));
	}
	getNonOptionalType(type) {
		return type._nonOptional ?? type;
	}
	getUnionType(types) {
		return this._type(this._native.unionType(types.map((type) => this._typeId(type))));
	}
	createArrayType(elementType, readonly) {
		return this._type(this._native.arrayType(this._typeId(elementType), !!readonly));
	}
	getStringLiteralType(value) {
		return this._type(this._native.stringLiteralType(value));
	}
	getNumberLiteralType(value) {
		return this._type(this._native.numberLiteralType(String(value)));
	}
	getTrueType() {
		return this._type(this._native.booleanLiteralType(true));
	}
	getFalseType() {
		return this._type(this._native.booleanLiteralType(false));
	}
	isArrayLikeType(type) {
		return this._native.isArrayLikeType(this._typeId(type));
	}
	isUndefinedSymbol(symbol) {
		return this._native.isUndefinedSymbol(this._symbolId(symbol));
	}
	isArgumentsSymbol(symbol) {
		return this._native.isArgumentsSymbol(this._symbolId(symbol));
	}
	/** surge's symbols are merged across declarations already. */
	getMergedSymbol(symbol) {
		return symbol;
	}
	resolveName(name, location, meaning, _excludeGlobals) {
		const [file, index] = location ? this._node(location) : [undefined, undefined];
		return this._symbolOrUndefined(this._native.resolveName(name, file, index, meaning));
	}
	getShorthandAssignmentValueSymbol(node) {
		if (!node || node.kind !== SyntaxKind.ShorthandPropertyAssignment) return undefined;
		const [file, index] = this._node(node);
		return this._symbolOrUndefined(this._native.shorthandAssignmentValueSymbol(file, index));
	}
	/**
	 * `getExportSpecifierLocalTargetSymbol`: what `export { x }` exports (the
	 * local `x`), or the module member a re-export names.
	 */
	getExportSpecifierLocalTargetSymbol(node) {
		const meaning = SymbolFlags.Value | SymbolFlags.Type | SymbolFlags.Namespace | SymbolFlags.Alias;
		if (node.kind !== SyntaxKind.ExportSpecifier) return this.resolveName(node.text, node, meaning, false);
		if (node.parent.parent.moduleSpecifier) {
			const alias = node.symbol;
			return alias && this.getImmediateAliasedSymbol(alias);
		}
		const local = node.propertyName ?? node.name;
		return this.resolveName(local.text, local, meaning, false);
	}
	resolveExternalModuleName(moduleSpecifier) {
		return this.getSymbolAtLocation(moduleSpecifier);
	}
	/**
	 * The type-only `import`/`export` declaration an alias, or an alias it
	 * leads to, was declared through (tsc's `typeOnlyDeclaration`).
	 */
	getTypeOnlyAliasDeclaration(symbol) {
		const seen = new Set();
		for (let current = symbol; current && current.flags & SymbolFlags.Alias && !seen.has(current); current = this.getImmediateAliasedSymbol(current)) {
			seen.add(current);
			const declaration = current.declarations?.find(isTypeOnlyImportOrExportDeclaration);
			if (declaration) return declaration;
		}
		return undefined;
	}
	/** tsc's `getTypeOfPropertyOfContextualType`, over each union constituent. */
	getTypeOfPropertyOfContextualType(type, name) {
		const results = [];
		for (const member of type.isUnion() ? type.types : [type]) {
			if (!(member.flags & (TypeFlags.Object | TypeFlags.Intersection))) continue;
			const property = this.getPropertyOfType(member, name);
			if (property) {
				results.push(this.getTypeOfSymbol(property));
				continue;
			}
			const numeric = /^(?:0|[1-9]\d*)$/.test(name);
			const index = (numeric && this.getIndexTypeOfType(member, 1)) || this.getIndexTypeOfType(member, 0);
			if (index) results.push(index);
		}
		return results.length > 0 ? this.getUnionType(results) : undefined;
	}
	/**
	 * `getConstantValue`: an enum member's value; for an access, a member of
	 * a `const` enum's, as tsc inlines it.
	 */
	getConstantValue(node) {
		if (node.kind === SyntaxKind.EnumMember) return this._enumMemberValue(node);
		const name =
			node.kind === SyntaxKind.PropertyAccessExpression ? node.name : node.kind === SyntaxKind.ElementAccessExpression ? node.argumentExpression : node;
		const symbol = this._enumMemberOfElementAccess(node) ?? this.getSymbolAtLocation(name);
		const member = symbol && symbol.flags & SymbolFlags.EnumMember ? symbol.valueDeclaration : undefined;
		return member && hasSyntacticModifier(member.parent, ModifierFlags.Const) ? this._enumMemberValue(member) : undefined;
	}
	/** `E["member"]`: the member of the enum `E` names. */
	_enumMemberOfElementAccess(node) {
		if (node.kind !== SyntaxKind.ElementAccessExpression || !isStringLiteralLike(node.argumentExpression)) return undefined;
		let owner = this.getSymbolAtLocation(node.expression);
		if (owner && owner.flags & SymbolFlags.Alias) owner = this.getAliasedSymbol(owner);
		if (!owner || !(owner.flags & SymbolFlags.Enum)) return undefined;
		return owner.exports?.get(escapeLeadingUnderscores(node.argumentExpression.text));
	}
	/** tsc's `computeEnumMemberValue`: the initializer evaluated, or the previous member's value plus one. */
	_enumMemberValue(member, visiting = new Set()) {
		if (visiting.has(member)) return undefined;
		visiting.add(member);
		try {
			if (member.initializer) return this._evaluate(member.initializer, visiting);
			const declaration = member.parent;
			if ((declaration.flags & NodeFlags.Ambient || hasSyntacticModifier(declaration, ModifierFlags.Ambient)) && !hasSyntacticModifier(declaration, ModifierFlags.Const)) {
				return undefined;
			}
			const index = declaration.members.indexOf(member);
			if (index === 0) return 0;
			const previous = this._enumMemberValue(declaration.members[index - 1], visiting);
			return typeof previous === "number" ? previous + 1 : undefined;
		} finally {
			visiting.delete(member);
		}
	}
	/** tsc's constant evaluator over an enum initializer. */
	_evaluate(expression, visiting) {
		switch (expression.kind) {
			case SyntaxKind.NumericLiteral:
				return Number(expression.text);
			case SyntaxKind.StringLiteral:
			case SyntaxKind.NoSubstitutionTemplateLiteral:
				return expression.text;
			case SyntaxKind.TemplateExpression: {
				let text = expression.head.text;
				for (const span of expression.templateSpans) {
					const value = this._evaluate(span.expression, visiting);
					if (value === undefined) return undefined;
					text += value + span.literal.text;
				}
				return text;
			}
			case SyntaxKind.ParenthesizedExpression:
				return this._evaluate(expression.expression, visiting);
			case SyntaxKind.PrefixUnaryExpression: {
				const value = this._evaluate(expression.operand, visiting);
				if (typeof value !== "number") return undefined;
				switch (expression.operator) {
					case SyntaxKind.PlusToken:
						return value;
					case SyntaxKind.MinusToken:
						return -value;
					case SyntaxKind.TildeToken:
						return ~value;
				}
				return undefined;
			}
			case SyntaxKind.BinaryExpression: {
				const left = this._evaluate(expression.left, visiting);
				const right = this._evaluate(expression.right, visiting);
				if (left === undefined || right === undefined) return undefined;
				const operator = expression.operatorToken.kind;
				if (operator === SyntaxKind.PlusToken && (typeof left === "string" || typeof right === "string")) return `${left}${right}`;
				if (typeof left !== "number" || typeof right !== "number") return undefined;
				switch (operator) {
					case SyntaxKind.PlusToken:
						return left + right;
					case SyntaxKind.MinusToken:
						return left - right;
					case SyntaxKind.AsteriskToken:
						return left * right;
					case SyntaxKind.SlashToken:
						return left / right;
					case SyntaxKind.PercentToken:
						return left % right;
					case SyntaxKind.AsteriskAsteriskToken:
						return left ** right;
					case SyntaxKind.LessThanLessThanToken:
						return left << right;
					case SyntaxKind.GreaterThanGreaterThanToken:
						return left >> right;
					case SyntaxKind.GreaterThanGreaterThanGreaterThanToken:
						return left >>> right;
					case SyntaxKind.AmpersandToken:
						return left & right;
					case SyntaxKind.BarToken:
						return left | right;
					case SyntaxKind.CaretToken:
						return left ^ right;
				}
				return undefined;
			}
			case SyntaxKind.Identifier:
			case SyntaxKind.PropertyAccessExpression:
			case SyntaxKind.ElementAccessExpression: {
				const name =
					expression.kind === SyntaxKind.PropertyAccessExpression
						? expression.name
						: expression.kind === SyntaxKind.ElementAccessExpression
							? expression.argumentExpression
							: expression;
				if (expression.kind === SyntaxKind.Identifier && expression.text === "Infinity") return Number.POSITIVE_INFINITY;
				if (expression.kind === SyntaxKind.Identifier && expression.text === "NaN") return Number.NaN;
				const symbol = this.getSymbolAtLocation(name);
				const declaration = symbol?.valueDeclaration;
				if (!declaration) return undefined;
				if (declaration.kind === SyntaxKind.EnumMember) return this._enumMemberValue(declaration, visiting);
				// tsc's `isConstantVariable`: a `const` with a literal initializer.
				if (declaration.kind === SyntaxKind.VariableDeclaration && declaration.parent.flags & NodeFlags.Const && declaration.initializer && !declaration.type) {
					return this._evaluate(declaration.initializer, visiting);
				}
				return undefined;
			}
		}
		return undefined;
	}
	getEmitResolver() {
		this._emitResolver ??= new EmitResolver(this);
		return this._emitResolver;
	}
	getTypeOfSymbolAtLocation(symbol, node) {
		// tsc's `checkIdentifier`: `arguments` read in a property initializer
		// or class static block is the error type.
		if (symbol.name === "arguments" && isInPropertyInitializerOrClassStaticBlock(node) && this.isArgumentsSymbol(symbol)) return this.getAnyType();
		const [file, index] = this._node(node);
		return this._type(this._native.typeOfSymbolAtLocation(this._symbolId(symbol), file, index));
	}
	getTypeOfSymbol(symbol) {
		return this._type(this._native.typeOfSymbol(this._symbolId(symbol)));
	}
	getDeclaredTypeOfSymbol(symbol) {
		return this._type(this._native.declaredTypeOfSymbol(this._symbolId(symbol)));
	}
	getPropertiesOfType(type) {
		return this._native.propertiesOfType(this._typeId(type)).map((id) => this._symbol(id));
	}
	getPropertyOfType(type, name) {
		return this._symbolOrUndefined(this._native.propertyOfType(this._typeId(type), name));
	}
	getSignaturesOfType(type, kind) {
		const construct = kind === SignatureKind.Construct;
		return this._native.signaturesOfType(this._typeId(type), construct).map((id) => this._signature(id));
	}
	getIndexTypeOfType(type, kind) {
		return this._typeOrUndefined(this._native.indexTypeOfType(this._typeId(type), kind === 1));
	}
	getReturnTypeOfSignature(signature) {
		if (signature instanceof InstantiatedSignature) return signature.getReturnType();
		return this._type(this._native.returnTypeOfSignature(this._signatureId(signature)));
	}
	getExportsOfModule(moduleSymbol) {
		return this._native.exportsOfModule(this._symbolId(moduleSymbol)).map((id) => this._symbol(id));
	}
	getAliasedSymbol(symbol) {
		return this._symbolOrUndefined(this._native.aliasedSymbol(this._symbolId(symbol))) ?? symbol;
	}
	getImmediateAliasedSymbol(symbol) {
		return this._symbolOrUndefined(this._native.immediateAliasedSymbol(this._symbolId(symbol)));
	}
	getApparentType(type) {
		return this._type(this._native.apparentType(this._typeId(type)));
	}
	getBaseConstraintOfType(type) {
		return this._typeOrUndefined(this._native.baseConstraintOfType(this._typeId(type)));
	}
	getNonNullableType(type) {
		return this._type(this._native.nonNullableType(this._typeId(type)));
	}
	getBaseTypeOfLiteralType(type) {
		return this._type(this._native.baseTypeOfLiteralType(this._typeId(type)));
	}
	getWidenedType(type) {
		return this._type(this._native.widenedType(this._typeId(type)));
	}
	isArrayType(type) {
		return this._native.isArrayType(this._typeId(type));
	}
	isTupleType(type) {
		return this._native.isTupleType(this._typeId(type));
	}
	getTypeArguments(type) {
		return this._native.typeArguments(this._typeId(type)).map((id) => this._type(id));
	}
	isTypeAssignableTo(source, target) {
		return this._native.isTypeAssignableTo(this._typeId(source), this._typeId(target));
	}
	typeToString(type) {
		return this._native.typeToString(this._typeId(type));
	}
	symbolToString(symbol) {
		return this._native.symbolToString(this._symbolId(symbol));
	}
	/** `signatureToString`: `new ` leads only when `kind` says the signature constructs. */
	signatureToString(signature, _enclosingDeclaration, _flags, kind) {
		const text = signature instanceof InstantiatedSignature ? signature._toString() : this._native.signatureToString(this._signatureId(signature));
		return kind === SignatureKind.Construct ? `new ${text}` : text;
	}
	getFullyQualifiedName(symbol) {
		const names = [];
		for (let current = symbol; current !== undefined; current = current.parent) {
			if (current.flags & SymbolFlags.ValueModule && current.name.startsWith('"')) {
				names.unshift(current.name);
				break;
			}
			names.unshift(current.name);
		}
		return names.join(".");
	}
	getAnyType() {
		return this._type(this._native.intrinsicType("any"));
	}
	/** tsc's `errorType`: an `any` that stands for a failure. */
	_errorType() {
		return this._type(this._native.intrinsicType("error"));
	}
	getUnknownType() {
		return this._type(this._native.intrinsicType("unknown"));
	}
	getStringType() {
		return this._type(this._native.intrinsicType("string"));
	}
	getNumberType() {
		return this._type(this._native.intrinsicType("number"));
	}
	getBooleanType() {
		return this._type(this._native.intrinsicType("boolean"));
	}
	getBigIntType() {
		return this._type(this._native.intrinsicType("bigint"));
	}
	getESSymbolType() {
		return this._type(this._native.intrinsicType("symbol"));
	}
	getUndefinedType() {
		return this._type(this._native.intrinsicType("undefined"));
	}
	getNullType() {
		return this._type(this._native.intrinsicType("null"));
	}
	getVoidType() {
		return this._type(this._native.intrinsicType("void"));
	}
	getNeverType() {
		return this._type(this._native.intrinsicType("never"));
	}
	getTypeFromTypeNode(node) {
		return this.getTypeAtLocation(node);
	}
	/**
	 * `getBaseTypes`: a class's `extends` base, then the `extends` of each
	 * interface declaration merged with it, in declaration order.
	 */
	getBaseTypes(type) {
		const declarations = type.symbol?.declarations ?? [];
		const bases = [];
		const add = (base) => {
			if (base && !(base.flags & (TypeFlags.Any | TypeFlags.Unknown)) && !bases.includes(base)) bases.push(base);
		};
		for (const declaration of declarations) {
			if (declaration.kind !== SyntaxKind.ClassDeclaration && declaration.kind !== SyntaxKind.ClassExpression) continue;
			const clause = declaration.heritageClauses?.find((heritage) => heritage.token === SyntaxKind.ExtendsKeyword);
			if (clause?.types.length) add(this.getTypeAtLocation(clause.types[0]));
		}
		for (const declaration of declarations) {
			if (declaration.kind !== SyntaxKind.InterfaceDeclaration) continue;
			for (const clause of declaration.heritageClauses ?? []) {
				for (const node of clause.types) add(this.getTypeFromTypeNode(node));
			}
		}
		return bases;
	}
	getTypeOfPropertyOfType(type, name) {
		const property = this.getPropertyOfType(type, name);
		return property && this.getTypeOfSymbol(property);
	}
}

// Methods of TypeScript's TypeChecker surge-ts does not answer yet. Each
// throws, naming itself, rather than being absent or answering a guess.
for (const method of [
	"getAmbientModules",
	"getAugmentedPropertiesOfType",
	"getAwaitedType",
	"getExportSymbolOfSymbol",
	"getIndexInfoOfType",
	"getIndexInfosOfType",
	"getJsxIntrinsicTagNamesAt",
	"getNullableType",
	"getPromisedTypeOfPromise",
	"getRootSymbols",
	"getSignatureFromDeclaration",
	"getSymbolsInScope",
	"getTypePredicateOfSignature",
	"isUnknownSymbol",
	"isValidPropertyAccess",
	"signatureToSignatureDeclaration",
	"symbolToEntityName",
	"typeToTypeNode",
]) {
	TypeChecker.prototype[method] = function () {
		throw new Error(`surge-ts: TypeChecker.${method} is not supported yet`);
	};
}

module.exports = { TypeChecker, TypeObject, SymbolObject, SignatureObject };
