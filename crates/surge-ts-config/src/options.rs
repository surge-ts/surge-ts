#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TsConfigOptionSupport {
    Supported,
    KnownNoop,
    UnsupportedLegacy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TsConfigOptionValueKind {
    Boolean,
    String,
    StringArray,
    StringMapToStringArray,
    ObjectArray,
}

#[derive(Debug, Clone, Copy)]
pub struct TsConfigOptionDefinition {
    pub name: &'static str,
    pub value_kind: TsConfigOptionValueKind,
    pub support: TsConfigOptionSupport,
}

static TS_CONFIG_OPTION_DEFINITIONS: &[TsConfigOptionDefinition] = &[
    TsConfigOptionDefinition {
        name: "strict",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "noImplicitAny",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "useUnknownInCatchVariables",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "target",
        value_kind: TsConfigOptionValueKind::String,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "module",
        value_kind: TsConfigOptionValueKind::String,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "moduleResolution",
        value_kind: TsConfigOptionValueKind::String,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "jsx",
        value_kind: TsConfigOptionValueKind::String,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "jsxFactory",
        value_kind: TsConfigOptionValueKind::String,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "jsxFragmentFactory",
        value_kind: TsConfigOptionValueKind::String,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "reactNamespace",
        value_kind: TsConfigOptionValueKind::String,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "jsxImportSource",
        value_kind: TsConfigOptionValueKind::String,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "allowJs",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "checkJs",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "noEmit",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "noCheck",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "noResolve",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "skipLibCheck",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "paths",
        value_kind: TsConfigOptionValueKind::StringMapToStringArray,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "typeRoots",
        value_kind: TsConfigOptionValueKind::StringArray,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "types",
        value_kind: TsConfigOptionValueKind::StringArray,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "resolvePackageJsonExports",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "resolvePackageJsonImports",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "customConditions",
        value_kind: TsConfigOptionValueKind::StringArray,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "rootDir",
        value_kind: TsConfigOptionValueKind::String,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "outDir",
        value_kind: TsConfigOptionValueKind::String,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "declarationDir",
        value_kind: TsConfigOptionValueKind::String,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "declaration",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "declarationMap",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "emitDeclarationOnly",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "sourceMap",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "inlineSourceMap",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "removeComments",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "experimentalDecorators",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "emitDecoratorMetadata",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "pretty",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "noErrorTruncation",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "allowImportingTsExtensions",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "isolatedDeclarations",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "importHelpers",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "isolatedModules",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "preserveConstEnums",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "verbatimModuleSyntax",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "moduleDetection",
        value_kind: TsConfigOptionValueKind::String,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "resolveJsonModule",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "esModuleInterop",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "allowSyntheticDefaultImports",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "allowUmdGlobalAccess",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "forceConsistentCasingInFileNames",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "noUncheckedIndexedAccess",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "exactOptionalPropertyTypes",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "strictNullChecks",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "strictFunctionTypes",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "strictBindCallApply",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "strictBuiltinIteratorReturn",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "strictPropertyInitialization",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "noImplicitThis",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "alwaysStrict",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "noImplicitReturns",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "noFallthroughCasesInSwitch",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "noUnusedLocals",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "noUnusedParameters",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "allowUnreachableCode",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "allowUnusedLabels",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "noImplicitOverride",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "noPropertyAccessFromIndexSignature",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "noUncheckedSideEffectImports",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "useDefineForClassFields",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "erasableSyntaxOnly",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "newLine",
        value_kind: TsConfigOptionValueKind::String,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "stripInternal",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "noEmitOnError",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "noLib",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "lib",
        value_kind: TsConfigOptionValueKind::StringArray,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "plugins",
        value_kind: TsConfigOptionValueKind::ObjectArray,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "incremental",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "composite",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "tsBuildInfoFile",
        value_kind: TsConfigOptionValueKind::String,
        support: TsConfigOptionSupport::KnownNoop,
    },
    TsConfigOptionDefinition {
        name: "baseUrl",
        value_kind: TsConfigOptionValueKind::String,
        support: TsConfigOptionSupport::Supported,
    },
    TsConfigOptionDefinition {
        name: "downlevelIteration",
        value_kind: TsConfigOptionValueKind::Boolean,
        support: TsConfigOptionSupport::UnsupportedLegacy,
    },
    TsConfigOptionDefinition {
        name: "outFile",
        value_kind: TsConfigOptionValueKind::String,
        support: TsConfigOptionSupport::UnsupportedLegacy,
    },
];

pub fn find_tsconfig_option(name: &str) -> Option<&'static TsConfigOptionDefinition> {
    TS_CONFIG_OPTION_DEFINITIONS
        .iter()
        .find(|definition| definition.name == name)
}
