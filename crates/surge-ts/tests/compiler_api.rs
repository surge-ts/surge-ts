//! The compiler API (`surge_ts::api`) over small programs on disk and in
//! memory: program construction, syntax trees and positions, diagnostics,
//! symbols, types and signatures, configuration and module resolution.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use surge_ts::api::{
    CompilerHost, CompilerOptions, CreateProgramOptions, DirectoryEntry, NodeId, NodePropertyValue, Program, SignatureKind,
    SyntaxKind, TypeChecker, parse_json_config_file_content, resolve_module_name, symbol_flags, type_flags,
};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A fresh directory holding `files`, canonicalized as the checker names files.
fn project(files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "surge-api-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    for (name, text) in files {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
    }
    std::fs::canonicalize(&dir).unwrap()
}

fn strict() -> CompilerOptions {
    let mut options = CompilerOptions::new();
    options.set("strict", serde_json::Value::Bool(true));
    options.set("noEmit", serde_json::Value::Bool(true));
    options
}

fn program(dir: &Path, roots: &[&str]) -> Program {
    Program::create(CreateProgramOptions {
        root_names: roots.iter().map(|root| dir.join(root).to_string_lossy().into_owned()).collect(),
        options: strict(),
        ..Default::default()
    })
    .expect("program")
}

/// Every node of a file in `forEachChild` pre-order.
fn nodes(program: &Program, root: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        out.push(node);
        let mut children = program.node_children(node);
        children.reverse();
        stack.extend(children);
    }
    out
}

/// The first identifier node spelled `name` at or after `from` in the file.
fn identifier(program: &Program, file: surge_ts::api::SourceFileId, name: &str, occurrence: usize) -> NodeId {
    nodes(program, program.root(file))
        .into_iter()
        .filter(|node| program.node_kind(*node) == SyntaxKind::Identifier && program.node_text(*node) == name)
        .nth(occurrence)
        .unwrap_or_else(|| panic!("no identifier `{name}` #{occurrence}"))
}

#[test]
fn creates_a_program_with_its_files_and_libraries() {
    let dir = project(&[("index.ts", "import { a } from './a';\nexport const b = a + 1;\n"), ("a.ts", "export const a = 1;\n")]);
    let program = program(&dir, &["index.ts"]);
    let names: Vec<String> = program.source_files().into_iter().map(|file| program.file_name(file).to_string()).collect();
    assert!(names.iter().any(|name| name.ends_with("/index.ts")));
    assert!(names.iter().any(|name| name.ends_with("/a.ts")), "imports join the program: {names:?}");
    let first = program.source_files()[0];
    assert!(program.is_source_file_default_library(first), "default libraries come first");
    assert_eq!(program.root_file_names().len(), 1);
    let index = program.source_file(&dir.join("index.ts").to_string_lossy()).expect("by name");
    assert!(program.is_external_module(index));
    assert!(!program.is_declaration_file(index));
    assert!(program.pre_emit_diagnostics(None).is_empty());
}

#[test]
fn positions_are_utf16_offsets() {
    // '😀' is two UTF-16 units and four UTF-8 bytes.
    let dir = project(&[("index.ts", "const s = \"😀\";\nconst n: number = s;\n")]);
    let program = program(&dir, &["index.ts"]);
    let file = program.source_file(&dir.join("index.ts").to_string_lossy()).unwrap();
    let n = identifier(&program, file, "n", 0);
    assert_eq!(program.node_start(n), 22);
    assert_eq!(program.line_and_character_of_position(file, 22).line, 1);
    assert_eq!(program.position_of_line_and_character(file, 1, 6), Some(22));
    let diagnostics = program.semantic_diagnostics(Some(file));
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, 2322);
    assert_eq!(diagnostics[0].start, Some(22), "TS2322 is reported at `n`, in UTF-16 units");
    assert_eq!(diagnostics[0].length, Some(1));
    assert_eq!(program.text_length(file), 37);
}

