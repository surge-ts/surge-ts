// Declarations for the TypeScript-internal API surge-ts provides: helpers
// TypeScript's own declarations leave out, which index.d.ts re-exports
// (scripts/api/generate-enums.ts reads this file), and the internal members
// of TypeScript's interfaces. Signatures follow TypeScript's source.
import ts = require("./typescript");

// --- core
export function identity<T>(value: T): T;
export function returnTrue(): true;
export function returnFalse(): false;
export function returnUndefined(): undefined;
export function noop(..._: unknown[]): void;
export function arrayFrom<T>(iterator: Iterable<T>): T[];
export function arrayFrom<T, U>(iterator: Iterable<T>, map: (value: T) => U): U[];

// --- modifiers
export function modifierToFlag(token: ts.SyntaxKind): ts.ModifierFlags;
export function modifiersToFlags(modifiers: readonly ts.ModifierLike[] | undefined): ts.ModifierFlags;
export function getSyntacticModifierFlags(node: ts.Node): ts.ModifierFlags;
export function getSyntacticModifierFlagsNoCache(node: ts.Node): ts.ModifierFlags;
/** TypeScript adds JSDoc modifiers in a JavaScript file; surge-ts reads the written modifiers only. */
export function getEffectiveModifierFlags(node: ts.Node): ts.ModifierFlags;
export function hasSyntacticModifier(node: ts.Node, flags: ts.ModifierFlags): boolean;
export function hasEffectiveModifier(node: ts.Node, flags: ts.ModifierFlags): boolean;
export function hasStaticModifier(node: ts.Node): boolean;
export function hasAbstractModifier(node: ts.Node): boolean;
export function hasAccessorModifier(node: ts.Node): boolean;
export function hasDecorators(node: ts.Node): boolean;

// --- syntax
export function isFunctionLikeKind(kind: ts.SyntaxKind): boolean;
export function isFunctionLikeDeclaration(node: ts.Node | undefined): node is ts.FunctionLikeDeclaration;
export function isFunctionBody(node: ts.Node): node is ts.FunctionBody;
export function isNodeKind(kind: ts.SyntaxKind): boolean;
export function isStringOrNumericLiteralLike(node: ts.Node): node is ts.StringLiteralLike | ts.NumericLiteral;
export function isSignedNumericLiteral(node: ts.Node): node is ts.PrefixUnaryExpression & { operator: ts.SyntaxKind.PlusToken | ts.SyntaxKind.MinusToken; operand: ts.NumericLiteral };
export function isAnyImportSyntax(node: ts.Node): node is ts.ImportDeclaration | ts.ImportEqualsDeclaration;
export function isImportCall(node: ts.Node): node is ts.ImportCall;
export function isSuperCall(node: ts.Node): node is ts.SuperCall;
export function isSuperProperty(node: ts.Node): node is ts.SuperProperty;
export function isThisIdentifier(node: ts.Node | undefined): boolean;
export function isJsonSourceFile(file: ts.SourceFile): file is ts.JsonSourceFile;
export function positionIsSynthesized(pos: number): boolean;
export function skipParentheses(node: ts.Expression): ts.Expression;
export function isEntityNameExpression(node: ts.Node): node is ts.EntityNameExpression;
export function isPropertyAccessEntityNameExpression(node: ts.Node): node is ts.PropertyAccessEntityNameExpression;
export function isBindableStaticAccessExpression(node: ts.Node, excludeThisKeyword?: boolean): boolean;
export function isPrototypeAccess(node: ts.Node): boolean;
export function isAssignmentOperator(token: ts.SyntaxKind): boolean;
export function isAssignmentExpression(node: ts.Node, excludeCompoundAssignment?: boolean): node is ts.AssignmentExpression<ts.AssignmentOperatorToken>;
export function isLogicalOrCoalescingAssignmentOperator(token: ts.SyntaxKind): token is ts.LogicalOrCoalescingAssignmentOperator;
export function isLogicalOrCoalescingAssignmentExpression(node: ts.Node): node is ts.AssignmentExpression<ts.Token<ts.LogicalOrCoalescingAssignmentOperator>>;
export function isAssignmentTarget(node: ts.Node): boolean;
/** 0 none, 1 definite, 2 compound. */
export function getAssignmentTargetKind(node: ts.Node): 0 | 1 | 2;
export function isWriteAccess(node: ts.Node): boolean;
export function isWriteOnlyAccess(node: ts.Node): boolean;
export function isNodeDescendantOf(node: ts.Node | undefined, ancestor: ts.Node | undefined): boolean;
export function getThisContainer(node: ts.Node, includeArrowFunctions: boolean, includeClassComputedPropertyName: boolean): ts.Node;
export function getInterfaceBaseTypeNodes(node: ts.InterfaceDeclaration): ts.NodeArray<ts.ExpressionWithTypeArguments> | undefined;
export function getEffectiveBaseTypeNode(node: ts.ClassLikeDeclaration | ts.InterfaceDeclaration): ts.ExpressionWithTypeArguments | undefined;
export function getEffectiveImplementsTypeNodes(node: ts.ClassLikeDeclaration): readonly ts.ExpressionWithTypeArguments[] | undefined;
export function getAllSuperTypeNodes(node: ts.Node): readonly ts.TypeNode[];
export function getTextOfJsxNamespacedName(node: ts.JsxNamespacedName): string;
export function getEscapedTextOfJsxNamespacedName(node: ts.JsxNamespacedName): ts.__String;
export function getPropertyNameForPropertyNameNode(name: ts.PropertyName | ts.JsxAttributeName): ts.__String | undefined;
export function skipAlias(symbol: ts.Symbol, checker: ts.TypeChecker): ts.Symbol;
export function isInstantiatedModule(node: ts.ModuleDeclaration, preserveConstEnums: boolean): boolean;

