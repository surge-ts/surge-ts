/// A parse failure, kept in the shape tsc reports rather than flattened to a
/// rendered string. oxc already computes a TS code and a span for the
/// TypeScript-specific failures (`ts_error` in its `diagnostics` module), and
/// both are needed to report one the way tsc does.
#[derive(Debug, Clone, PartialEq)]
pub struct ParserError {
    /// The TypeScript error number when oxc classified the failure as one
    /// (`TS1172` -> `1172`); `None` for a generic parse failure.
    pub code: Option<u32>,
    /// oxc's own rendering, used when the code is unknown or uncatalogued.
    pub message: String,
    pub span: Option<TextSpan>,
    /// The source text under `span`: the modifier a modifier error names.
    pub span_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedSource {
    pub file_name: String,
    pub statements: Vec<ParsedStatement>,
    pub parser_errors: Vec<ParserError>,
    /// oxc gave up at a fatal error, so `statements` holds nothing of the file.
    pub parse_aborted: bool,
    /// A JavaScript file's JSDoc reparse errors (tsgo's
    /// `checkNonIdentifierName`): parse errors tsc's parser port cannot see,
    /// since it does not read JSDoc.
    pub jsdoc_parse_errors: Vec<ParserError>,
    pub is_module: bool,
    /// A JavaScript file that is a CommonJS module (tsc's
    /// `CommonJSModuleIndicator`), which it is only without any ECMAScript
    /// module syntax (`ExternalModuleIndicator`).
    pub commonjs_module: bool,
    /// Leading `/// <reference types="..." />` directives, in source order.
    pub reference_type_directives: Vec<ReferenceTypeDirective>,
    /// Every value- and type-position identifier name referenced anywhere in the
    /// module (including export specifiers, collected from the full oxc AST).
    pub module_reads: Vec<String>,
    /// The names JSDoc `{@link X}` tags refer to, which tsc resolves as uses.
    pub jsdoc_link_names: Vec<String>,
    /// Every name the file writes by definite assignment, sorted (tsc's
    /// `AssignmentKindDefinite`; see `parser::writes`). Backs TS2454's
    /// never-initialized rule for a `let` read from a nested function.
    pub definite_writes: Vec<String>,
    /// Where each `let` tsc may type by control flow is assigned, sorted by
    /// the binding's name position (see [`LetAssignmentSummary`]).
    pub let_assignments: Vec<LetAssignmentSummary>,
    /// The file's `@ts-expect-error`/`@ts-ignore` directives in source order.
    /// Diagnostics starting on a directive's suppressed line are dropped, and
    /// an `@ts-expect-error` that dropped nothing is TS2578, matching tsc.
    pub comment_directives: Vec<CommentDirective>,
    /// Module specifiers written as `import("...")` — type-position import
    /// types and dynamic import expressions — deduplicated in source order.
    /// They belong to the module graph exactly like declaration specifiers do,
    /// but the lossy `Parsed*` tree does not model either form.
    pub import_call_specifiers: Vec<String>,
    /// Every `import("...")` with a literal specifier, in source order and not
    /// deduplicated: tsc resolves each one as it checks it and reports an
    /// unresolved module at that literal.
    pub import_calls: Vec<ParsedImportCall>,
    /// Every top-level import or export declaration's `with { … }` clause,
    /// in source order (tsc's `checkImportAttributes`).
    pub import_attributes: Vec<ParsedImportAttributes>,
    /// Grammar-level findings collected in one walk of the full oxc AST (see
    /// `parser::grammar`): a `const` with no initializer, a duplicate
    /// object-literal key, an overload group with no implementation, a member
    /// with no annotation. Empty for declaration and non-TypeScript files,
    /// which surge does not report on.
    pub grammar_diagnostics: Vec<ParsedGrammarDiagnostic>,
    /// Every parenthesized expression, sorted by the span of the expression it
    /// wraps. The `Parsed*` tree drops the parentheses, but tsc reports on the
    /// parenthesized node — `(x) * 1` is the unnamed `Object is possibly
    /// 'undefined'`, anchored at the `(`. Collected with `grammar_diagnostics`.
    pub parenthesized_expressions: Vec<ParenthesizedExpressionSpan>,
    /// Where each `this` whose container is the file's top level starts —
    /// arrow functions looked through, as tsc's `getThisContainer` does. It is
    /// `globalThis` in a script and `undefined` in a module. Collected with
    /// `grammar_diagnostics`.
    pub global_this_starts: Vec<u32>,
    /// Where each object-literal member whose `this` is the literal itself
    /// starts (its function's span): a method, accessor or `key: function`
    /// value that reads `this` and has no `this` parameter, in a literal with
    /// no contextual type — tsc's `getContextualThisParameterType` types that
    /// `this` as the literal. Sorted. Collected with `grammar_diagnostics`.
    pub literal_this_members: Vec<u32>,
    /// For a `.json` file: the type of the value it holds, and the marker that
    /// this *is* a JSON module. Nothing in a JSON file is code, so it is never
    /// parsed as TypeScript and `statements` is empty; this carries its whole
    /// meaning. `None` for every other file. A `.json` file whose contents do
    /// not parse is still a module, with the degradation sentinel for a value.
    pub json_module_type: Option<ParsedType>,
    /// See [`JsxFactoryUses`].
    pub jsx_factory_uses: JsxFactoryUses,
}

/// One [`ParsedSource::import_attributes`] entry.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedImportAttributes {
    /// The clause, `with` keyword included: where tsc reports it.
    pub span: TextSpan,
    pub attributes: Vec<(String, ParsedExpression)>,
    /// `import type`/`export type` as a whole.
    pub type_only: bool,
}

/// One [`ParsedSource::import_calls`] entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedImportCall {
    pub specifier: String,
    /// The string literal, quotes included: where tsc reports the module.
    pub specifier_span: TextSpan,
    pub kind: ParsedImportCallKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedImportCallKind {
    /// A dynamic `import("m")` (`checkImportCallExpression`).
    Expression,
    /// An import type, `import("m").T` or `typeof import("m")`
    /// (`getTypeFromImportTypeNode`), written without import attributes.
    Type,
    /// A JavaScript `require("m")` the lowering does not turn into an import
    /// declaration (one nested in a function or an expression).
    Require,
}

/// What a file's JSX refers to implicitly (tsc's `markJsxAliasReferenced`):
/// whether it has elements and fragments, and what its leading `@jsx`,
/// `@jsxFrag`, `@jsxImportSource` and `@jsxRuntime` pragmas say.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JsxFactoryUses {
    pub has_elements: bool,
    pub has_fragments: bool,
    /// The file's first element, or first fragment's opening tag, where tsc
    /// reports a JSX runtime import that resolves to nothing.
    pub first_tag: Option<TextSpan>,
    /// The first identifier of the `@jsx` factory.
    pub factory_pragma: Option<String>,
    /// The first identifier of the `@jsxFrag` factory.
    pub fragment_pragma: Option<String>,
    /// The `@jsxImportSource` package.
    pub import_source_pragma: Option<String>,
    pub runtime_pragma: Option<String>,
}

/// How a flow-typed `let` (un-annotated, initialized with nothing, `undefined`,
/// `null` or `[]`) is assigned across its declaring function — tsc's
/// `markNodeAssignments`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LetAssignmentSummary {
    pub name_start: u32,
    /// The last write's position, extended to the end of the statement that
    /// contains it (`extendAssignmentPosition`); `u32::MAX` when a nested
    /// function writes it, `None` when nothing does.
    pub last_assignment: Option<u32>,
    /// Whether anything assigns it definitely (`=`, a logical assignment, a
    /// destructuring or `for…in`/`for…of` target).
    pub definitely_assigned: bool,
}

/// One [`ParsedSource::parenthesized_expressions`] entry: the outermost
/// parentheses around the expression spanning `inner`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParenthesizedExpressionSpan {
    pub inner: TextSpan,
    pub outer: TextSpan,
}

/// One [`ParsedSource::grammar_diagnostics`] finding: what was wrong, where,
/// and the name the message needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedGrammarDiagnostic {
    pub kind: ParsedGrammarDiagnosticKind,
    pub span: TextSpan,
    /// The member/function name for the kinds whose message names one.
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedGrammarDiagnosticKind {
    /// `const x;` outside an ambient context — TS1155.
    ConstNotInitialized,
    /// `await` inside a function that is not `async` — TS1308.
    AwaitOutsideAsyncFunction,
    /// `export { … }` inside a namespace, or `export … from "m"` inside any
    /// namespace — TS1194.
    ExportDeclarationInNamespace,
    /// `delete someBinding` in strict-mode code — TS1102. Only a *direct*
    /// reference to a binding is a syntax error; `delete o.p` is the legal form.
    DeleteOnIdentifierInStrictMode,
    /// An enum member initializer naming a member declared after it — TS2651.
    EnumForwardReference,
    /// An enum member with no initializer after one whose value is not a
    /// number — TS1061.
    EnumMemberInitializerRequired,
    /// A `const enum` member initializer that is not a constant expression —
    /// TS2474.
    ConstEnumInitializerNotConstant,
    /// An ambient enum member initializer that is not a constant expression —
    /// TS1066.
    AmbientEnumInitializerNotConstant,
    /// A second property of the same name in one object literal — TS1117.
    DuplicateObjectLiteralProperty,
    /// An overload group with no implementation — TS2391.
    FunctionImplementationMissing,
    /// A class whose constructor signatures have no implementation — TS2390.
    ConstructorImplementationMissing,
    /// More than one `export default` in a module — TS2528.
    MultipleDefaultExports,
    /// A property with neither annotation nor initializer — TS7008, reported
    /// only under `noImplicitAny`.
    ImplicitAnyMember,
    /// A signature with neither a body nor a return type — TS7010, reported
    /// only under `noImplicitAny`.
    ImplicitAnyReturn,
    /// A JSDoc `@overload` without `@returns` — TS7012, under `noImplicitAny`.
    ImplicitAnyOverloadReturn,
    /// A construct signature without a return type — TS7013, under
    /// `noImplicitAny`.
    ImplicitAnyConstructReturn,
    /// A call signature without a return type — TS7020, under `noImplicitAny`.
    ImplicitAnyCallReturn,
    /// A type-level signature's parameter with neither annotation nor
    /// initializer — TS7006, under `noImplicitAny`.
    ImplicitAnySignatureParameter,
    /// A call signature, method signature or function type's parameter with
    /// neither annotation nor initializer, where tsc asks whether the name is
    /// a type keyword or a type in scope (`(string) => void`) — TS7051 under
    /// `noImplicitAny`, otherwise the plain implicit `any`. `name` holds the
    /// written name, the `argN` tsc suggests, and `[]` for a rest parameter,
    /// NUL-separated.
    NamedSignatureParameterWithoutType,
    /// A module-level type literal or interface member whose computed name is
    /// a bare identifier that no top-level value in the file declares, which
    /// tsc resolves as a value (`{ [Keys]: string }`). The checker answers it
    /// once the file's types are installed: TS2693, or TS2690 when the member
    /// is a type literal's only one. `name` holds the identifier and `1` when
    /// the member is a type literal's only property, NUL-separated.
    ComputedTypeMemberName,
    /// An index signature whose key type is written as a bare type name
    /// (`[key: Name]`) where the file's top-level scope resolves it. tsc
    /// resolves the key (`getTypeFromTypeNode`) before it asks whether it is
    /// a valid index key type; the checker answers it once the file's types
    /// are installed: TS1268 at the parameter name, beside whatever the
    /// reference itself reports. `name` holds the type name and the
    /// reference's start and end offsets, NUL-separated.
    IndexSignatureKeyReference,
    /// An `async` function whose written return type is a type reference:
    /// TS1064 unless it names the global `Promise`, which only the file's
    /// type declarations can tell. `name` holds the reference's name and the
    /// written text of its first type argument, `\0`-separated.
    AsyncReturnTypeReference,
    /// Statements after a `switch` with no `default` whose every case leaves
    /// it: unreachable (TS7027) when the checker finds the cases exhaustive
    /// (`isReachableFlowNode` over the implicit default clause). `name` holds
    /// the `switch`'s start and end offsets, `\0`-separated.
    UnreachableAfterExhaustiveSwitch,
    /// A rest parameter annotated with a plain type reference: TS2370 when
    /// the name is a class or interface that is no array, which only the
    /// file's type declarations can tell. `name` holds the reference.
    RestParameterTypeReference,
    /// Two members of one class, interface, or object literal declaring the
    /// same name where neither is an overload of the other — TS2300.
    DuplicateMember,
    /// Two implementations of the same method — TS2393.
    DuplicateImplementation,
    /// Two constructor implementations — TS2392.
    MultipleConstructorImplementations,
    /// A derived class constructor with no `super` call — TS2377.
    MissingSuperCall,
    /// A member overriding a base class member of another kind — TS2610
    /// (accessor overridden by property), TS2611 (property by accessor),
    /// TS2423 (method by accessor), TS2425 (property by method), TS2426
    /// (accessor by method). `name` holds the member, base and derived class
    /// names separated by NULs.
    PropertyAccessorOverride,
    AccessorPropertyOverride,
    MethodAccessorOverride,
    PropertyMethodOverride,
    AccessorMethodOverride,
    /// `this` read in a derived constructor before `super()` — TS17009.
    ThisBeforeSuperCall,
    /// `super.x` read in a derived constructor before `super()` — TS17011.
    SuperPropertyBeforeSuperCall,
    /// A comma operator whose left side is discarded and cannot have an
    /// effect — TS2695.
    UnusedCommaOperand,
    /// A tested expression whose syntax makes it always truthy — TS2872.
    AlwaysTruthyExpression,
    /// A tested expression whose syntax makes it always falsy — TS2873.
    AlwaysFalsyExpression,
    /// A `??` left operand whose syntax is never nullish — TS2869.
    NeverNullishCoalesceOperand,
    /// A `??` left operand whose syntax is always nullish — TS2871.
    AlwaysNullishCoalesceOperand,
    /// An `async` function whose written return type is not `Promise<T>` —
    /// TS1064. `name` holds the written type, which the message wraps.
    AsyncReturnTypeNotPromise,
    /// A type alias whose resolution reaches itself before any deferred
    /// position — TS2456. `name` holds the alias.
    CircularTypeAlias,
    /// A type parameter without a default after one with a default — TS2706.
    RequiredTypeParameterAfterOptional,
    /// A parameter written both optional and with a default — TS1015.
    OptionalParameterWithInitializer,
    /// A required parameter after an optional one — TS1016.
    RequiredParameterAfterOptional,
    /// An initializer in an ambient context — TS1039.
    AmbientInitializer,
    /// An ambient `const` (or `readonly` property) initializer that is not a
    /// literal or enum reference — TS1254.
    AmbientConstInitializer,
    /// A `set` accessor whose parameter list is not exactly one — TS1049.
    SetAccessorParameterCount,
    /// A `set` accessor with a return type annotation — TS1095.
    SetAccessorReturnType,
    /// A `get` accessor whose body can complete without returning — TS2378.
    GetAccessorWithoutReturn,
    /// A `get` accessor less accessible than its `set` accessor — TS2808,
    /// reported on both.
    GetAccessorLessAccessible,
    /// A `get`/`set` pair of which only one is `abstract` — TS2676, reported on
    /// both.
    AccessorAbstractMismatch,
    /// One name written in an object literal as both an accessor and a plain
    /// property — TS1119.
    ObjectLiteralPropertyAndAccessor,
    /// An `abstract` method in a class that is not abstract — TS1244.
    AbstractMethodOutsideAbstractClass,
    /// An `abstract` property in a class that is not abstract — TS1253.
    AbstractPropertyOutsideAbstractClass,
    /// A parameter default on a signature with no body — TS2371.
    ParameterInitializerOutsideImplementation,
    /// A grammar error identified by its TypeScript number alone; `name`
    /// holds the message arguments separated by NULs.
    Ts(u32),
    /// A [`Self::Ts`] error that holds only under `strictNullChecks`.
    TsUnderStrictNullChecks(u32),
    /// A [`Self::Ts`] error that holds only without `strictNullChecks`.
    TsWithoutStrictNullChecks(u32),
    /// A [`Self::Ts`] error that holds only under `experimentalDecorators`.
    TsUnderLegacyDecorators(u32),
    /// A [`Self::Ts`] error that holds only without `experimentalDecorators`.
    TsUnderEsDecorators(u32),
    /// A [`Self::Ts`] error that holds only when class fields are not emitted
    /// as standard fields (`GetEmitStandardClassFields` is false).
    TsUnlessStandardClassFields(u32),
}