#[test]
fn traverses_the_syntax_tree_with_typescript_kinds() {
    let dir = project(&[("index.ts", "function add(a: number, b: number) { return a + b; }\nadd(1, 2);\n")]);
    let program = program(&dir, &["index.ts"]);
    let file = program.source_file(&dir.join("index.ts").to_string_lossy()).unwrap();
    let root = program.root(file);
    assert_eq!(program.node_kind(root), SyntaxKind::SourceFile);
    let statements = match program.node_property(root, "statements") {
        Some(NodePropertyValue::List { nodes, .. }) => nodes,
        other => panic!("statements: {other:?}"),
    };
    assert_eq!(statements.len(), 2);
    assert_eq!(program.node_kind(statements[0]), SyntaxKind::FunctionDeclaration);
    assert_eq!(program.node_kind(statements[1]), SyntaxKind::ExpressionStatement);
    let call = match program.node_property(statements[1], "expression") {
        Some(NodePropertyValue::Node(call)) => call,
        other => panic!("expression: {other:?}"),
    };
    assert_eq!(program.node_kind(call), SyntaxKind::CallExpression);
    assert_eq!(program.node_text(call), "add(1, 2)");
    assert_eq!(program.node_parent(call), Some(statements[1]));
    let children = program.node_children(root);
    assert_eq!(program.node_kind(*children.last().unwrap()), SyntaxKind::EndOfFileToken);
    let plus = nodes(&program, root)
        .into_iter()
        .find(|node| program.node_kind(*node) == SyntaxKind::PlusToken)
        .expect("a binary expression keeps its operator token");
    assert_eq!(program.node_kind(program.node_parent(plus).unwrap()), SyntaxKind::BinaryExpression);
}

#[test]
fn resolves_symbols_and_their_types() {
    let dir = project(&[(
        "index.ts",
        "interface Person { name: string; age?: number }\n\
         function greet(person: Person): string { return person.name; }\n\
         const who: Person = { name: 'Ada' };\n\
         let message = greet(who);\n\
         const length = message.length;\n",
    )]);
    let program = program(&dir, &["index.ts"]);
    let checker = program.type_checker();
    let file = program.source_file(&dir.join("index.ts").to_string_lossy()).unwrap();

    let message = identifier(&program, file, "message", 0);
    let symbol = checker.symbol_at_location(message).expect("declaration name");
    assert_eq!(checker.symbol_name(symbol), "message");
    assert_ne!(checker.symbol_flags(symbol) & symbol_flags::BlockScopedVariable, 0);
    assert_eq!(checker.type_to_string(checker.type_of_symbol(symbol)), "string");

    let reference = identifier(&program, file, "message", 1);
    assert_eq!(checker.symbol_at_location(reference), Some(symbol), "a reference resolves to the declaration's symbol");
    assert_eq!(checker.symbol_declarations(symbol)[0], program.node_parent(message).unwrap());

    let length = identifier(&program, file, "length", 1);
    let length_symbol = checker.symbol_at_location(length).expect("property name");
    assert_eq!(checker.symbol_name(length_symbol), "length");
    assert_eq!(checker.type_to_string(checker.type_of_symbol_at_location(length_symbol, length)), "number");

    let person = identifier(&program, file, "Person", 0);
    let interface = checker.symbol_at_location(person).unwrap();
    assert_ne!(checker.symbol_flags(interface) & symbol_flags::Interface, 0);
    let declared = checker.declared_type_of_symbol(interface);
    assert_eq!(checker.type_to_string(declared), "Person");
    let properties: Vec<String> =
        checker.properties_of_type(declared).into_iter().map(|property| checker.symbol_name(property)).collect();
    assert_eq!(properties, ["name", "age"]);
    let age = checker.property_of_type(declared, "age").unwrap();
    assert_eq!(checker.type_to_string(checker.type_of_symbol(age)), "number | undefined");
    assert!(checker.symbol_value_declaration(age).is_some(), "an interface member keeps its declaration");
}