// --- scanner
export function utf16EncodeAsString(codePoint: number): string;

// --- compiler options
export function getEmitDeclarations(options: ts.CompilerOptions): boolean;
export function isIncrementalCompilation(options: ts.CompilerOptions): boolean;
export function isSolutionConfig(config: ts.ParsedCommandLine): boolean;

// --- paths
export const directorySeparator: "/";
export function isAnyDirectorySeparator(charCode: number): boolean;
export function getRootLength(path: string): number;
export function isRootedDiskPath(path: string): boolean;
export function normalizeSlashes(path: string): string;
export function hasTrailingDirectorySeparator(path: string): boolean;
export function ensureTrailingDirectorySeparator(path: ts.Path): ts.Path;
export function ensureTrailingDirectorySeparator(path: string): string;
export function removeTrailingDirectorySeparator(path: ts.Path): ts.Path;
export function removeTrailingDirectorySeparator(path: string): string;
export function combinePaths(path: string, ...paths: (string | undefined)[]): string;
export function getNormalizedAbsolutePath(path: string, currentDirectory: string | undefined): string;
export function normalizePath(path: string): string;
export function resolvePath(path: string, ...paths: (string | undefined)[]): string;
export function toPath(fileName: string, basePath: string | undefined, getCanonicalFileName: (path: string) => string): ts.Path;
export function toFileNameLowerCase(fileName: string): string;
export function createGetCanonicalFileName(useCaseSensitiveFileNames: boolean): (fileName: string) => string;
export function getDirectoryPath(path: ts.Path): ts.Path;
export function getDirectoryPath(path: string): string;
export function getBaseFileName(path: string): string;
export function getPathComponents(path: string, currentDirectory?: string): string[];
export function reducePathComponents(components: readonly string[]): string[];
export function getPathFromPathComponents(components: readonly string[], length?: number): string;
export function getRelativePathFromDirectory(fromDirectory: string, to: string, getCanonicalFileNameOrIgnoreCase: ((fileName: string) => string) | boolean): string;
export function fileExtensionIs(path: string, extension: string): boolean;
export function isDeclarationFileName(fileName: string): boolean;
export function removeFileExtension(path: string): string;
export function isInsideNodeModules(fileOrDirectory: string): boolean;
export function forEachAncestorDirectory<T>(directory: string, callback: (directory: string) => T | undefined): T | undefined;
export function findPackageJson(directory: string, host: Pick<ts.ModuleResolutionHost, "fileExists"> & { getGlobalTypingsCacheLocation?(): string | undefined }): string | undefined;