/// A leading `/// <reference types="..." />` directive, or the `path` form
/// (whose `resolution_mode` means nothing). `lib` references are not collected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceTypeDirective {
    /// The referenced type-package specifier, e.g. `node` or `@scope/pkg`, or
    /// the referenced file's path.
    pub value: String,
    /// Byte span of the value inside its quotes, where the directive's
    /// resolution errors (TS2688, TS6053) are reported.
    pub value_span: TextSpan,
    /// A valid `resolution-mode` attribute.
    pub resolution_mode: Option<ResolutionModeOverride>,
}

/// A `resolution-mode` value: the package face a reference resolves against
/// in place of its file's own mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResolutionModeOverride {
    Import,
    Require,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParsedStatement {
    VariableDeclaration(Box<ParsedVariableDeclaration>),
    Assignment(Box<ParsedAssignment>),
    /// A module-scope write through a member or element access
    /// (`Component.displayName = "X"`, `registry[key] = value`). Checked with
    /// the same code a function body uses, over a scope stack rooted at the
    /// module's symbol table.
    MemberAssignment(Box<ParsedMemberAssignment>),
    FunctionDeclaration(Box<ParsedFunctionDeclaration>),
    Call(Box<ParsedCall>),
    Expression(Box<ParsedExpression>),
    TypeAliasDeclaration(Box<ParsedTypeAliasDeclaration>),
    InterfaceDeclaration(Box<ParsedInterfaceDeclaration>),
    ClassDeclaration(Box<ParsedClassDeclaration>),
    ImportDeclaration(Box<ParsedImportDeclaration>),
    ExportDeclaration(Box<ParsedExportDeclaration>),
    DeclareModuleDeclaration(Box<ParsedDeclareModuleDeclaration>),
    /// An identifier-named namespace/module block such as `declare namespace JSX { ... }`.
    /// String-named `declare module "pkg"` blocks use [`ParsedDeclareModuleDeclaration`].
    NamespaceDeclaration(Box<ParsedNamespaceDeclaration>),
    /// A module-scope `if`. Its branches are function-body statements, the
    /// same lowering a function body uses, so the checker can decide whether a
    /// branch leaves the module (`if (isCancel(x)) process.exit(0)`) and narrow
    /// the statements that follow.
    If(Box<ParsedIfStatement>),
    /// A module-scope loop, block, `switch`, `try` or labelled statement,
    /// lowered as a function body lowers it so its statements are checked the
    /// same way.
    Block(Vec<ParsedFunctionBodyStatement>),
    UnsupportedDeclaration {
        span: Option<TextSpan>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedDeclareModuleDeclaration {
    pub module_specifier: String,
    pub module_specifier_span: Option<TextSpan>,
    pub statements: Vec<ParsedStatement>,
    pub span: Option<TextSpan>,
    /// `declare module "x";`, with no body: every import of the module is
    /// `any` (tsc's shorthand ambient module).
    pub is_shorthand: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedNamespaceDeclaration {
    /// The namespace identifier, e.g. `JSX`. Nested names (`A.B`) are joined with `.`.
    pub name: String,
    /// Written `declare namespace`: every declaration in the body is ambient,
    /// and its members are exported without `export`.
    pub is_declare: bool,
    pub name_span: Option<TextSpan>,
    pub statements: Vec<ParsedStatement>,
    pub span: Option<TextSpan>,
}

impl ParsedNamespaceDeclaration {
    /// The name this namespace has as a member of the namespace it is nested
    /// in: `namespace A.B {}` parses as `A` holding a namespace already named
    /// `A.B`, whose member name in `A` is `B`.
    pub fn member_name(&self) -> &str {
        self.name.rsplit_once('.').map_or(&self.name, |(_, last)| last)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedVariableKind {
    Var,
    Let,
    Const,
}

/// The name an import type (`import("m").A.B`) is bound under in the
/// importing file: the specifier, double-quoted, and the qualifier.
pub fn import_type_name(specifier: &str, members: &[String]) -> String {
    let mut name = format!("import(\"{specifier}\")");
    for member in members {
        name.push('.');
        name.push_str(member);
    }
    name
}

/// The specifier and qualifier of an [`import_type_name`].
pub fn split_import_type_name(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix("import(\"")?;
    let (specifier, qualifier) = rest.split_once("\")")?;
    Some((specifier, qualifier.strip_prefix('.').unwrap_or(qualifier)))
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParsedType {
    String,
    Number,
    Boolean,
    BigInt,
    Symbol,
    Undefined,
    Null,
    Void,
    Any,
    /// A type the source itself does not resolve (an unresolved name, or a
    /// name imported from a module that does not resolve). tsc models this as
    /// `errorType`; see [`surge_ts_types::Type::ErrorType`].
    ErrorType,
    Unknown,
    /// The genuine `unknown` keyword, kept distinct from [`ParsedType::Unknown`]
    /// (which doubles as surge's conservative degrade target for `intrinsic`
    /// and unparseable annotations). Only this
    /// variant lowers to [`Type::GenuineUnknown`], so the checker can emit
    /// `TS18046` on a genuinely-`unknown`-typed receiver without flagging a
    /// merely-degraded one.
    UnknownKeyword,
    Never,
    StringLiteral(String),
    NumberLiteral(String),
    BooleanLiteral(bool),
    Object(std::sync::Arc<ParsedObjectType>),
    Array(std::sync::Arc<ParsedType>),
    Tuple(std::sync::Arc<Vec<ParsedType>>),
    /// A tuple carrying a spread element (`[a, ...b]`, `[...a, ...b]`,
    /// `[infer head, ...infer tail]`). Kept distinct from [`ParsedType::Tuple`]
    /// because its length is only known once the spread operands resolve: a
    /// spread of a concrete tuple flattens into a fixed one, and anything else
    /// falls back to the length-less array lowering.
    VariadicTuple(std::sync::Arc<Vec<ParsedTupleElement>>),
    /// `readonly T[]` / `readonly [A, B]`. The operand is an array or tuple
    /// shape; the modifier survives so a readonly array is not assignable to a
    /// mutable one and the two are not identical.
    Readonly(std::sync::Arc<ParsedType>),
    Union(std::sync::Arc<Vec<ParsedType>>),
    Intersection(std::sync::Arc<Vec<ParsedType>>),
    Function(std::sync::Arc<ParsedFunctionType>),
    Named(std::sync::Arc<ParsedNamedType>),
    TypeOf(std::sync::Arc<ParsedTypeOfType>),
    KeyOf(std::sync::Arc<ParsedType>),
    IndexedAccess(std::sync::Arc<ParsedIndexedAccessType>),
    Mapped(std::sync::Arc<ParsedMappedType>),
    Conditional(std::sync::Arc<ParsedConditionalType>),
    TemplateLiteral(std::sync::Arc<ParsedTemplateLiteralType>),
    /// An `infer X` capture inside a conditional type's `extends` clause. Modelled
    /// so a conditional that uses it (e.g. React's `ComponentProps<T>`) survives
    /// parsing instead of degrading the whole conditional to `Unknown`.
    Infer(std::sync::Arc<ParsedInferType>),
    /// A type-predicate return annotation (`x is T`, `this is T`, `asserts x`,
    /// `asserts x is T`). Resolves to `boolean` in type position; the guard
    /// narrowing consumes the payload to narrow the tested argument.
    Predicate(std::sync::Arc<ParsedPredicateType>),
    /// An unannotated class member whose type tsc reads from an expression: a
    /// property's initializer or a getter's body. The checker resolves it on
    /// first read, with `this` bound to the class instance.
    InferredMember(std::sync::Arc<ParsedInferredMember>),
    /// `const name: unique symbol`: a symbol type of its own, distinct from every
    /// other unique symbol (`$input` is not `$output`). Only a variable
    /// declaration's annotation lowers to it; `unique symbol` elsewhere is
    /// `symbol`.
    UniqueSymbol(std::sync::Arc<str>),
}

/// The source of a [`ParsedType::InferredMember`].
#[derive(Debug)]
pub struct ParsedInferredMember {
    /// The declaring class; its instance is what `this` means in the source.
    pub class_name: String,
    /// Where the member is declared, which identifies it: two members compare
    /// equal exactly when they are the same declaration.
    pub member_start: usize,
    pub member_name: String,
    /// A `readonly` property keeps its initializer's literal type.
    pub keep_literal: bool,
    pub source: ParsedInferredMemberSource,
}

#[derive(Debug)]
pub enum ParsedInferredMemberSource {
    Initializer(ParsedExpression),
    GetterBody(Vec<ParsedFunctionBodyStatement>),
    ThisAssignments(ParsedThisAssignments),
    /// An unannotated method's return, read from its body.
    MethodBody(ParsedInferredMethodBody),
}

#[derive(Debug)]
pub struct ParsedInferredMethodBody {
    pub parameters: Vec<ParsedFunctionParameter>,
    /// Each parameter's type as a caller sees it.
    pub parameter_types: Vec<ParsedType>,
    pub body: Vec<ParsedFunctionBodyStatement>,
    /// A static method's `this` is the class itself.
    pub is_static: bool,
    pub is_async: bool,
    pub this_parameter_type: Option<ParsedType>,
}

impl PartialEq for ParsedInferredMember {
    fn eq(&self, other: &Self) -> bool {
        self.member_start == other.member_start
            && self.member_name == other.member_name
            && self.class_name == other.class_name
    }
}

impl Eq for ParsedInferredMember {}

/// One element of a [`ParsedType::VariadicTuple`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedTupleElement {
    Fixed(ParsedType),
    /// `...T` — spreads every element of another tuple or array type. The
    /// span is the written element, where a non-array operand is reported
    /// (TS2574).
    Rest(ParsedType, Option<TextSpan>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPredicateType {
    /// The tested parameter's name (`"this"` for a `this is T` predicate).
    pub parameter_name: String,
    /// `None` for a bare `asserts x` assertion with no type.
    pub ty: Option<ParsedType>,
    /// Where `ty` is written, for a predicate type its parameter does not
    /// admit (TS2677).
    pub type_span: Option<TextSpan>,
    pub asserts: bool,
}

impl Clone for ParsedType {
    fn clone(&self) -> Self {
        crate::clone_census::record_parsed_type_clone(self.census_variant());
        match self {
            Self::String => Self::String,
            Self::Number => Self::Number,
            Self::Boolean => Self::Boolean,
            Self::BigInt => Self::BigInt,
            Self::Symbol => Self::Symbol,
            Self::Undefined => Self::Undefined,
            Self::Null => Self::Null,
            Self::Void => Self::Void,
            Self::Any => Self::Any,
            Self::Unknown => Self::Unknown,
            Self::ErrorType => Self::ErrorType,
            Self::UnknownKeyword => Self::UnknownKeyword,
            Self::Never => Self::Never,
            Self::StringLiteral(value) => Self::StringLiteral(value.clone()),
            Self::NumberLiteral(value) => Self::NumberLiteral(value.clone()),
            Self::BooleanLiteral(value) => Self::BooleanLiteral(*value),
            Self::Object(payload) => Self::Object(payload.clone()),
            Self::Array(payload) => Self::Array(payload.clone()),
            Self::Tuple(payload) => Self::Tuple(payload.clone()),
            Self::VariadicTuple(payload) => Self::VariadicTuple(payload.clone()),
            Self::Readonly(payload) => Self::Readonly(payload.clone()),
            Self::Union(payload) => Self::Union(payload.clone()),
            Self::Intersection(payload) => Self::Intersection(payload.clone()),
            Self::Function(payload) => Self::Function(payload.clone()),
            Self::Named(payload) => Self::Named(payload.clone()),
            Self::TypeOf(payload) => Self::TypeOf(payload.clone()),
            Self::KeyOf(payload) => Self::KeyOf(payload.clone()),
            Self::IndexedAccess(payload) => Self::IndexedAccess(payload.clone()),
            Self::Mapped(payload) => Self::Mapped(payload.clone()),
            Self::Conditional(payload) => Self::Conditional(payload.clone()),
            Self::TemplateLiteral(payload) => Self::TemplateLiteral(payload.clone()),
            Self::Infer(payload) => Self::Infer(payload.clone()),
            Self::Predicate(payload) => Self::Predicate(payload.clone()),
            Self::InferredMember(payload) => Self::InferredMember(payload.clone()),
            Self::UniqueSymbol(name) => Self::UniqueSymbol(name.clone()),
        }
    }
}

impl ParsedType {
    fn census_variant(&self) -> usize {
        match self {
            Self::String
            | Self::Number
            | Self::Boolean
            | Self::BigInt
            | Self::Symbol
            | Self::Undefined
            | Self::Null
            | Self::Void
            | Self::Any
            | Self::ErrorType
            | Self::Unknown
            | Self::UnknownKeyword
            | Self::Never
            | Self::BooleanLiteral(_) => 0,
            Self::StringLiteral(_) => 1,
            Self::NumberLiteral(_) => 2,
            Self::Object(_) => 3,
            Self::Array(_) | Self::KeyOf(_) | Self::Readonly(_) => 4,
            Self::Tuple(_)
            | Self::VariadicTuple(_)
            | Self::Union(_)
            | Self::Intersection(_) => 5,
            Self::Function(_) => 6,
            Self::Named(_) => 7,
            Self::TypeOf(_) => 8,
            Self::IndexedAccess(_) => 9,
            Self::Mapped(_) | Self::Conditional(_) => 10,
            Self::TemplateLiteral(_)
            | Self::Infer(_)
            | Self::Predicate(_)
            | Self::InferredMember(_)
            | Self::UniqueSymbol(_) => 11,
        }
    }
}

/// A template literal type in type position, e.g. `` `/${Entity}/${Action}` ``.
///
/// `quasis` holds the literal string segments and always has exactly one more
/// element than `interpolations` (the head, the text between each
/// interpolation, and the tail). `interpolations[i]` sits between `quasis[i]`
/// and `quasis[i + 1]`. A template with no interpolations (`` `hello` ``) has a
/// single quasi and no interpolations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTemplateLiteralType {
    pub quasis: Vec<String>,
    pub interpolations: Vec<ParsedType>,
    pub span: Option<TextSpan>,
}

/// `infer X` or `infer X extends C`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedInferType {
    pub name: String,
    pub constraint: Option<ParsedType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedConditionalType {
    pub check_type: Box<ParsedType>,
    pub extends_type: Box<ParsedType>,
    pub true_type: Box<ParsedType>,
    pub false_type: Box<ParsedType>,
    pub span: Option<TextSpan>,
}

/// A mapped type's optionality modifier. `-?` makes every mapped property
/// required even when the homomorphic source property is optional, so the
/// three states cannot collapse to a bool: `Keep` inherits the source's
/// optionality, `Add` forces optional, `Remove` forces required.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappedOptionality {
    Keep,
    Add,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedMappedType {
    pub key_name: String,
    pub key_span: Option<TextSpan>,
    pub constraint: Box<ParsedType>,
    pub value_type: Box<ParsedType>,
    pub optional: MappedOptionality,
    /// The `readonly` modifier, with the same three states: `readonly` adds it,
    /// `-readonly` removes it, and no modifier keeps the source property's.
    pub readonly: MappedOptionality,
    /// The `as` clause (`[K in keyof T as Rename<K>]`): each key is mapped
    /// through it, `never` drops the key, a union of literals fans it out.
    pub name_type: Option<Box<ParsedType>>,
    pub span: Option<TextSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedIndexedAccessType {
    pub object_type: Box<ParsedType>,
    pub index_type: Box<ParsedType>,
    pub span: Option<TextSpan>,
    /// Where tsc reports a key the object cannot be indexed by.
    pub index_span: Option<TextSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTypeOfType {
    pub name: String,
    pub name_span: Option<TextSpan>,
    /// Dotted member path following the base `name` for qualified queries such
    /// as `typeof NS.Root` (`members == ["Root"]`). Empty for a plain `typeof x`.
    pub members: Vec<String>,
    /// `typeof import("spec")`: the module specifier the query reads the
    /// namespace value of. `name` then carries the rendered `import("spec")`
    /// and `members` the qualifier written after it.
    pub import_specifier: Option<String>,
    /// Where each of `members` is written, when the parser kept it (the
    /// qualifier of `typeof import("m").a.b`).
    pub member_spans: Vec<TextSpan>,
    /// The instantiation expression's arguments: `typeof f<A, B>` binds the
    /// generic value's type parameters in type position, exactly as a call with
    /// explicit type arguments does. Empty for a plain `typeof f`. Dropping
    /// these left `ReturnType<typeof withTRPC<TRouter, TSSRContext>>` — the one
    /// member that degrades tRPC's exported client — at the sentinel.
    pub type_arguments: Vec<ParsedType>,
    /// The written argument list, `<` through `>`.
    pub type_arguments_span: Option<TextSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTypeParameter {
    pub name: String,
    pub name_span: Option<TextSpan>,
    pub constraint: Option<ParsedType>,
    pub default_type: Option<ParsedType>,
    pub span: Option<TextSpan>,
    /// `<const T>`: an argument inferred for it keeps its const-context shape
    /// (an array literal is a tuple).
    pub is_const: bool,
    /// `<in T>`, `<out T>`, `<in out T>`: the variance the relation takes as
    /// written instead of measuring it (`getVariancesWorker`).
    pub is_in: bool,
    pub is_out: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFunctionType {
    pub parameters: Vec<ParsedFunctionTypeParameter>,
    pub return_type: Box<ParsedType>,
    pub type_parameters: Vec<ParsedTypeParameter>,
}

/// One step from a destructured parameter's type to a name it binds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedBindingStep {
    Property(String),
    Index(usize),
    ObjectRest,
    ArrayRest,
}

/// A name a destructuring pattern binds, with the path that reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedBoundName {
    pub name: String,
    pub path: Vec<ParsedBindingStep>,
    /// The last step carries a default, so the binding is never `undefined`.
    pub has_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFunctionTypeParameter {
    pub name: Option<String>,
    pub name_span: Option<TextSpan>,
    pub ty: ParsedType,
    pub optional: bool,
    /// A `this: T` fake parameter. It carries typing metadata but is not a real
    /// call parameter, so it is excluded from arity and argument matching.
    pub is_this: bool,
    /// A `this` parameter typed `this` in a class or interface member, whose
    /// `ty` stands in for the polymorphic `this` with the declaring type.
    pub polymorphic_this: bool,
    /// A `...rest: T[]` parameter. Its annotation is the array type; lowering
    /// stores the element type and marks the signature variadic.
    pub rest: bool,
    /// The names a destructured parameter binds; empty for an identifier.
    pub bound_names: Vec<ParsedBoundName>,
}

impl ParsedBindingName {
    /// Every name this pattern binds, with the path from the bound value.
    pub fn bound_names(&self) -> Vec<ParsedBoundName> {
        fn collect(
            binding: &ParsedBindingName,
            path: &mut Vec<ParsedBindingStep>,
            has_default: bool,
            out: &mut Vec<ParsedBoundName>,
        ) {
            match binding {
                ParsedBindingName::Identifier { name, .. } => out.push(ParsedBoundName {
                    name: name.clone(),
                    path: path.clone(),
                    has_default,
                }),
                ParsedBindingName::ObjectPattern(pattern) => {
                    for element in &pattern.elements {
                        path.push(ParsedBindingStep::Property(element.property_name.clone()));
                        collect(&element.binding_name, path, element.has_default, out);
                        path.pop();
                    }
                    if let Some(rest) = &pattern.rest {
                        path.push(ParsedBindingStep::ObjectRest);
                        collect(rest, path, false, out);
                        path.pop();
                    }
                }
                ParsedBindingName::ArrayPattern(pattern) => {
                    for (index, element) in pattern.elements.iter().enumerate() {
                        if let Some(element) = element {
                            path.push(ParsedBindingStep::Index(index));
                            collect(element, path, false, out);
                            path.pop();
                        }
                    }
                    if let Some(rest) = &pattern.rest {
                        path.push(ParsedBindingStep::ArrayRest);
                        collect(rest, path, false, out);
                        path.pop();
                    }
                }
                ParsedBindingName::Unsupported { .. } => {}
            }
        }
        let mut out = Vec::new();
        collect(self, &mut Vec::new(), false, &mut out);
        out
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParsedBindingName {
    Identifier {
        name: String,
        span: Option<TextSpan>,
    },
    ObjectPattern(ParsedObjectBindingPattern),
    ArrayPattern(ParsedArrayBindingPattern),
    Unsupported {
        span: Option<TextSpan>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedArrayBindingPattern {
    /// Each position binds the corresponding element; `None` is an elision hole
    /// (`[, b]`). The element type is the source tuple element at that index, or
    /// the array element type for a non-tuple source.
    pub elements: Vec<Option<ParsedBindingName>>,
    /// Whether each position carries a default (`[a = 1]`), parallel to
    /// `elements`.
    pub defaults: Vec<bool>,
    /// Each position's default with its span, parallel to `elements`.
    pub default_values: Vec<Option<(ParsedExpression, Option<TextSpan>)>>,
    /// The `...rest` binding of `[a, ...rest]`, if present.
    pub rest: Option<Box<ParsedBindingName>>,
    pub span: Option<TextSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedObjectBindingPattern {
    pub elements: Vec<ParsedObjectBindingElement>,
    /// The `...rest` binding of an object destructuring pattern, if present.
    /// `{ a, ...rest }` binds `rest` to the remaining properties.
    pub rest: Option<Box<ParsedBindingName>>,
    pub span: Option<TextSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedObjectBindingElement {
    /// Empty for a [`Self::computed_key`] element.
    pub property_name: String,
    /// `{ [key]: name }` with a key that is not a literal name: the element
    /// reads whatever the key indexes, which names no property surge knows.
    pub computed_key: bool,
    /// The key expression of a [`Self::computed_key`] element, which is
    /// evaluated for its own diagnostics.
    pub key_expression: Option<Box<(ParsedExpression, Option<TextSpan>)>>,
    /// `{ a }` rather than `{ a: a }`. Unused-binding reporting keys on it:
    /// tsc exempts an `_`-prefixed local only when it renames a property.
    pub shorthand: bool,
    pub binding_name: ParsedBindingName,
    pub name_span: Option<TextSpan>,
    pub has_default: bool,
    /// The initializer of `{ a = value }`, checked against the property's type.
    pub default_value: Option<Box<ParsedExpression>>,
    pub default_span: Option<TextSpan>,
    pub span: Option<TextSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedNamedType {
    pub type_argument_spans: Vec<TextSpan>,
    pub name: String,
    pub span: Option<TextSpan>,
    pub type_arguments: Vec<ParsedType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTypeAliasDeclaration {
    pub is_declare: bool,
    pub name: String,
    pub name_span: Option<TextSpan>,
    pub type_parameters: Vec<ParsedTypeParameter>,
    pub ty: ParsedType,
    pub type_span: Option<TextSpan>,
    /// The enum this alias stands for, when it was synthesized by lowering an
    /// `enum` declaration (both the enum's own alias and one per member). An
    /// enum type is nominal in tsc and displayed by the enum's name, which the
    /// literal-union body cannot express on its own.
    pub enum_name: Option<String>,
    /// Whether that `enum` was exported. tsc qualifies an exported enum's type
    /// as `import("<module>").Color` and names a file-local one bare.
    pub enum_exported: bool,
    /// Whether that `enum` was a `const enum`, which has no runtime binding to
    /// be used before its declaration.
    pub enum_is_const: bool,
    /// On an enum's own alias: each member's name and lowered type, in
    /// declaration order across merged declarations. tsc relates two enums
    /// member by member (`isEnumTypeRelatedTo`).
    pub enum_members: Vec<(String, ParsedType)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedInterfaceDeclaration {
    pub is_declare: bool,
    pub name: String,
    pub name_span: Option<TextSpan>,
    pub type_parameters: Vec<ParsedTypeParameter>,
    pub extends: Vec<ParsedNamedType>,
    pub members: Vec<ParsedInterfaceMember>,
    /// Value type of a string/number index signature (`[key: string]: T`), if
    /// present. The key type is not modelled separately; both string and number
    /// index signatures map here.
    pub string_index_type: Option<ParsedType>,
    /// See [`ParsedObjectType::number_index_type`].
    pub number_index_type: Option<ParsedType>,
    /// Where the string and number index signatures above are declared, for
    /// the diagnostics tsc reports on the signature itself.
    pub string_index_span: Option<TextSpan>,
    pub number_index_span: Option<TextSpan>,
    /// See [`ParsedObjectType::string_index_readonly`].
    pub string_index_readonly: bool,
    pub number_index_readonly: bool,
    /// A bare call signature (`(value?: any): number`) on the interface, making
    /// values of this type callable without `new` (e.g. `NumberConstructor`).
    pub call_signature: Option<ParsedFunctionType>,
    /// Every call signature as written, in source order, kept only when the
    /// interface declares more than one. `call_signature` above stays the
    /// permissive fold; this list is what overload resolution picks a candidate
    /// from, the way a function declaration's overload group does.
    pub call_signature_overloads: Vec<ParsedFunctionType>,
    /// Construct signatures (`new <T>(executor): Promise<T>`) on the interface,
    /// making values of this type usable with `new` (e.g. `PromiseConstructor`,
    /// `SetConstructor`). One entry per overload; the resolver merges them into a
    /// single permissive signature. Each carries its own `type_parameters`.
    pub construct_signatures: Vec<ParsedFunctionType>,
    /// See [`ParsedKeyedIndexes`].
    pub keyed_indexes: Option<std::sync::Arc<ParsedKeyedIndexes>>,
}

/// The index signatures keyed by a template literal pattern or a branded
/// string, one per key constituent (`getIndexInfosOfIndexSymbol`). The string
/// slot carries their first value as a stand-in; these decide which keys it
/// really answers. Only recorded when no `string` keyword key is declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedKeyedIndexes {
    pub signatures: Vec<ParsedKeyedIndexSignature>,
    /// The value of the first `[k: symbol]: T` signature.
    pub symbol: Option<ParsedType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedKeyedIndexSignature {
    pub key: ParsedType,
    pub value: ParsedType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedInterfaceMember {
    pub name: String,
    pub name_span: Option<TextSpan>,
    pub optional: bool,
    pub is_abstract: bool,
    /// Declared with method syntax (`m(): T`). tsc checks such a member's
    /// parameters bivariantly even under `strictFunctionTypes`.
    pub is_method: bool,
    pub ty: ParsedType,
    /// Declared `readonly`, or a getter with no matching setter — tsc's
    /// `isReadonlySymbol`, which turns a write into TS2540 before any
    /// assignability check runs.
    pub readonly: bool,
    /// What a *write* to this member is checked against, when that differs from
    /// `ty`. tsc: "Distinct write types come only from set accessors"
    /// (`getWriteTypeOfSymbol`), so this is the setter's parameter type of an
    /// accessor pair whose getter declares something else. `None` everywhere
    /// else, where reading and writing share a type.
    pub write_ty: Option<ParsedType>,
}

/// A `class` declaration. The instance side (fields + methods) is modelled as a
/// type; the constructor/static side is modelled as a value. Unsupported members
/// (getters/setters, index signatures, static blocks) are dropped during parsing
/// rather than causing a fatal parse error.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedClassDeclaration {
    pub is_declare: bool,
    /// Lowered from `const X = class {}`: `X` is a constant, and writing it is
    /// TS2588 rather than a class's TS2629.
    pub const_binding: bool,
    /// `abstract class C {}`. A non-abstract class must implement every
    /// abstract member it inherits, and only an abstract class may declare one.
    pub is_abstract: bool,
    pub name: String,
    pub name_span: Option<TextSpan>,
    pub type_parameters: Vec<ParsedTypeParameter>,
    /// Base classes named in an `extends` clause. A class has at most one, but
    /// this is modelled as a list so the instance side can reuse the interface
    /// heritage-merge path. A base that is not a (dotted) name, e.g. a mixin
    /// call, is named [`EXPRESSION_HERITAGE_BASE`].
    pub extends: Vec<ParsedNamedType>,
    /// The `extends` expression behind an [`EXPRESSION_HERITAGE_BASE`] base,
    /// whose type is the base constructor type (tsc's
    /// `getBaseConstructorTypeOfClass`).
    pub heritage_expression: Option<Box<ParsedExpression>>,
    /// Interfaces named in an `implements` clause. They contribute nothing to
    /// the instance type — the class has to declare the members itself, which
    /// is what makes an unimplemented one reportable.
    pub implements: Vec<ParsedNamedType>,
    pub members: Vec<ParsedClassMember>,
    /// Members declared `private` or `protected`, including constructor
    /// parameter properties. Everything not listed is public.
    pub restricted_members: Vec<ParsedRestrictedMember>,
    /// Value types of the instance side's string and number index signatures
    /// (`[key: string]: T`), as on [`ParsedInterfaceDeclaration`].
    pub string_index_type: Option<ParsedType>,
    pub number_index_type: Option<ParsedType>,
    /// See [`ParsedInterfaceDeclaration::string_index_span`].
    pub string_index_span: Option<TextSpan>,
    pub number_index_span: Option<TextSpan>,
    pub symbol_index_span: Option<TextSpan>,
    /// The static side's index signatures (`static [key: string]: T`), which
    /// the class value answers every other key with.
    pub static_string_index_type: Option<ParsedType>,
    pub static_number_index_type: Option<ParsedType>,
    /// See [`ParsedObjectType::string_index_readonly`], for the instance and
    /// the static side.
    pub string_index_readonly: bool,
    pub number_index_readonly: bool,
    pub static_string_index_readonly: bool,
    pub static_number_index_readonly: bool,
    /// The instance side's `symbol` index signature, the only keyed index a
    /// class carries.
    pub keyed_indexes: Option<std::sync::Arc<ParsedKeyedIndexes>>,
    /// Where the static number index signature is declared, which a conflict
    /// with the static string index is reported on (TS2413).
    pub static_number_index_span: Option<TextSpan>,
    pub span: Option<TextSpan>,
    /// Every member's computed key (`[expr]`), with the bracketed name's span:
    /// each is checked as an expression (tsc's `checkComputedPropertyName`)
    /// whether or not the member it names could be modelled.
    pub computed_keys: Vec<(ParsedExpression, Option<TextSpan>)>,
    /// Beside each of `computed_keys`: the member is `abstract` or `declare`,
    /// where a type-only import is a valid use (tsc's
    /// `isValidTypeOnlyAliasUseSite`).
    pub non_emitting_computed_keys: Vec<bool>,
    /// Beside each of `computed_keys`: for a key whose text mentions `this`,
    /// whether its member is static. tsc types `this` there as the member's
    /// own (`checkThisExpression` past the computed name).
    pub this_computed_keys: Vec<Option<bool>>,
    /// The decorators on the class, its members and their parameters, in
    /// source order.
    pub decorators: Vec<ParsedDecorator>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedClassExpression {
    /// Named as tsc writes an unassigned class (`getNameOfSymbolAsWritten`):
    /// its own name, else `(Anonymous class)`.
    pub class: ParsedClassDeclaration,
    /// The class writes a name of its own, which is bound inside its body.
    pub has_own_name: bool,
}

/// A decorator's expression, and what tsc's `nodeCanBeDecorated` asks of the
/// declaration it decorates (which decides whether it is checked at all).
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedDecorator {
    pub expression: ParsedExpression,
    pub span: Option<TextSpan>,
    pub target: ParsedDecoratorTarget,
    /// The expression is not one a decorator may be written as without
    /// parentheses (tsc's `checkGrammarDecorator`, TS1497).
    pub needs_parentheses: bool,
    /// The whole decorator, `@` included, and the expression as written.
    pub decorator_span: Option<TextSpan>,
    pub expression_text: String,
    pub is_parenthesized: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedDecoratorTarget {
    Class,
    /// A property, or an auto-accessor.
    Property { is_abstract: bool, is_declare: bool, private_name: bool, is_auto_accessor: bool },
    /// A method or a `get`/`set` accessor.
    Method { has_body: bool, private_name: bool },
    /// A parameter of a constructor, method or `set` accessor that has a body.
    Parameter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedMemberAccessibility {
    Private,
    Protected,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedRestrictedMember {
    pub name: String,
    pub is_static: bool,
    pub accessibility: ParsedMemberAccessibility,
    /// Set when only one side of an accessor pair carries the modifier
    /// (`public get x()` beside `private set x(v)`): a read checks the getter's
    /// accessibility and a write the setter's.
    pub accessor_side: Option<ParsedAccessorSide>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedAccessorSide {
    Get,
    Set,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParsedClassMember {
    Property(ParsedClassProperty),
    Method(ParsedClassMethod),
    Accessor(ParsedClassAccessor),
    Constructor(ParsedClassConstructor),
    /// `static { … }`: code run once with `this` bound to the class.
    StaticBlock(ParsedClassStaticBlock),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedClassStaticBlock {
    pub body: Vec<ParsedFunctionBodyStatement>,
    /// See [`ParsedFunctionDeclaration::body_reads`].
    pub body_reads: Vec<String>,
    /// The member's whole source range, which is where a class type
    /// parameter is out of scope when the member is static (TS2302).
    pub span: Option<TextSpan>,
}

/// A `get`/`set` accessor pair, collapsed into a single member keyed by name.
/// Either side may be absent (getter-only or setter-only). The instance side
/// lowers this to a property whose type is the getter return type when present,
/// otherwise the setter parameter type.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedClassAccessor {
    pub name: String,
    pub name_span: Option<TextSpan>,
    pub is_static: bool,
    pub is_override: bool,
    pub is_abstract: bool,
    pub getter_return_type: Option<ParsedType>,
    pub setter_param_type: Option<ParsedType>,
    pub has_getter: bool,
    pub has_setter: bool,
    /// The written `get`/`set` declarations this member merges, each with its
    /// own body to check.
    pub declarations: Vec<ParsedAccessorDeclaration>,
    /// The member's whole source range, which is where a class type
    /// parameter is out of scope when the member is static (TS2302).
    pub span: Option<TextSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedAccessorDeclaration {
    pub is_getter: bool,
    /// A setter's single parameter; empty for a getter.
    pub parameters: Vec<ParsedFunctionParameter>,
    pub return_type_span: Option<TextSpan>,
    pub body: Vec<ParsedFunctionBodyStatement>,
    /// See [`ParsedFunctionDeclaration::body_reads`].
    pub body_reads: Vec<String>,
    /// False for an abstract or ambient accessor.
    pub has_body: bool,
}

/// The heritage name of a class whose `extends` clause is an expression surge
/// does not reduce to a name — `extends mixin(Base)`. tsc takes the base from
/// that expression's construct signatures; the checker resolves this name to
/// `any`, which leaves the instance open instead of closed over the members
/// the class declares itself.
pub const EXPRESSION_HERITAGE_BASE: &str = "\0expression-base";

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedClassProperty {
    pub name: String,
    /// Written with the `accessor` keyword: a get/set pair over a private
    /// slot, which reads as a property but is no class field.
    pub is_auto_accessor: bool,
    pub name_span: Option<TextSpan>,
    /// `"a": T` or `1: T`: a string or numeric literal key, which tsc's
    /// property-initialization check (TS2564) does not look at.
    pub has_literal_name: bool,
    pub is_static: bool,
    pub is_override: bool,
    pub is_abstract: bool,
    /// `declare x: T` — the property is declared elsewhere, so it carries no
    /// initialization obligation of its own.
    pub is_declare: bool,
    /// `x!: T` — asserted to be initialized outside the constructor, which is
    /// what exempts it from the initialization check.
    pub has_definite_assertion: bool,
    pub optional: bool,
    pub readonly: bool,
    pub declared_type: Option<ParsedType>,
    /// The annotation as the initialization check (TS2564) reads it when
    /// polymorphic `this` stands at its top — alone, as a union or
    /// intersection member, or indexed (`this["k"]`). `this` is a type
    /// parameter, never `any` nor `undefined`, but `declared_type` lowers it to
    /// `any`, which would exempt the property; here it reads as `object`.
    pub initialization_type: Option<ParsedType>,
    pub initializer: Option<ParsedExpression>,
    pub initializer_span: Option<TextSpan>,
    /// The member's whole source range, which is where a class type
    /// parameter is out of scope when the member is static (TS2302).
    pub span: Option<TextSpan>,
    /// Set for a JavaScript member declared by `this.x = v` rather than
    /// written in the class body.
    pub this_assignments: Option<ParsedThisAssignments>,
}

/// The values a JavaScript class assigns to a member through `this`, which
/// is how tsc declares it (`bindThisPropertyAssignment`).
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedThisAssignments {
    pub values: Vec<ParsedExpression>,
    /// Whether the values are the constructor's (or a static block's), which
    /// then are the only ones that count (`isConstructorDeclaredThisProperty`).
    pub in_constructor: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedClassMethod {
    /// See [`ParsedFunctionDeclaration::body_reads`].
    pub body_reads: Vec<String>,
    pub name: String,
    pub name_span: Option<TextSpan>,
    pub is_static: bool,
    pub is_override: bool,
    pub is_abstract: bool,
    pub type_parameters: Vec<ParsedTypeParameter>,
    pub parameters: Vec<ParsedFunctionParameter>,
    /// See [`ParsedFunctionDeclaration::this_parameter_type`].
    pub this_parameter_type: Option<ParsedType>,
    /// See [`ParsedFunctionTypeParameter::polymorphic_this`].
    pub this_parameter_polymorphic: bool,
    pub return_type: Option<ParsedType>,
    /// See [`ParsedFunctionDeclaration::return_type_span`]. tsc reports a
    /// missing return on the written return type, falling back to the name.
    pub return_type_span: Option<TextSpan>,
    pub body: Vec<ParsedFunctionBodyStatement>,
    /// See [`ParsedFunctionDeclaration::has_body`].
    pub has_body: bool,
    /// See [`ParsedFunctionDeclaration::is_generator`].
    pub is_generator: bool,
    pub is_async: bool,
    /// The member's whole source range, which is where a class type
    /// parameter is out of scope when the member is static (TS2302).
    pub span: Option<TextSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedClassConstructor {
    /// See [`ParsedFunctionDeclaration::body_reads`].
    pub body_reads: Vec<String>,
    /// `private constructor()` / `protected constructor()`: `new` and
    /// `extends` are allowed only within the class (or, for `protected`, a
    /// class deriving from it).
    pub accessibility: Option<ParsedMemberAccessibility>,
    pub parameters: Vec<ParsedFunctionParameter>,
    pub body: Vec<ParsedFunctionBodyStatement>,
    pub span: Option<TextSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedImportDeclaration {
    pub kind: ParsedImportKind,
    pub module_specifier: String,
    pub module_specifier_span: Option<TextSpan>,
    pub span: Option<TextSpan>,
    pub resolution_mode: Option<ParsedResolutionModeAttribute>,
    /// The `type`-marked specifiers of an import that is not itself
    /// `import type`, split off to bind in type space alone.
    pub inline_type_specifiers: bool,
}

/// A valid `resolution-mode` import attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedResolutionModeAttribute {
    pub mode: ResolutionModeOverride,
    /// The declaration is `import type`/`export type` as a whole, the only
    /// form whose resolution the attribute selects (tsc's
    /// `IsExclusivelyTypeOnlyImportOrExport`).
    pub selects_resolution: bool,
}

impl ParsedResolutionModeAttribute {
    /// The mode the declaration's specifier resolves in, when the attribute
    /// decides it.
    pub fn resolution_override(attribute: Option<Self>) -> Option<ResolutionModeOverride> {
        attribute
            .filter(|attribute| attribute.selects_resolution)
            .map(|attribute| attribute.mode)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParsedImportKind {
    Named {
        is_type_only: bool,
        specifiers: Vec<ParsedImportSpecifier>,
    },
    DefaultAndNamed {
        local_name: String,
        name_span: Option<TextSpan>,
        is_type_only: bool,
        specifiers: Vec<ParsedImportSpecifier>,
    },
    Default {
        local_name: String,
        name_span: Option<TextSpan>,
    },
    /// `import type Foo from "specifier"` — a default import that binds only in
    /// type space. Kept distinct from `Default` so the value side stays unbound
    /// while the name still counts as declared (shadowing, unused-locals).
    TypeOnlyDefault {
        local_name: String,
        name_span: Option<TextSpan>,
    },
    Namespace {
        local_name: String,
        name_span: Option<TextSpan>,
        is_type_only: bool,
    },
    /// `import local = require("specifier")` — declaration-lite CommonJS import
    /// equals against a package/module entrypoint.
    Equals {
        local_name: String,
        name_span: Option<TextSpan>,
        /// `import type local = require("specifier")`.
        is_type_only: bool,
        /// `export import local = require("specifier")`.
        exported: bool,
    },
    /// `import local = N.M` — an alias of an entity name. The parser already
    /// rewrote every reference to it; this records the alias for what names it
    /// without a reference, an export clause (`export { local }`).
    EntityAlias {
        local_name: String,
        name_span: Option<TextSpan>,
        /// The entity path, dotted (`N.M`).
        target: String,
    },
    SideEffect,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedImportSpecifier {
    pub imported_name: String,
    pub local_name: String,
    pub name_span: Option<TextSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParsedExportDeclaration {
    Statement {
        declaration: Box<ParsedStatement>,
        is_type_only: bool,
    },
    Named {
        is_type_only: bool,
        specifiers: Vec<ParsedExportSpecifier>,
        module_specifier: Option<String>,
        module_specifier_span: Option<TextSpan>,
        span: Option<TextSpan>,
        resolution_mode: Option<ParsedResolutionModeAttribute>,
    },
    Default {
        declaration: ParsedDefaultExportDeclaration,
        span: Option<TextSpan>,
    },
    All {
        module_specifier: String,
        module_specifier_span: Option<TextSpan>,
        span: Option<TextSpan>,
        /// `export type * from "x"` re-exports the target's TYPES only. Binding
        /// its values too would invent names the module does not have, masking
        /// the TS2304 a consumer should get.
        is_type_only: bool,
        resolution_mode: Option<ParsedResolutionModeAttribute>,
    },
    Namespace {
        exported_name: String,
        exported_name_span: Option<TextSpan>,
        module_specifier: String,
        module_specifier_span: Option<TextSpan>,
        span: Option<TextSpan>,
        /// `export type * as ns from "x"`.
        is_type_only: bool,
    },
    Empty {
        span: Option<TextSpan>,
    },
    /// `export as namespace Foo` — the UMD global declaration. Names the global
    /// this module is exposed under when loaded via a script tag.
    NamespaceExport {
        exported_name: String,
        exported_name_span: Option<TextSpan>,
        span: Option<TextSpan>,
    },
    /// `export = identifier` — declaration-lite CommonJS export assignment.
    /// Only a bare identifier target is represented here; any other target is
    /// an `EqualsExpression`.
    Equals {
        exported_name: String,
        exported_name_span: Option<TextSpan>,
        span: Option<TextSpan>,
    },
    /// `export = <expression>` with any target but a bare identifier.
    EqualsExpression {
        expression: Box<ParsedExpression>,
        expression_span: Option<TextSpan>,
        /// The dotted name when the target is an entity name (`export = A.B`,
        /// tsc's `isEntityNameExpression`): the export is then an alias of every
        /// meaning the entity has. `None` for any other expression, whose value
        /// is all the module exports.
        entity_name: Option<String>,
        span: Option<TextSpan>,
    },
    Unsupported {
        span: Option<TextSpan>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedExportSpecifier {
    pub local_name: String,
    pub exported_name: String,
    pub name_span: Option<TextSpan>,
    /// The exported name's own span (`z` in `p as z`), where tsc anchors the
    /// duplicate-export diagnostics. `name_span` stays the local name, which is
    /// where the specifier node — and the conflict diagnostics reported on it —
    /// begins.
    pub exported_name_span: Option<TextSpan>,
    pub is_type_only: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParsedDefaultExportDeclaration {
    Function(ParsedFunctionDeclaration),
    Class(ParsedClassDeclaration),
    Expression(ParsedExpression),
    Unsupported { span: Option<TextSpan> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedObjectType {
    pub properties: Vec<ParsedObjectTypeProperty>,
    /// A string/number index signature (`[k: string]: T`).
    pub string_index_type: Option<Box<ParsedType>>,
    /// `[key: number]: T`. Kept apart from the string index: a numeric key
    /// prefers it, and a string key a number-only type cannot answer is an
    /// implicit `any` rather than a resolved member.
    pub number_index_type: Option<Box<ParsedType>>,
    /// Whether the index signatures above are declared `readonly` (tsc's
    /// `IndexInfo.isReadonly`), which refuses a write through them (TS2542).
    pub string_index_readonly: bool,
    pub number_index_readonly: bool,
    /// A bare call signature (`(value?: any): number`) on the object type,
    /// making values of this type callable without `new`.
    pub call_signature: Option<Box<ParsedFunctionType>>,
    /// Every call signature as written, in source order, kept only when the
    /// type literal declares more than one. `call_signature` above stays the
    /// permissive fold every other consumer reads; this list exists for
    /// `infer` capture binding, which must pair the pattern's signatures with
    /// the check type's overloads one for one — the fold widens a slot the
    /// overloads disagree on, and an `infer` capture in that slot is exactly
    /// what gets widened away.
    pub call_signature_overloads: Vec<ParsedFunctionType>,
    /// A construct signature. Carries the lowering of a constructor *type*
    /// (`new (args) => T`, `abstract new (args) => T`), which is an object type
    /// with only this signature — modelled distinctly from a call signature so
    /// a plain function does not satisfy `T extends new (…) => …`.
    pub construct_signature: Option<Box<ParsedFunctionType>>,
    /// Every construct signature as written, in source order, kept only when
    /// the type literal declares more than one — the group a relation takes
    /// member by member, as `call_signature_overloads` is for calls.
    pub construct_signature_overloads: Vec<ParsedFunctionType>,
    /// The `object` keyword: every non-primitive. Its member surface is the
    /// empty object, but a primitive does not satisfy it, which `{}` cannot say.
    pub non_primitive: bool,
    /// The name tsc displays the type by when it is not a written literal —
    /// `typeof E` for an enum's object.
    pub display_name: Option<String>,
    /// The construct signature is `abstract new (…) => T`
    /// (`SignatureFlagsAbstract`).
    pub abstract_construct_signature: bool,
    /// See [`ParsedKeyedIndexes`].
    pub keyed_indexes: Option<std::sync::Arc<ParsedKeyedIndexes>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedObjectTypeProperty {
    pub name: String,
    pub name_span: Option<TextSpan>,
    pub ty: ParsedType,
    pub optional: bool,
    /// See [`ParsedInterfaceMember::is_method`].
    pub is_method: bool,
    /// See [`ParsedInterfaceMember::readonly`].
    pub readonly: bool,
    /// See [`ParsedInterfaceMember::write_ty`].
    pub write_ty: Option<ParsedType>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParsedExpression {
    StringLiteral(String),
    NumberLiteral(String),
    /// `10n`. Typed as `bigint`: surge has no bigint literal type.
    BigIntLiteral(String),
    BooleanLiteral(bool),
    UndefinedLiteral,
    NullLiteral,
    Identifier {
        name: String,
        span: Option<TextSpan>,
    },
    /// The `this` keyword. Resolves to the enclosing class instance (in instance
    /// methods/constructors) or static side (in static methods); elsewhere it is
    /// typed conservatively rather than reported as an unresolved identifier.
    This {
        span: Option<TextSpan>,
    },
    ObjectLiteral {
        properties: Vec<ParsedObjectProperty>,
        span: Option<TextSpan>,
    },
    ArrayLiteral {
        elements: Vec<ParsedArrayElement>,
        span: Option<TextSpan>,
        /// Contextually typed by a tuple-like type the parser can see — the
        /// type a destructuring pattern implies for its initializer — so tsc's
        /// `checkArrayLiteral` types the literal as a tuple (`inTupleContext`).
        tuple_context: bool,
    },
    /// A template literal (`` `a${x}b` ``).
    TemplateLiteral {
        expressions: Vec<ParsedExpression>,
        span: Option<TextSpan>,
        /// The cooked text around the interpolations (`None` for an invalid
        /// escape), one more than `expressions`.
        quasis: Vec<Option<String>>,
        /// Where each interpolation is written, in `expressions` order.
        expression_spans: Vec<Option<TextSpan>>,
    },
    Unary {
        operator: ParsedUnaryOperator,
        operator_span: Option<TextSpan>,
        operand: Box<ParsedExpression>,
        operand_span: Option<TextSpan>,
    },
    /// `x++` / `x--` / `++x` / `--x`. All four forms check their operand the
    /// same way and produce the same result type, so neither the operator nor
    /// its position is recorded.
    Update {
        operand: Box<ParsedExpression>,
        operand_span: Option<TextSpan>,
        /// The operand is no reference (`++1`, `f()--`): oxc kept it past its
        /// write-target error, which the checker answers as tsc does — TS2357
        /// (TS2777 for an optional chain) once the operand passes the
        /// arithmetic check.
        rejected_target: bool,
    },
    /// `await x`. The operand is kept so the checker can unwrap the awaited
    /// type; erasing the `await` at parse time left every `await` expression
    /// typed as the promise itself.
    Await {
        operand: Box<ParsedExpression>,
        operand_span: Option<TextSpan>,
    },
    /// `yield x` / `yield* x` / `yield`. The operand is checked against the
    /// yield type of an annotated generator (`checkYieldExpression`).
    Yield {
        operand: Option<Box<ParsedExpression>>,
        operand_span: Option<TextSpan>,
        delegate: bool,
        span: Option<TextSpan>,
    },
    Binary {
        left: Box<ParsedExpression>,
        left_span: Option<TextSpan>,
        operator: ParsedBinaryOperator,
        operator_span: Option<TextSpan>,
        right: Box<ParsedExpression>,
        right_span: Option<TextSpan>,
    },
    Logical {
        left: Box<ParsedExpression>,
        left_span: Option<TextSpan>,
        operator: ParsedLogicalOperator,
        operator_span: Option<TextSpan>,
        right: Box<ParsedExpression>,
        right_span: Option<TextSpan>,
        /// See [`ParsedIfStatement::unreferenced_truthiness_tests`]: the left
        /// operands of the `&&` chain this expression is the root of, which
        /// only the rest of the chain can use. Empty inside an `if` condition,
        /// whose own tests cover them.
        truthiness_tests: Vec<ParsedTruthinessTest>,
    },
    Conditional {
        condition: Box<ParsedExpression>,
        condition_span: Option<TextSpan>,
        when_true: Box<ParsedExpression>,
        when_true_span: Option<TextSpan>,
        when_false: Box<ParsedExpression>,
        when_false_span: Option<TextSpan>,
        /// See [`ParsedIfStatement::unreferenced_truthiness_tests`]; the guarded
        /// code is `when_true`.
        truthiness_tests: Vec<ParsedTruthinessTest>,
    },
    PropertyAccess {
        object: Box<ParsedExpression>,
        object_span: Option<TextSpan>,
        property_name: String,
        property_span: Option<TextSpan>,
        /// True when the source wrote `obj["key"]` (lowered here to a property
        /// access for lookup reuse) rather than `obj.key`. Only dotted accesses
        /// are subject to TS4111 (`noPropertyAccessFromIndexSignature`).
        is_bracketed: bool,
        /// A read a destructuring element makes of its source (tsc's
        /// `getIndexedAccessType` at the element's name, with no access
        /// expression): a missing property is TS2339 on the name, as for a
        /// dotted access, while the rest of it behaves as bracketed.
        binding_element: bool,
    },
    IndexAccess {
        object_name: String,
        object_span: Option<TextSpan>,
        index: Box<ParsedExpression>,
        index_span: Option<TextSpan>,
    },
    /// Element access on an arbitrary object expression (`expr[index]`), as
    /// opposed to [`IndexAccess`] whose object is a bare identifier. Currently
    /// produced when desugaring array destructuring of a non-identifier
    /// initializer (`const [a, b] = useState()` -> `b = useState()[1]`).
    ElementAccess {
        object: Box<ParsedExpression>,
        object_span: Option<TextSpan>,
        index: Box<ParsedExpression>,
        index_span: Option<TextSpan>,
    },
    Call {
        callee_name: String,
        callee_span: Option<TextSpan>,
        type_arguments: Vec<ParsedType>,
        arguments: Vec<ParsedCallArgument>,
    },
    New {
        callee: Box<ParsedExpression>,
        callee_span: Option<TextSpan>,
        /// The whole `new C(…)` expression, which is what tsc underlines for a
        /// diagnostic about the instantiation itself rather than the callee.
        span: Option<TextSpan>,
        type_arguments: Vec<ParsedType>,
        arguments: Vec<ParsedCallArgument>,
    },
    PropertyCall {
        object: Box<ParsedExpression>,
        object_span: Option<TextSpan>,
        property_name: String,
        property_span: Option<TextSpan>,
        call_span: Option<TextSpan>,
        type_arguments: Vec<ParsedType>,
        arguments: Vec<ParsedCallArgument>,
    },
    TypeAssertion {
        expression: Box<ParsedExpression>,
        expression_span: Option<TextSpan>,
        ty: ParsedType,
        type_span: Option<TextSpan>,
        /// Not written as an assertion: the annotation of a destructuring
        /// declaration (`const { a }: T = init`), whose elements read from `T`.
        /// `init` must be assignable to it (TS2322), not merely comparable, and
        /// is reported at the pattern, which `type_span` then spans.
        annotation: bool,
    },
    SatisfiesExpression {
        expression: Box<ParsedExpression>,
        target_type: ParsedType,
        span: Option<TextSpan>,
        target_span: Option<TextSpan>,
    },
    OptionalPropertyAccess {
        object: Box<ParsedExpression>,
        object_span: Option<TextSpan>,
        property_name: String,
        property_span: Option<TextSpan>,
        /// See [`ParsedExpression::PropertyAccess::is_bracketed`].
        is_bracketed: bool,
    },
    OptionalPropertyCall {
        object: Box<ParsedExpression>,
        object_span: Option<TextSpan>,
        property_name: String,
        property_span: Option<TextSpan>,
        call_span: Option<TextSpan>,
        type_arguments: Vec<ParsedType>,
        arguments: Vec<ParsedCallArgument>,
    },
    OptionalIndexAccess {
        object: Box<ParsedExpression>,
        object_span: Option<TextSpan>,
        index: Box<ParsedExpression>,
        index_span: Option<TextSpan>,
    },
    OptionalCall {
        callee: Box<ParsedExpression>,
        callee_span: Option<TextSpan>,
        type_arguments: Vec<ParsedType>,
        arguments: Vec<ParsedCallArgument>,
    },
    /// A call whose callee is neither a bare identifier nor a static member —
    /// an IIFE (`(() => { … })()`), a call on a call, a parenthesized
    /// expression. Without it the whole call (and everything written inside the
    /// callee) parsed to `Unknown` and was never checked.
    ExpressionCall {
        callee: Box<ParsedExpression>,
        callee_span: Option<TextSpan>,
        type_arguments: Vec<ParsedType>,
        arguments: Vec<ParsedCallArgument>,
    },
    NullishCoalescing {
        left: Box<ParsedExpression>,
        left_span: Option<TextSpan>,
        right: Box<ParsedExpression>,
        right_span: Option<TextSpan>,
    },
    NonNullAssertion {
        expression: Box<ParsedExpression>,
        span: Option<TextSpan>,
        /// Indicates if this assertion removes the `| undefined` component from an optional chain.
        /// If false, the optional chain wrapper's undefined short-circuit may still be kept.
        in_optional_chain: bool,
    },
    ConstAssertion {
        expression: Box<ParsedExpression>,
        span: Option<TextSpan>,
    },
    /// A JSX element such as `<div id="x">child</div>` or `<Button />`. Parsed in
    /// `.tsx` mode only; the checker types it conservatively (see [`ParsedJsxChild`]).
    JsxElement {
        /// The tag name exactly as written, e.g. `div`, `Button`, `UI.Button`.
        /// Used for diagnostics/debugging only.
        tag_name: String,
        /// Set when the tag refers to a value that must resolve in scope: the head
        /// identifier of a component (`Button`) or member tag (`UI.Button`).
        /// `None` for intrinsic lowercase elements (`div`), which are not value
        /// references.
        component_name: Option<String>,
        component_span: Option<TextSpan>,
        attributes: Vec<ParsedJsxAttribute>,
        children: Vec<ParsedJsxChild>,
        /// Boxed so the rarely-read tag details do not widen every expression.
        tag: Box<ParsedJsxTag>,
    },
    /// A JSX fragment, `<>...</>`.
    JsxFragment {
        children: Vec<ParsedJsxChild>,
        span: Option<TextSpan>,
    },
    ArrowFunction(Box<ParsedArrowFunction>),
    /// An assignment used as a value (`if ((m = re.exec(s)))`, `a = b = c`).
    /// The target is a plain identifier, and `value` is what it stores, with a
    /// compound or logical operator folded in as for an assignment statement.
    /// The expression's own value is `value`'s.
    Assignment {
        target_name: String,
        target_span: Option<TextSpan>,
        value: Box<ParsedExpression>,
        value_span: Option<TextSpan>,
    },
    /// A member write used as a value (`(o.p = f())`, `a.b = c.d = v`), in a
    /// TypeScript file: checked as the statement form is, and the expression's
    /// own value is `value`'s.
    MemberAssignment(Box<ParsedMemberAssignment>),
    /// A comma expression (`a, b`): every operand runs in order and the value
    /// is the last one's.
    Sequence {
        expressions: Vec<(ParsedExpression, Option<TextSpan>)>,
    },
    /// The members of an object literal assigned to a destructuring pattern
    /// that the pattern does not name (`({ x } = { x: 0, y: 0 })`), each
    /// reported against the pattern's implied type. Synthesized, never written.
    PatternExcessProperties(Vec<ParsedPatternExcessProperty>),
    /// A declaration whose pattern binds nothing (`const [] = source`,
    /// `const {}: T = source`), kept so its source is still checked
    /// (`checkVariableLikeDeclaration`). Synthesized, never written.
    EmptyBindingPattern {
        pattern: Box<ParsedBindingName>,
        source: Box<ParsedExpression>,
        source_span: Option<TextSpan>,
        declared_type: Option<ParsedType>,
    },
    /// The `rest` of `const { a, ...rest } = source`: `source` without the
    /// properties the pattern names (tsc's `getRestType`). Synthesized by the
    /// destructuring lowering, never written.
    ObjectRest {
        source: Box<ParsedExpression>,
        omitted: Vec<String>,
        /// The rest binding's name, where a source that is not an object type
        /// is reported (TS2700).
        name_span: Option<TextSpan>,
    },
    /// Element `index` of an array destructuring assignment's `source`
    /// (`[a, ...rest] = source`), synthesized by the lowering. tsc reads an
    /// array-like source by index, which `read` is, and any other source
    /// through its iteration protocol; a rest element takes the rest of a
    /// tuple or an array of the element type
    /// (`checkArrayLiteralDestructuringElementAssignment`).
    ArrayPatternElement {
        source: Box<ParsedExpression>,
        source_span: Option<TextSpan>,
        index: usize,
        rest: bool,
        /// An array literal `source` is contextually typed by a tuple-like
        /// pattern, and so is a tuple of its elements' widened types.
        tuple_literal: bool,
        read: Box<ParsedExpression>,
    },
    /// The strings array a tagged template passes as its tag's first argument
    /// (`getEffectiveCallArguments`): a value of the global
    /// `TemplateStringsArray`, spanning the template.
    TemplateStringsArray {
        span: Option<TextSpan>,
    },
    /// `/pattern/flags`: a value of the global `RegExp`
    /// (`checkRegularExpressionLiteral`).
    RegExpLiteral,
    /// `import(specifier)` / `import(specifier, options)`: its arguments are
    /// checked (`checkImportCallExpression`); the `Promise` of the module it
    /// evaluates to is not modelled.
    ImportCall {
        specifier: Box<ParsedExpression>,
        specifier_span: Option<TextSpan>,
        options: Option<Box<ParsedExpression>>,
        options_span: Option<TextSpan>,
        span: Option<TextSpan>,
    },
    /// A class expression (`return class extends Base { … }`): a class whose
    /// value is its constructor, checked where it is evaluated (tsc's
    /// `checkClassExpression`). `const C = class {}` is lowered to a class
    /// declaration instead.
    ClassExpression(Box<ParsedClassExpression>),
    /// `f<T>` / `ns.f<T>` with no argument list: an instantiation expression
    /// (tsc's `ExpressionWithTypeArguments`), the value of `expression` with
    /// the type arguments applied to its signatures.
    Instantiation {
        expression: Box<ParsedExpression>,
        expression_span: Option<TextSpan>,
        type_arguments: Vec<ParsedType>,
        /// The written list, `<` through `>`.
        type_arguments_span: Option<TextSpan>,
    },
    Unknown,
}

/// The parts of a JSX element tsc resolves besides its attributes and
/// children.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedJsxTag {
    pub name_span: Option<TextSpan>,
    /// The whole element.
    pub span: Option<TextSpan>,
    /// The tag read as a value (tsc's `checkExpression(tagName)`): `Button`,
    /// `UI.Button`, `this.tag`, `this`. `None` for an intrinsic tag
    /// (`isJsxIntrinsicTagName`: a lowercase or hyphenated identifier, or a
    /// namespaced name).
    pub expression: Option<ParsedExpression>,
    pub type_arguments: Vec<ParsedType>,
    pub type_arguments_span: Option<TextSpan>,
    /// Each child's own node, parallel to the element's children: a `{…}`
    /// child is its container, where tsc reports a child that does not fit
    /// the children prop.
    pub child_spans: Vec<Option<TextSpan>>,
    /// tsc resolves the closing tag again (`checkJsxElementDeferred`), so an
    /// unknown tag is reported at both.
    pub closing: Option<ParsedJsxClosingElement>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedJsxClosingElement {
    pub span: Option<TextSpan>,
    /// See [`ParsedJsxTag::expression`].
    pub expression: Option<ParsedExpression>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedJsxAttribute {
    /// Attribute name, e.g. `id`. Empty for a `{...spread}` attribute.
    pub name: String,
    pub name_span: Option<TextSpan>,
    /// The expression inside a `{...}` attribute value, a JSX element value, or the
    /// argument of a `{...spread}` attribute. `None` for string-literal values
    /// (`id="x"`) and boolean shorthand (`enabled`); their type is recovered from
    /// [`ParsedJsxAttribute::value_kind`] instead.
    pub value: Option<ParsedExpression>,
    pub value_span: Option<TextSpan>,
    /// Classifies values that carry no [`ParsedExpression`] so the checker can type
    /// `id="x"` as `string` and the `disabled` shorthand as `true`.
    pub value_kind: ParsedJsxAttributeValueKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ParsedJsxAttributeValueKind {
    /// The value is the `value` expression (`name={expr}`), an empty container
    /// (`name={}`), or a `{...spread}` argument.
    #[default]
    Expression,
    /// A string-literal value, `name="literal"` — typed as the string literal
    /// (tsc keeps the literal, so `type="submit"` satisfies a literal union).
    StringLiteral(String),
    /// Boolean shorthand, `name` — equivalent to `name={true}`.
    BooleanShorthand,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParsedJsxChild {
    /// Plain text content; nothing to type-check.
    Text,
    /// A `{expression}` (or `{...spread}`) container child. An empty `{}` container
    /// carries `None`.
    Expression {
        expression: Option<ParsedExpression>,
        span: Option<TextSpan>,
        /// A `{...spread}` child, which tsc requires to be an array.
        spread: bool,
    },
    /// A nested JSX element or fragment (itself a [`ParsedExpression`]).
    Element(ParsedExpression),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedObjectProperty {
    pub name: String,
    pub name_span: Option<TextSpan>,
    pub value: ParsedExpression,
    pub value_span: Option<TextSpan>,
    pub span: Option<TextSpan>,
    /// True when this property originates from method shorthand (`{ foo(arg): R { ... } }`).
    /// The `value` is lowered to an arrow function so it reuses arrow checking, but the
    /// declared parameter/return types must be honored when inferring the property type.
    pub is_method: bool,
    /// True for a spread element (`{ ...source }`). `name` is empty and `value`
    /// is the spread argument expression; inference merges the argument's own
    /// object properties into the result.
    pub is_spread: bool,
    /// True for shorthand (`{ value }`), where the property name is also the
    /// identifier read. tsc reports an unresolved shorthand name as TS18004
    /// rather than as a plain missing name.
    pub is_shorthand: bool,
    /// True for a `get`/`set` accessor (`{ get value() { … } }`). The `value` is
    /// lowered to an arrow like method shorthand, but the property's type is the
    /// accessor's *value* type — the getter's return type, or the setter's
    /// parameter type — not the accessor function itself.
    pub is_accessor: bool,
    /// True for the `get` half of an accessor. A getter without a setter is a
    /// read-only property (tsc's `isReadonlySymbol`).
    pub is_getter: bool,
    /// The key expression of a computed name that is not itself a literal
    /// (`{ [key]: v }`). `name` holds its written path; the checker names the
    /// property by the key's literal type once it is known.
    pub computed_key: Option<Box<ParsedExpression>>,
    /// On a getter, the `set` accessor of the same name. The property is typed
    /// from the pair (`getTypeOfAccessors`) and the setter's body is checked.
    pub paired_setter: Option<Box<ParsedArrowFunction>>,
    /// The value of a computed member whose key no member name can model
    /// (`{ [f()]: v }`). Such a member is lowered to an empty spread that adds
    /// nothing to the literal's type; the value is kept only to be checked.
    pub unnamed_key_value: Option<Box<ParsedExpression>>,
    /// The key of a computed method or accessor name (`{ [k]() {} }`), which
    /// tsc checks where the literal is (`checkComputedPropertyName`) but which
    /// does not rename the member the way `computed_key` does.
    pub member_key: Option<Box<ParsedExpression>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedBinaryOperator {
    StrictEquals,
    StrictNotEquals,
    Equals,
    NotEquals,
    LessThan,
    LessThanEquals,
    GreaterThan,
    GreaterThanEquals,
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Exponential,
    ShiftLeft,
    ShiftRight,
    ShiftRightZeroFill,
    BitwiseOR,
    BitwiseXOR,
    BitwiseAnd,
    /// The `in` operator (`"prop" in obj`). Evaluates to `boolean`; used as a
    /// property-presence type guard for narrowing.
    In,
    /// The `instanceof` operator (`x instanceof Ctor`). Evaluates to `boolean`;
    /// used as a type guard for narrowing a union by class membership.
    Instanceof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedLogicalOperator {
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedUnaryOperator {
    Not,
    Plus,
    Minus,
    /// `~`, typed like unary `-`.
    BitwiseNot,
    Typeof,
    /// `delete o.p`: its operand must be a property reference, and that
    /// property must be optional and writable.
    Delete,
    Void,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextSpan {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentDirectiveKind {
    ExpectError,
    Ignore,
}

/// tsc's `CommentDirective`: `span` is where tsc reports the directive (the
/// whole `//` comment, or the last line of a block comment), `suppressed_line`
/// the line whose diagnostics it drops — the next line that is neither blank
/// nor a `//` comment, or `None` when no such line follows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentDirective {
    pub kind: CommentDirectiveKind,
    pub span: TextSpan,
    pub suppressed_line: Option<TextSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedVariableDeclaration {
    pub is_declare: bool,
    pub kind: ParsedVariableKind,
    /// Whether this binding came from a destructuring pattern rather than a
    /// plain `const x = …`. tsc exempts an `_`-prefixed *destructured* binding
    /// from `noUnusedLocals` (the idiom for dropping properties out of a rest
    /// spread) but not a plain one, so the two must stay distinguishable after
    /// the pattern is flattened into one declaration per binding.
    pub from_binding_pattern: bool,
    /// `let x!: T` — a definite-assignment assertion. The binding is asserted to
    /// be initialized elsewhere, so definite-assignment analysis skips it.
    pub has_definite_assertion: bool,
    /// The innermost array pattern this binding is an element of, whose source
    /// must be iterable (TS2488 is reported on the pattern).
    pub array_pattern_span: Option<TextSpan>,
    /// The object side of a lowered `enum`. An enum declared twice in one file
    /// is one enum, so these declarations merge their members instead of the
    /// later one replacing the earlier.
    pub is_enum_object: bool,
    /// The rest element of an array pattern, bound to the whole source: a
    /// tuple source is sliced from this index when it is checked
    /// (`sliceTupleType`).
    pub array_rest_start: Option<usize>,
    pub name: String,
    pub name_span: Option<TextSpan>,
    pub declared_type: Option<ParsedType>,
    pub initializer: Option<ParsedExpression>,
    pub initializer_span: Option<TextSpan>,
    /// The declaration list this binding was written in, shared by every
    /// binding the list declares. `None` for a declaration surge synthesized.
    pub declaration_list: Option<std::sync::Arc<ParsedDeclarationList>>,
    /// The annotated destructuring pattern this binding was lowered from, kept
    /// on the pattern's first binding only.
    pub annotated_pattern: Option<std::sync::Arc<ParsedAnnotatedBindingPattern>>,
    /// The members of the initializer's literals that the unannotated pattern
    /// this binding was lowered from does not name, kept on the pattern's
    /// first binding only.
    pub pattern_excess_properties: Vec<ParsedPatternExcessProperty>,
    /// The member initializers of the `enum` this object was lowered from, one
    /// entry per declaration of it, for the checker to check.
    pub enum_members: Option<std::sync::Arc<Vec<ParsedEnumBody>>>,
}

/// One `enum` declaration's member initializers. tsc checks each as an
/// expression (`checkEnumMember`) with the enum's members in scope, and one
/// that is not a constant expression must be numeric
/// (`computeConstantEnumMemberValue`, TS18033). A literal initializer, which
/// can report nothing, is left out, and so is every initializer of an ambient
/// enum, which may only be constant (TS1066).
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedEnumBody {
    pub is_const: bool,
    pub members: Vec<ParsedEnumMemberInitializer>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedEnumMemberInitializer {
    /// The member's name, as tsc's messages spell it.
    pub name: String,
    pub initializer: ParsedExpression,
    pub initializer_span: Option<TextSpan>,
    /// tsc's evaluator has no value for it (a computed member), as far as the
    /// file alone can tell.
    pub computed: bool,
}

/// `const { a = 1 }: T = …`: tsc types each element of an annotated pattern
/// from the annotation (`getBindingElementTypeFromParentType`) and checks the
/// element's default against that type, which the flattened bindings lose.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedAnnotatedBindingPattern {
    pub pattern: ParsedBindingName,
    pub declared_type: ParsedType,
}

/// A member of an object literal that the binding pattern contextually typing
/// it (tsc's `getContextualTypeForInitializerExpression`) does not name.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedPatternExcessProperty {
    /// The pattern, possibly a nested one, whose implied type types the literal.
    pub pattern: ParsedBindingName,
    pub name: String,
    pub name_span: Option<TextSpan>,
    /// A destructuring *assignment's* pattern, whose targets are existing
    /// names: its implied type takes their types rather than `any`.
    pub typed_targets: bool,
}

/// One `var`/`let`/`const`/`using` declaration list as written, for the
/// unused-binding report (tsc's `reportUnusedVariables`): its declarations
/// with their destructuring patterns, which the lowering flattens.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedDeclarationList {
    pub span: Option<TextSpan>,
    pub declarations: Vec<ParsedDeclarationShape>,
    /// `Some(false)` for `using`, `Some(true)` for `await using`.
    pub disposal: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParsedDeclarationShape {
    /// A bound name. `always_used` when tsc never reports it
    /// (`isUnreferencedVariableDeclaration`): an `_`-prefixed name in an array
    /// pattern, a renaming object element or a `using` declaration, and an
    /// object element whose pattern ends in a rest element.
    Name {
        name: String,
        span: Option<TextSpan>,
        always_used: bool,
    },
    Pattern {
        span: Option<TextSpan>,
        elements: Vec<ParsedDeclarationShape>,
    },
    /// An omitted array element (`[, b]`).
    Omitted,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedAssignment {
    pub target_name: String,
    pub target_span: Option<TextSpan>,
    /// The target as written, parentheses included (`(x) = v`), which is where
    /// tsc reports the value not fitting; `target_span` is the name itself.
    pub written_target_span: Option<TextSpan>,
    pub value: ParsedExpression,
    pub value_span: Option<TextSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedFunctionDeclaration {
    /// `function f(this: T)` — oxc keeps the `this` parameter out of the
    /// parameter list, so its presence has to be carried separately. Only the
    /// implicit-`this` check reads it; nothing about the signature depends on it.
    pub has_this_parameter: bool,
    /// The annotation of that `this` parameter, which types `this` in the body.
    pub this_parameter_type: Option<ParsedType>,
    /// All value-position identifier names read anywhere in the body (including
    /// nested functions, spreads, for-in, and object methods), collected from the
    /// full oxc AST during parsing. Backs unused-binding diagnostics (TS6133).
    ///
    /// Sorted and deduplicated: the unused-binding checks binary-search it, so a
    /// producer that builds this list by hand must preserve that order or those
    /// checks start missing reads (a false TS6133/TS6196).
    pub body_reads: Vec<String>,
    pub is_declare: bool,
    pub name: String,
    pub name_span: Option<TextSpan>,
    pub type_parameters: Vec<ParsedTypeParameter>,
    pub parameters: Vec<ParsedFunctionParameter>,
    pub return_type: Option<ParsedType>,
    pub return_type_span: Option<TextSpan>,
    pub body: Vec<ParsedFunctionBodyStatement>,
    /// False for an overload signature (no body block); its parameters are not
    /// subject to TS6133.
    pub has_body: bool,
    /// `function*` / `async function*`. A generator's declared return type
    /// describes what it *yields*, so tsc does not require it to `return` a
    /// value — the missing-return checks skip it.
    pub is_generator: bool,
    pub is_async: bool,
    /// A JavaScript declaration's `@type` that is its whole signature
    /// (`FullSignature`) and that the parser could not spread over the
    /// parameters: the checker resolves it as the declaration's signature.
    pub full_signature: Option<ParsedType>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParsedFunctionBodyStatement {
    VariableDeclaration(Box<ParsedVariableDeclaration>),
    Return(Box<ParsedReturnStatement>),
    Throw(Box<ParsedThrowStatement>),
    Assignment(Box<ParsedAssignment>),
    /// A `this.<property> = <value>` assignment inside a class method or
    /// constructor body. Checked against the instance property's declared type.
    ThisPropertyAssignment(Box<ParsedThisPropertyAssignment>),
    MemberAssignment(Box<ParsedMemberAssignment>),
    Expression(Box<ParsedExpression>),
    Block(Vec<ParsedFunctionBodyStatement>),
    /// A nested `function` declaration. Retained so identifier reads inside its
    /// body (e.g. a captured outer parameter) stay visible to use-tracking; the
    /// enclosing function's control-flow analysis treats it as inert.
    Function(Box<ParsedFunctionDeclaration>),
    If(Box<ParsedIfStatement>),
    While(Box<ParsedWhileStatement>),
    ForOf(Box<ParsedForOfStatement>),
    Switch(Box<ParsedSwitchStatement>),
    Try(Box<ParsedTryStatement>),
    /// `continue;` — diverts straight-line flow back to the enclosing loop head.
    /// Carries no label/target; modelled only so flow analysis knows the branch
    /// does not fall through (enabling post-guard narrowing of `if (c) continue;`).
    Continue,
    /// `break;` — exits the enclosing loop/switch. Modelled for the same
    /// flow-divergence reason as [`ParsedFunctionBodyStatement::Continue`].
    Break,
    /// A body-local `type` alias. Bound ahead of the statement loop so a
    /// forward reference from an earlier statement still resolves; inert for
    /// control flow.
    TypeAlias(Box<ParsedTypeAliasDeclaration>),
    /// A body-local `interface`. Bound and treated like
    /// [`ParsedFunctionBodyStatement::TypeAlias`].
    Interface(Box<ParsedInterfaceDeclaration>),
    /// A body-local `class`. Contributes both a type and a value binding; its
    /// members are checked once its block is bound.
    Class(Box<ParsedClassDeclaration>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedThrowStatement {
    pub expression: ParsedExpression,
    pub expression_span: Option<TextSpan>,
    pub span: Option<TextSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedSwitchCase {
    pub test: Option<ParsedExpression>,
    pub test_span: Option<TextSpan>,
    pub consequent: Vec<ParsedFunctionBodyStatement>,
    pub span: Option<TextSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedSwitchStatement {
    pub discriminant: ParsedExpression,
    pub discriminant_span: Option<TextSpan>,
    pub cases: Vec<ParsedSwitchCase>,
    pub span: Option<TextSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedTryStatement {
    pub block: Vec<ParsedFunctionBodyStatement>,
    pub handler: Option<ParsedCatchClause>,
    pub finalizer: Vec<ParsedFunctionBodyStatement>,
    pub span: Option<TextSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedCatchClause {
    pub binding_name: Option<ParsedBindingName>,
    pub declared_type: Option<ParsedType>,
    /// The written annotation's type node, where tsc anchors TS1196.
    pub declared_type_span: Option<TextSpan>,
    pub body: Vec<ParsedFunctionBodyStatement>,
    pub span: Option<TextSpan>,
}

/// `o.p = v` / `o.a.b = v` — an assignment whose target is a member of
/// something other than `this`. The target keeps its full expression so the
/// checker can recover the reference path (`o` plus `["a", "b"]`) it narrows.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedMemberAssignment {
    pub target: ParsedExpression,
    pub target_span: Option<TextSpan>,
    pub value: ParsedExpression,
    pub value_span: Option<TextSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedThisPropertyAssignment {
    pub property_name: String,
    pub property_span: Option<TextSpan>,
    /// Written `this["x"]`: an element access, whose missing member is the
    /// implicit `any` TS7053 rather than TS2339.
    pub is_bracketed: bool,
    /// The whole `this.<property>` target, which a mismatch is reported on.
    pub target_span: Option<TextSpan>,
    pub value: ParsedExpression,
    pub value_span: Option<TextSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedReturnStatement {
    pub expression: Option<ParsedExpression>,
    pub expression_span: Option<TextSpan>,
    /// The whole statement, where tsc reports a returned value that does not
    /// fit the return type.
    pub span: Option<TextSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedIfStatement {
    pub condition: ParsedExpression,
    pub condition_span: Option<TextSpan>,
    pub then_body: Vec<ParsedFunctionBodyStatement>,
    pub else_body: Vec<ParsedFunctionBodyStatement>,
    /// References the condition tests for truthiness that neither the then
    /// branch nor the rest of their `&&` chain mentions again. tsc reports such a
    /// reference when its type is a function (TS2774); whether it is used is a
    /// question about the source, so it is answered here.
    pub unreferenced_truthiness_tests: Vec<ParsedTruthinessTest>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedTruthinessTest {
    pub expression: ParsedExpression,
    pub span: Option<TextSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedWhileStatement {
    pub condition: ParsedExpression,
    pub condition_span: Option<TextSpan>,
    pub body: Vec<ParsedFunctionBodyStatement>,
    /// The body runs before the condition can stop it — a lowered `do … while
    /// (c)`, or a `while (true)`. Assignments the body makes are definite
    /// afterwards, and the condition may read what the body assigned.
    pub runs_at_least_once: bool,
    /// A lowered `for`: `body[0]` is the loop body and the rest its update
    /// clause, which a `continue` in the body reaches too.
    pub has_update: bool,
}

/// How a `for…of`/`for…in` head binds its name. A `var` outlives the loop and
/// is unassigned where the loop never ran; a bare name assigns an existing
/// binding on each iteration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedForBindingKind {
    Var,
    BlockScoped,
    ExistingBinding,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedForOfStatement {
    pub binding_name: ParsedBindingName,
    pub binding_kind: ParsedForBindingKind,
    pub iterable: ParsedExpression,
    pub iterable_span: Option<TextSpan>,
    pub body: Vec<ParsedFunctionBodyStatement>,
    /// `for (k in o)` rather than `for (k of o)`: the binding is the property
    /// key (always `string`), not the iterated element, and the right-hand side
    /// need not be iterable.
    pub keys_only: bool,
    /// `for await (x of xs)`, which may iterate an async iterable.
    pub is_await: bool,
    /// A head that writes something other than a name (`for (o.k in x)`,
    /// or a target tsc rejects), checked as the expression it is; its
    /// `binding_name` is `Unsupported`.
    pub head_target: Option<(ParsedExpression, Option<TextSpan>)>,
    /// The names a destructuring head (`for ([a, b] of pairs)`) writes, which
    /// must resolve like any other reference.
    pub head_names: Vec<(String, Option<TextSpan>)>,
    /// A destructuring head of a `for…of`, lowered as the assignments it makes
    /// from each iterated value, which they read as [`FOR_OF_HEAD_ELEMENT`].
    pub head_assignments: Vec<ParsedAssignment>,
}

/// The name a lowered `for…of` destructuring head reads the iterated value by;
/// the checker binds it to the element type.
pub const FOR_OF_HEAD_ELEMENT: &str = "\0for-of-element";

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedFunctionParameter {
    pub binding_name: ParsedBindingName,
    pub declared_type: Option<ParsedType>,
    pub initializer: Option<ParsedExpression>,
    pub initializer_span: Option<TextSpan>,
    pub optional: bool,
    /// A parameter of an untyped JavaScript signature
    /// (`isUntypedSignatureInJSFile`): `optional` for arity only, so its type
    /// gains no `undefined`.
    pub untyped_javascript: bool,
    /// `...args` rest parameter. Marks the signature variadic so arity checks
    /// accept any number of trailing arguments.
    pub rest: bool,
    /// Constructor parameter property: the parameter carries an accessibility
    /// (`public`/`private`/`protected`), `readonly` or `override` modifier,
    /// which declares a class instance member of the same name and type.
    pub is_parameter_property: bool,
    /// The parameter property was declared `readonly`, so the member it
    /// declares rejects writes (tsc's `isReadonlySymbol`).
    pub is_readonly_parameter_property: bool,
    /// The parameter property was declared `override`.
    pub is_override_parameter_property: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedArrowFunction {
    /// What `this` means inside the body. A `function` expression and an
    /// object-literal method both lower to this shape, and neither inherits
    /// `this` the way a real arrow does.
    pub this_binding: ParsedThisBinding,
    /// A written `this` parameter's annotation (`function (this: C) {…}`),
    /// which types the `this` a non-arrow binds.
    pub this_parameter_type: Option<ParsedType>,
    /// A named `function` expression's own name, which is in scope in its own
    /// signature and body only. A method's name is a property, never a binding.
    pub name: Option<String>,
    /// See [`ParsedFunctionDeclaration::body_reads`].
    pub body_reads: Vec<String>,
    pub type_parameters: Vec<ParsedTypeParameter>,
    pub parameters: Vec<ParsedFunctionParameter>,
    pub return_type: Option<ParsedType>,
    pub return_type_span: Option<TextSpan>,
    pub is_async: bool,
    /// A `function*` / `async function*` expression lowered to this shape. Its
    /// return type is a `Generator`/`AsyncGenerator`, never the body's
    /// completion value — inferring the latter typed `async function* () {
    /// yield 'a' }` as `void`.
    pub is_generator: bool,
    pub body: ParsedArrowFunctionBody,
    /// The expression of an expression-bodied arrow, which tsc anchors a
    /// mismatched return on (`elaborateArrowFunction`).
    pub body_span: Option<TextSpan>,
    pub span: Option<TextSpan>,
    /// The captured parameters and mutable locals the closure reads at their
    /// declared types: assigned after it starts, or inside any closure
    /// (tsc's `isPastLastAssignment`).
    pub narrowing_resets: Vec<String>,
}

/// Where a lowered function body's `this` comes from. An arrow does not bind
/// `this` at all, so it sees the enclosing function's; everything else lowered
/// to [`ParsedArrowFunction`] binds its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedThisBinding {
    /// A real arrow: `this` is whatever the enclosing function bound, which is
    /// why `function f() { return () => this }` reports on the *arrow's* `this`.
    Inherited,
    /// An object-literal method or accessor, or a `function` expression that
    /// annotates `this`: the body has a `this` of its own.
    Own,
    /// A `function` expression with no `this` parameter: `this` has no type.
    ImplicitAny,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParsedArrowFunctionBody {
    Expression(Box<ParsedExpression>),
    Block(Vec<ParsedFunctionBodyStatement>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedCall {
    pub callee_name: String,
    pub callee_span: Option<TextSpan>,
    pub span: Option<TextSpan>,
    pub type_arguments: Vec<ParsedType>,
    pub arguments: Vec<ParsedCallArgument>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedCallArgument {
    pub expression: ParsedExpression,
    pub span: Option<TextSpan>,
    /// `f(...xs)`. The count this contributes depends on the spread's own type,
    /// so a call carrying one has no statically known argument count.
    pub spread: bool,
    /// The argument without a spread's `...`.
    pub expression_span: Option<TextSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedArrayElement {
    pub expression: ParsedExpression,
    pub span: Option<TextSpan>,
    /// `[...xs]`. The element stands for however many elements `xs` holds, and
    /// what it contributes is `xs`'s *element* type, not `xs` itself.
    pub spread: bool,
    /// `[1, , 3]`: an omitted element, `undefined` to tsc, which never
    /// elaborates a mismatch into one.
    pub omitted: bool,
}

/// Census-only estimated owned-heap size of a parsed type tree, used by the
/// retained-memory instrumentation to attribute parsed-annotation retention.
/// Shallow-struct sizes plus owned string/vec heap; not allocator ground truth.
impl ParsedType {
    /// Visits every named reference written anywhere inside this type.
    pub fn for_each_named_type(&self, visit: &mut dyn FnMut(&ParsedNamedType)) {
        fn function(signature: &ParsedFunctionType, visit: &mut dyn FnMut(&ParsedNamedType)) {
            for parameter in &signature.parameters {
                parameter.ty.for_each_named_type(visit);
            }
            signature.return_type.for_each_named_type(visit);
            for type_parameter in &signature.type_parameters {
                for written in [&type_parameter.constraint, &type_parameter.default_type]
                    .into_iter()
                    .flatten()
                {
                    written.for_each_named_type(visit);
                }
            }
        }
        match self {
            ParsedType::Named(named) => {
                visit(named);
                for argument in &named.type_arguments {
                    argument.for_each_named_type(visit);
                }
            }
            ParsedType::Array(element) | ParsedType::Readonly(element) | ParsedType::KeyOf(element) => {
                element.for_each_named_type(visit);
            }
            ParsedType::Tuple(elements) | ParsedType::Union(elements) | ParsedType::Intersection(elements) => {
                for element in elements.iter() {
                    element.for_each_named_type(visit);
                }
            }
            ParsedType::VariadicTuple(elements) => {
                for element in elements.iter() {
                    match element {
                        ParsedTupleElement::Fixed(ty) | ParsedTupleElement::Rest(ty, _) => {
                            ty.for_each_named_type(visit);
                        }
                    }
                }
            }
            ParsedType::Object(object) => {
                for property in &object.properties {
                    property.ty.for_each_named_type(visit);
                }
                for index in [&object.string_index_type, &object.number_index_type]
                    .into_iter()
                    .flatten()
                {
                    index.for_each_named_type(visit);
                }
                if let Some(call_signature) = object.call_signature.as_deref() {
                    function(call_signature, visit);
                }
            }
            ParsedType::Function(signature) => function(signature, visit),
            ParsedType::IndexedAccess(indexed_access) => {
                indexed_access.object_type.for_each_named_type(visit);
                indexed_access.index_type.for_each_named_type(visit);
            }
            ParsedType::Mapped(mapped) => {
                mapped.constraint.for_each_named_type(visit);
                mapped.value_type.for_each_named_type(visit);
                if let Some(name_type) = mapped.name_type.as_deref() {
                    name_type.for_each_named_type(visit);
                }
            }
            ParsedType::Conditional(conditional) => {
                conditional.check_type.for_each_named_type(visit);
                conditional.extends_type.for_each_named_type(visit);
                conditional.true_type.for_each_named_type(visit);
                conditional.false_type.for_each_named_type(visit);
            }
            ParsedType::TemplateLiteral(template) => {
                for interpolation in &template.interpolations {
                    interpolation.for_each_named_type(visit);
                }
            }
            ParsedType::Predicate(predicate) => {
                if let Some(ty) = predicate.ty.as_ref() {
                    ty.for_each_named_type(visit);
                }
            }
            ParsedType::String
            | ParsedType::Number
            | ParsedType::Boolean
            | ParsedType::BigInt
            | ParsedType::Symbol
            | ParsedType::Undefined
            | ParsedType::Null
            | ParsedType::Void
            | ParsedType::Any
            | ParsedType::ErrorType
            | ParsedType::Unknown
            | ParsedType::UnknownKeyword
            | ParsedType::Never
            | ParsedType::StringLiteral(_)
            | ParsedType::NumberLiteral(_)
            | ParsedType::BooleanLiteral(_)
            | ParsedType::InferredMember(_)
            | ParsedType::UniqueSymbol(_)
            | ParsedType::TypeOf(_)
            | ParsedType::Infer(_) => {}
        }
    }

    pub fn estimated_heap_bytes(&self) -> u64 {
        let own = std::mem::size_of::<ParsedType>() as u64;
        own + match self {
            ParsedType::StringLiteral(value) | ParsedType::NumberLiteral(value) => {
                value.capacity() as u64
            }
            ParsedType::Object(object) => {
                let mut bytes = 0u64;
                for property in &object.properties {
                    bytes += property.name.capacity() as u64
                        + std::mem::size_of::<ParsedObjectTypeProperty>() as u64
                        + property.ty.estimated_heap_bytes();
                }
                if let Some(call) = object.call_signature.as_ref() {
                    bytes += call.estimated_heap_bytes();
                }
                if let Some(construct) = object.construct_signature.as_ref() {
                    bytes += construct.estimated_heap_bytes();
                }
                bytes
            }
            ParsedType::Array(element) | ParsedType::KeyOf(element) => {
                element.estimated_heap_bytes()
            }
            ParsedType::Tuple(elements)
            | ParsedType::Union(elements)
            | ParsedType::Intersection(elements) => elements
                .iter()
                .map(ParsedType::estimated_heap_bytes)
                .sum::<u64>(),
            ParsedType::Function(function) => function.estimated_heap_bytes(),
            ParsedType::Named(named) => named.estimated_heap_bytes(),
            ParsedType::TypeOf(type_of) => {
                type_of.name.capacity() as u64
                    + type_of
                        .members
                        .iter()
                        .map(|member| {
                            member.capacity() as u64 + std::mem::size_of::<String>() as u64
                        })
                        .sum::<u64>()
            }
            ParsedType::IndexedAccess(indexed) => {
                indexed.object_type.estimated_heap_bytes()
                    + indexed.index_type.estimated_heap_bytes()
            }
            ParsedType::Mapped(mapped) => {
                mapped.key_name.capacity() as u64
                    + mapped.constraint.estimated_heap_bytes()
                    + mapped.value_type.estimated_heap_bytes()
            }
            ParsedType::Conditional(conditional) => {
                conditional.check_type.estimated_heap_bytes()
                    + conditional.extends_type.estimated_heap_bytes()
                    + conditional.true_type.estimated_heap_bytes()
                    + conditional.false_type.estimated_heap_bytes()
            }
            ParsedType::TemplateLiteral(template) => {
                template
                    .quasis
                    .iter()
                    .map(|quasi| quasi.capacity() as u64 + std::mem::size_of::<String>() as u64)
                    .sum::<u64>()
                    + template
                        .interpolations
                        .iter()
                        .map(ParsedType::estimated_heap_bytes)
                        .sum::<u64>()
            }
            ParsedType::Infer(infer) => infer.name.capacity() as u64,
            _ => 0,
        }
    }
}

impl ParsedFunctionType {
    pub fn estimated_heap_bytes(&self) -> u64 {
        let mut bytes = std::mem::size_of::<ParsedFunctionType>() as u64;
        for parameter in &self.parameters {
            bytes += std::mem::size_of::<ParsedFunctionTypeParameter>() as u64;
            bytes += parameter
                .name
                .as_ref()
                .map_or(0, |name| name.capacity() as u64);
            bytes += parameter.ty.estimated_heap_bytes();
        }
        bytes += self.return_type.estimated_heap_bytes();
        for parameter in &self.type_parameters {
            bytes += parameter.estimated_heap_bytes();
        }
        bytes
    }
}

impl ParsedTypeParameter {
    pub fn estimated_heap_bytes(&self) -> u64 {
        let mut bytes = (std::mem::size_of::<ParsedTypeParameter>() + self.name.capacity()) as u64;
        if let Some(constraint) = self.constraint.as_ref() {
            bytes += constraint.estimated_heap_bytes();
        }
        if let Some(default_type) = self.default_type.as_ref() {
            bytes += default_type.estimated_heap_bytes();
        }
        bytes
    }
}

impl ParsedNamedType {
    pub fn estimated_heap_bytes(&self) -> u64 {
        let mut bytes = (std::mem::size_of::<ParsedNamedType>() + self.name.capacity()) as u64;
        bytes += (self.type_argument_spans.capacity() * std::mem::size_of::<TextSpan>()) as u64;
        for argument in &self.type_arguments {
            bytes += argument.estimated_heap_bytes();
        }
        bytes
    }
}

impl ParsedExpression {
    /// Calls `visit` on each direct operand, in evaluation order. A nested
    /// function's body is not an operand: it runs later, in its own flow.
    pub fn for_each_child<'a>(&'a self, visit: &mut impl FnMut(&'a ParsedExpression)) {
        let arguments = |arguments: &'a [ParsedCallArgument], visit: &mut dyn FnMut(&'a ParsedExpression)| {
            for argument in arguments {
                visit(&argument.expression);
            }
        };
        match self {
            ParsedExpression::ObjectLiteral { properties, .. } => {
                for property in properties {
                    if !property.is_method && !property.is_accessor {
                        visit(&property.value);
                    }
                }
            }
            ParsedExpression::ArrayLiteral { elements, .. } => {
                for element in elements {
                    visit(&element.expression);
                }
            }
            ParsedExpression::TemplateLiteral { expressions, .. } => {
                for expression in expressions {
                    visit(expression);
                }
            }
            ParsedExpression::Unary { operand, .. }
            | ParsedExpression::Update { operand, .. }
            | ParsedExpression::Await { operand, .. } => visit(operand),
            ParsedExpression::Yield { operand, .. } => {
                if let Some(operand) = operand {
                    visit(operand);
                }
            }
            ParsedExpression::ObjectRest { source, .. }
            | ParsedExpression::EmptyBindingPattern { source, .. } => visit(source),
            ParsedExpression::ArrayPatternElement { read, .. } => visit(read),
            ParsedExpression::ImportCall { specifier, options, .. } => {
                visit(specifier);
                if let Some(options) = options {
                    visit(options);
                }
            }
            ParsedExpression::Binary { left, right, .. }
            | ParsedExpression::Logical { left, right, .. }
            | ParsedExpression::NullishCoalescing { left, right, .. } => {
                visit(left);
                visit(right);
            }
            ParsedExpression::Conditional {
                condition,
                when_true,
                when_false,
                ..
            } => {
                visit(condition);
                visit(when_true);
                visit(when_false);
            }
            ParsedExpression::PropertyAccess { object, .. }
            | ParsedExpression::OptionalPropertyAccess { object, .. } => visit(object),
            ParsedExpression::IndexAccess { index, .. } => visit(index),
            ParsedExpression::ElementAccess { object, index, .. }
            | ParsedExpression::OptionalIndexAccess { object, index, .. } => {
                visit(object);
                visit(index);
            }
            ParsedExpression::Call { arguments: args, .. } => arguments(args, visit),
            ParsedExpression::New { callee, arguments: args, .. }
            | ParsedExpression::OptionalCall { callee, arguments: args, .. }
            | ParsedExpression::ExpressionCall { callee, arguments: args, .. } => {
                visit(callee);
                arguments(args, visit);
            }
            ParsedExpression::PropertyCall { object, arguments: args, .. }
            | ParsedExpression::OptionalPropertyCall { object, arguments: args, .. } => {
                visit(object);
                arguments(args, visit);
            }
            ParsedExpression::TypeAssertion { expression, .. }
            | ParsedExpression::SatisfiesExpression { expression, .. }
            | ParsedExpression::NonNullAssertion { expression, .. }
            | ParsedExpression::ConstAssertion { expression, .. }
            | ParsedExpression::Instantiation { expression, .. } => visit(expression),
            ParsedExpression::Assignment { value, .. } => visit(value),
            ParsedExpression::MemberAssignment(assignment) => {
                visit(&assignment.target);
                visit(&assignment.value);
            }
            ParsedExpression::Sequence { expressions } => {
                for (expression, _) in expressions {
                    visit(expression);
                }
            }
            ParsedExpression::JsxElement {
                attributes,
                children,
                ..
            } => {
                for attribute in attributes {
                    if let Some(value) = &attribute.value {
                        visit(value);
                    }
                }
                for child in children {
                    match child {
                        ParsedJsxChild::Expression {
                            expression: Some(expression),
                            ..
                        }
                        | ParsedJsxChild::Element(expression) => visit(expression),
                        _ => {}
                    }
                }
            }
            ParsedExpression::JsxFragment { children, .. } => {
                for child in children {
                    match child {
                        ParsedJsxChild::Expression {
                            expression: Some(expression),
                            ..
                        }
                        | ParsedJsxChild::Element(expression) => visit(expression),
                        _ => {}
                    }
                }
            }
            ParsedExpression::StringLiteral(_)
            | ParsedExpression::NumberLiteral(_)
            | ParsedExpression::BigIntLiteral(_)
            | ParsedExpression::BooleanLiteral(_)
            | ParsedExpression::UndefinedLiteral
            | ParsedExpression::NullLiteral
            | ParsedExpression::Identifier { .. }
            | ParsedExpression::This { .. }
            | ParsedExpression::ArrowFunction(_)
            | ParsedExpression::ClassExpression(_)
            | ParsedExpression::TemplateStringsArray { .. }
            | ParsedExpression::RegExpLiteral
            | ParsedExpression::PatternExcessProperties(_)
            | ParsedExpression::Unknown => {}
        }
    }

    /// Whether evaluating this expression assigns a binding (outside nested
    /// functions).
    pub fn contains_assignment(&self) -> bool {
        if matches!(self, ParsedExpression::Assignment { .. }) {
            return true;
        }
        let mut found = false;
        self.for_each_child(&mut |child| found |= child.contains_assignment());
        found
    }

    /// This expression with each assignment replaced by a read of its target:
    /// what a condition tests once its assignments have run (`(m = f()) !== null`
    /// narrows `m`).
    pub fn with_assignments_as_reads(&self) -> ParsedExpression {
        match self {
            ParsedExpression::Assignment {
                target_name,
                target_span,
                ..
            } => ParsedExpression::Identifier {
                name: target_name.clone(),
                span: *target_span,
            },
            _ if !self.contains_assignment() => self.clone(),
            ParsedExpression::Binary { left, left_span, operator, operator_span, right, right_span } => {
                ParsedExpression::Binary {
                    left: Box::new(left.with_assignments_as_reads()),
                    left_span: *left_span,
                    operator: *operator,
                    operator_span: *operator_span,
                    right: Box::new(right.with_assignments_as_reads()),
                    right_span: *right_span,
                }
            }
            ParsedExpression::Logical { left, left_span, operator, operator_span, right, right_span, truthiness_tests } => {
                ParsedExpression::Logical {
                    left: Box::new(left.with_assignments_as_reads()),
                    left_span: *left_span,
                    operator: *operator,
                    operator_span: *operator_span,
                    right: Box::new(right.with_assignments_as_reads()),
                    right_span: *right_span,
                    truthiness_tests: truthiness_tests.clone(),
                }
            }
            ParsedExpression::Unary { operator, operator_span, operand, operand_span } => {
                ParsedExpression::Unary {
                    operator: *operator,
                    operator_span: *operator_span,
                    operand: Box::new(operand.with_assignments_as_reads()),
                    operand_span: *operand_span,
                }
            }
            ParsedExpression::Sequence { expressions } => ParsedExpression::Sequence {
                expressions: expressions
                    .iter()
                    .map(|(expression, span)| (expression.with_assignments_as_reads(), *span))
                    .collect(),
            },
            other => other.clone(),
        }
    }

    /// Whether this expression is a later link of an optional chain, so a
    /// member access on it short-circuits with the chain instead of being a
    /// non-optional access on a possibly-`undefined` receiver: the `.c` of
    /// `a?.b.c` never runs when `a?.b` is `undefined`.
    pub fn continues_optional_chain(&self) -> bool {
        match self {
            ParsedExpression::OptionalPropertyAccess { .. }
            | ParsedExpression::OptionalIndexAccess { .. }
            | ParsedExpression::OptionalPropertyCall { .. }
            | ParsedExpression::OptionalCall { .. } => true,
            ParsedExpression::PropertyAccess { object, .. }
            | ParsedExpression::ElementAccess { object, .. }
            | ParsedExpression::PropertyCall { object, .. } => object.continues_optional_chain(),
            ParsedExpression::ExpressionCall { callee, .. } => callee.continues_optional_chain(),
            ParsedExpression::NonNullAssertion {
                in_optional_chain, ..
            } => *in_optional_chain,
            _ => false,
        }
    }
}