#[test]
fn answers_signatures_and_assignability() {
    let dir = project(&[("index.ts", "function twice(value: number): number { return value * 2; }\nconst n = twice(2);\n")]);
    let program = program(&dir, &["index.ts"]);
    let checker = program.type_checker();
    let file = program.source_file(&dir.join("index.ts").to_string_lossy()).unwrap();
    let twice = checker.symbol_at_location(identifier(&program, file, "twice", 0)).unwrap();
    let function = checker.type_of_symbol(twice);
    assert_eq!(checker.type_to_string(function), "(value: number) => number");
    let signatures = checker.signatures_of_type(function, SignatureKind::Call);
    assert_eq!(signatures.len(), 1);
    assert_eq!(checker.type_to_string(checker.return_type_of_signature(signatures[0])), "number");
    assert_eq!(checker.signature_to_string(signatures[0]), "(value: number): number");
    let parameters = checker.signature_parameters(signatures[0]);
    assert_eq!(parameters.iter().map(|parameter| checker.symbol_name(*parameter)).collect::<Vec<_>>(), ["value"]);
    assert!(checker.signatures_of_type(function, SignatureKind::Construct).is_empty());

    let number = checker.number_type();
    let string = checker.string_type();
    assert!(checker.is_type_assignable_to(number, number));
    assert!(!checker.is_type_assignable_to(number, string));
    assert!(checker.is_type_assignable_to(number, checker.any_type()));
    assert_ne!(checker.type_flags(number) & type_flags::Number, 0);
}

#[test]
fn follows_import_aliases_and_module_exports() {
    let dir = project(&[
        ("index.ts", "import { greet as hello } from './lib';\nimport * as lib from './lib';\nhello();\nlib.greet();\n"),
        ("lib.ts", "export function greet(): void {}\nexport const version = 1;\n"),
    ]);
    let program = program(&dir, &["index.ts"]);
    let checker = program.type_checker();
    let file = program.source_file(&dir.join("index.ts").to_string_lossy()).unwrap();
    let alias = checker.symbol_at_location(identifier(&program, file, "hello", 0)).unwrap();
    assert_ne!(checker.symbol_flags(alias) & symbol_flags::Alias, 0);
    let target = checker.aliased_symbol(alias).expect("the alias names an export");
    assert_ne!(checker.symbol_flags(target) & symbol_flags::Function, 0);
    let declaration = checker.symbol_declarations(target)[0];
    assert!(program.file_name(declaration.source_file()).ends_with("/lib.ts"));

    let namespace = checker.symbol_at_location(identifier(&program, file, "lib", 0)).unwrap();
    let module = checker.aliased_symbol(namespace).unwrap();
    let exports: Vec<String> = checker.exports_of_module(module).into_iter().map(|export| checker.symbol_name(export)).collect();
    assert_eq!(exports, ["greet", "version"]);
    let member = identifier(&program, file, "greet", 1);
    assert_eq!(checker.symbol_at_location(member), Some(target), "`lib.greet` is the module's export");
}

#[test]
#[should_panic(expected = "belongs to another program")]
fn a_handle_from_another_program_is_rejected() {
    let dir = project(&[("index.ts", "const x = 1;\n")]);
    let first = program(&dir, &["index.ts"]);
    let second = program(&dir, &["index.ts"]);
    let file = first.source_file(&dir.join("index.ts").to_string_lossy()).unwrap();
    let node = identifier(&first, file, "x", 0);
    let _ = second.type_checker().type_at_location(node);
}

#[test]
fn two_live_programs_answer_independently() {
    let first_dir = project(&[("index.ts", "export const value: string = 'a';\n")]);
    let second_dir = project(&[("index.ts", "export const value: number = 1;\n")]);
    let first = program(&first_dir, &["index.ts"]);
    let second = program(&second_dir, &["index.ts"]);
    let describe = |program: &Program, dir: &Path| {
        let checker = program.type_checker();
        let file = program.source_file(&dir.join("index.ts").to_string_lossy()).unwrap();
        let symbol = checker.symbol_at_location(identifier(program, file, "value", 0)).unwrap();
        checker.type_to_string(checker.type_of_symbol(symbol))
    };
    assert_eq!(describe(&second, &second_dir), "number");
    assert_eq!(describe(&first, &first_dir), "string");
    assert_eq!(describe(&second, &second_dir), "number");
}