// --- diagnostics
export const Diagnostics: { readonly [name: string]: ts.DiagnosticMessage };
export function getLocaleSpecificMessage(message: ts.DiagnosticMessage): string;
export function formatStringFromArgs(text: string, args: readonly (string | number)[]): string;
export function formatMessage(message: ts.DiagnosticMessage, ...args: (string | number | undefined)[]): string;
export function createCompilerDiagnostic(message: ts.DiagnosticMessage, ...args: (string | number | undefined)[]): ts.Diagnostic;
export function createFileDiagnostic(
	file: ts.SourceFile,
	start: number,
	length: number,
	message: ts.DiagnosticMessage,
	...args: (string | number | undefined)[]
): ts.DiagnosticWithLocation;

// --- the language service's reference search
export namespace FindAllReferences {
	namespace Core {
		function eachSymbolReferenceInFile<T>(
			definition: ts.Identifier,
			checker: ts.TypeChecker,
			sourceFile: ts.SourceFile,
			cb: (token: ts.Identifier) => T,
			searchContainer?: ts.Node,
		): T | undefined;
		function isSymbolReferencedInFile(definition: ts.Identifier, checker: ts.TypeChecker, sourceFile: ts.SourceFile, searchContainer?: ts.Node): boolean;
	}
}

// --- source files, nodes, programs and the checker
export enum CommentDirectiveType {
	ExpectError = 0,
	Ignore = 1,
}

declare module "./typescript" {
	interface CommentDirective {
		range: TextRange;
		type: CommentDirectiveType;
	}
	interface PragmaArguments {
		readonly [name: string]: string | { value: string; pos: number; end: number } | undefined;
	}
	interface PragmaValue {
		arguments: PragmaArguments;
		range: CommentRange;
	}
	interface Node {
		/** The binder's symbol of a declaration (a source file's: its module's). */
		readonly symbol: Symbol;
	}
	interface SourceFile {
		path: Path;
		resolvedPath: Path;
		originalFileName: string;
		/** `@ts-expect-error` and `@ts-ignore` comments; `undefined` when there are none. */
		commentDirectives?: CommentDirective[];
		/** The file's leading `/// <reference>`, `@ts-nocheck`, `@jsx`, … pragmas. */
		pragmas: ReadonlyMap<string, PragmaValue | PragmaValue[]>;
	}
	interface SymlinkCache {
		getSymlinkedDirectoriesByRealpath(): ReadonlyMap<Path, readonly string[]> | undefined;
	}
	interface Program {
		getCommonSourceDirectory(): string;
		getSymlinkCache(): SymlinkCache;
	}
	interface EmitResolver {
		isReferencedAliasDeclaration(node: Node, checkChildren?: boolean): boolean;
		getJsxFactoryEntity(location?: Node): EntityName;
		getJsxFragmentFactoryEntity(location?: Node): EntityName | undefined;
	}
	interface TypeChecker {
		/** The type without the `undefined` an optional chain adds to a link inside it. */
		getNonOptionalType(type: Type): Type;
		getUnionType(types: Type[]): Type;
		createArrayType(elementType: Type, readonly?: boolean): Type;
		isArrayLikeType(type: Type): boolean;
		getTypeOfPropertyOfContextualType(type: Type, name: string): Type | undefined;
		getTypeOnlyAliasDeclaration(symbol: Symbol): TypeOnlyAliasDeclaration | undefined;
		resolveExternalModuleName(moduleSpecifier: Expression): Symbol | undefined;
		getEmitResolver(sourceFile?: SourceFile): EmitResolver;
	}
}