#[test]
fn diagnostics_match_the_project_check() {
    let dir = project(&[
        ("tsconfig.json", r#"{ "compilerOptions": { "strict": true, "noEmit": true }, "include": ["src"] }"#),
        ("src/index.ts", "const a: string = 1;\nfunction f(x) { return x; }\nundefinedName;\n"),
    ]);
    let project = surge_ts::Project::load(dir.join("tsconfig.json"));
    let cli = project.check(&surge_ts::ProjectOptions::default()).unwrap();
    let text = std::fs::read_to_string(dir.join("tsconfig.json")).unwrap();
    let json = surge_ts::api::parse_config_file_text("tsconfig.json", &text).unwrap();
    let parsed = parse_json_config_file_content(&json, &dir.to_string_lossy(), None, Some("tsconfig.json"));
    let program = Program::create(CreateProgramOptions {
        root_names: parsed.file_names,
        options: parsed.options,
        ..Default::default()
    })
    .unwrap();
    let api: Vec<(u32, Option<u32>)> =
        program.pre_emit_diagnostics(None).into_iter().map(|diagnostic| (diagnostic.code, diagnostic.start)).collect();
    let mut expected: Vec<(u32, Option<u32>)> = cli
        .diagnostics
        .iter()
        .map(|diagnostic| {
            let code = diagnostic.code.to_string().trim_start_matches("TS").parse().unwrap();
            (code, diagnostic.span.map(|span| span.start as u32))
        })
        .collect();
    expected.sort();
    let mut api_sorted = api.clone();
    api_sorted.sort();
    assert_eq!(api_sorted, expected);
    assert!(api.iter().any(|(code, _)| *code == 7006));
}

#[test]
fn parses_configs_as_the_cli_does() {
    let dir = project(&[
        ("base.json", r#"{ "compilerOptions": { "strict": true, "target": "ES2022" } }"#),
        ("tsconfig.json", "{ // comments are allowed\n \"extends\": \"./base.json\", \"compilerOptions\": { \"module\": \"NodeNext\", \"lib\": [\"ES2022\", \"DOM\"] }, \"include\": [\"src\"], }"),
        ("src/a.ts", "export {};\n"),
        ("src/b.tsx", "export {};\n"),
    ]);
    let text = std::fs::read_to_string(dir.join("tsconfig.json")).unwrap();
    let json = surge_ts::api::parse_config_file_text("tsconfig.json", &text).unwrap();
    let parsed = parse_json_config_file_content(&json, &dir.to_string_lossy(), None, Some("tsconfig.json"));
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    assert_eq!(parsed.options.get("strict"), Some(&serde_json::json!(true)));
    assert_eq!(parsed.options.get("target"), Some(&serde_json::json!(9)));
    assert_eq!(parsed.options.get("module"), Some(&serde_json::json!(199)));
    assert_eq!(parsed.options.get("lib"), Some(&serde_json::json!(["lib.es2022.d.ts", "lib.dom.d.ts"])));
    assert!(parsed.options.get("configFilePath").is_some());
    assert_eq!(parsed.file_names.len(), 2);

    let mut unknown = CompilerOptions::new();
    unknown.set("notAnOption", serde_json::Value::Bool(true));
    let program = Program::create(CreateProgramOptions {
        root_names: vec![dir.join("src/a.ts").to_string_lossy().into_owned()],
        options: unknown,
        ..Default::default()
    })
    .unwrap();
    let config = program.config_file_parsing_diagnostics();
    assert!(
        config.iter().any(|diagnostic| diagnostic.surge_code.as_deref() == Some("UnknownCompilerOption")),
        "an option surge does not know is reported, not ignored: {config:?}"
    );
}

#[test]
fn resolves_module_names_with_the_loader() {
    let dir = project(&[
        ("src/index.ts", "export {};\n"),
        ("src/util/index.ts", "export {};\n"),
        ("node_modules/pkg/package.json", r#"{ "name": "pkg", "types": "./types.d.ts" }"#),
        ("node_modules/pkg/types.d.ts", "export declare const x: number;\n"),
    ]);
    let containing = dir.join("src/index.ts").to_string_lossy().into_owned();
    let relative = resolve_module_name("./util", &containing, &strict()).expect("relative directory import");
    assert!(relative.resolved_file_name.ends_with("/src/util/index.ts"));
    assert_eq!(relative.extension, ".ts");
    assert!(!relative.is_external_library_import);
    let package = resolve_module_name("pkg", &containing, &strict()).expect("package import");
    assert!(package.resolved_file_name.ends_with("/node_modules/pkg/types.d.ts"));
    assert_eq!(package.extension, ".d.ts");
    assert!(package.is_external_library_import);
    assert!(resolve_module_name("./missing", &containing, &strict()).is_none());
}

/// A host over files held in memory.
struct MemoryHost {
    files: HashMap<String, String>,
}

impl CompilerHost for MemoryHost {
    fn read_file(&self, path: &str) -> Option<String> {
        self.files.get(path).cloned()
    }
    fn file_exists(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }
    fn directory_exists(&self, path: &str) -> bool {
        let prefix = format!("{}/", path.trim_end_matches('/'));
        self.files.keys().any(|file| file.starts_with(&prefix))
    }
    fn read_directory_entries(&self, _path: &str) -> Option<Vec<DirectoryEntry>> {
        None
    }
    fn current_directory(&self) -> String {
        "/virtual".to_string()
    }
}

#[test]
fn reads_a_program_through_a_host() {
    let files = HashMap::from([
        ("/virtual/main.ts".to_string(), "import { two } from './two';\nconst s: string = two;\n".to_string()),
        ("/virtual/two.ts".to_string(), "export const two = 2;\n".to_string()),
    ]);
    let program = Program::create(CreateProgramOptions {
        root_names: vec!["main.ts".to_string()],
        options: strict(),
        host: Some(Arc::new(MemoryHost { files })),
        ..Default::default()
    })
    .expect("a host's files need not exist on disk");
    let names: Vec<String> = program.source_files().into_iter().map(|file| program.file_name(file).to_string()).collect();
    assert!(names.contains(&"/virtual/main.ts".to_string()), "{names:?}");
    assert!(names.contains(&"/virtual/two.ts".to_string()), "the host resolves imports too: {names:?}");
    let diagnostics = program.semantic_diagnostics(None);
    assert_eq!(diagnostics.iter().map(|diagnostic| diagnostic.code).collect::<Vec<_>>(), [2322]);
}

#[test]
fn parses_a_source_file_on_its_own() {
    let program = Program::parse_source_file("/standalone.ts", "let x = 1 +;\nclass A { m() {} }\n");
    let file = program.source_files()[0];
    assert_eq!(program.node_kind(program.root(file)), SyntaxKind::SourceFile);
    assert!(program.has_syntax_errors());
    assert!(!program.syntactic_diagnostics(Some(file)).is_empty());
    let checker: TypeChecker = program.type_checker();
    let x = identifier(&program, file, "x", 0);
    assert_eq!(checker.type_to_string(checker.type_at_location(x)), "any", "nothing is checked on its own");
}

#[test]
fn types_named_and_destructured() {
    let dir = project(&[(
        "index.ts",
        "declare const pattern: RegExp;\ndeclare const later: Promise<number>;\ntype Scores = Record<string, number>;\ninterface Show { show: (value: number) => string }\nexport function renamed({ show: render }: Show) { return render; }\nexport function local(o: { x: number; y: [boolean] }) { const { x, y: [flag] } = o; return x + Number(flag); }\nconst { a, b: [c] } = { a: 1, b: [true] as [boolean] };\nexport { pattern, later, a, c };\nexport type { Scores };\n",
    )]);
    let program = program(&dir, &["index.ts"]);
    let checker = program.type_checker();
    let file = program.source_file(&dir.join("index.ts").to_string_lossy()).unwrap();
    let at = |name: &str, occurrence: usize| checker.type_to_string(checker.type_at_location(identifier(&program, file, name, occurrence)));
    let actual: Vec<(&str, String)> = [("RegExp", 0), ("render", 0), ("x", 1), ("flag", 0), ("c", 0)]
        .into_iter()
        .map(|(name, occurrence)| (name, at(name, occurrence)))
        .collect();
    let expected: Vec<(&str, String)> = [
        ("RegExp", "RegExp"),
        ("render", "(value: number) => string"),
        ("x", "number"),
        ("flag", "boolean"),
        ("c", "boolean"),
    ]
    .into_iter()
    .map(|(name, ty)| (name, ty.to_string()))
    .collect();
    assert_eq!(actual, expected);
}

#[test]
fn declared_types_of_generic_declarations() {
    let dir = project(&[(
        "index.ts",
        "interface Box<T> { value: T }\ntype Pair<A> = [A, A];\ndeclare const m: Map<string, number>;\ndeclare const s: Set<string>;\ndeclare const a: Array<number>;\ndeclare const b: Box<number>;\ndeclare const p: Pair<number>;\ndeclare const q: Partial<Box<number>>;\nexport { m, s, a, b, p, q };\n",
    )]);
    let program = program(&dir, &["index.ts"]);
    let checker = program.type_checker();
    let file = program.source_file(&dir.join("index.ts").to_string_lossy()).unwrap();
    let at = |name: &str, occurrence: usize| checker.type_to_string(checker.type_at_location(identifier(&program, file, name, occurrence)));
    let actual: Vec<(&str, String)> = [("Box", 1), ("Map", 0), ("Set", 0)]
        .into_iter()
        .map(|(name, occurrence)| (name, at(name, occurrence)))
        .collect();
    let expected: Vec<(&str, String)> = [("Box", "Box<T>"), ("Map", "Map<K, V>"), ("Set", "Set<T>")]
    .into_iter()
    .map(|(name, ty)| (name, ty.to_string()))
    .collect();
    assert_eq!(actual, expected);
}

#[test]
fn types_derived_from_a_type_are_one_handle_each() {
    let dir = project(&[(
        "index.ts",
        "interface Box { value: number }\ninterface Tag { tag: string }\ndeclare const union: Box | Tag | string;\ndeclare const both: Box & Tag;\ndeclare const maybe: Box | undefined;\ndeclare const literals: 'a' | 'b' | 1;\nfunction pick<T extends Box, U>(first: T, second: T | number, fourth: U): T { return first; }\nexport {};\n",
    )]);
    let program = program(&dir, &["index.ts"]);
    let checker = program.type_checker();
    let file = program.source_file(&dir.join("index.ts").to_string_lossy()).unwrap();
    let at = |name: &str, occurrence: usize| checker.type_at_location(identifier(&program, file, name, occurrence));

    for name in ["union", "both", "maybe", "literals"] {
        let ty = at(name, 0);
        let constituents = checker.type_constituents(ty);
        assert!(!constituents.is_empty(), "{name} has constituents");
        assert_eq!(checker.type_constituents(ty), constituents, "{name}'s constituents");
    }
    let maybe = at("maybe", 0);
    let non_nullable = checker.non_nullable_type(maybe);
    assert_ne!(non_nullable, maybe);
    assert_eq!(checker.type_to_string(non_nullable), "Box");
    assert_eq!(checker.non_nullable_type(maybe), non_nullable);
    let literals = at("literals", 0);
    let base = checker.base_type_of_literal_type(literals);
    assert_eq!(checker.type_to_string(base), "string | number");
    assert_eq!(checker.base_type_of_literal_type(literals), base);

    for name in ["first", "second"] {
        let ty = at(name, 0);
        let constraint = checker.base_constraint_of_type(ty).unwrap();
        assert!(constraint.is_some_and(|constraint| constraint != ty), "{name} has a constraint of its own");
        assert_eq!(checker.base_constraint_of_type(ty).unwrap(), constraint, "{name}'s constraint");
    }

    let pick = checker.symbol_at_location(identifier(&program, file, "pick", 0)).unwrap();
    let signature = checker.signatures_of_type(checker.type_of_symbol(pick), SignatureKind::Call)[0];
    let parameters = checker.signature_type_parameters(signature);
    assert_eq!(parameters.iter().map(|parameter| checker.type_to_string(*parameter)).collect::<Vec<_>>(), ["T", "U"]);
    assert_eq!(checker.signature_type_parameters(signature), parameters);
}
