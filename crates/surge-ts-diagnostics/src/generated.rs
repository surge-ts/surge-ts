//! Generated diagnostic catalog. Do not edit by hand.

use crate::{
    Diagnostic, DiagnosticArg, DiagnosticCategory, DiagnosticDescriptor, DiagnosticSource,
    DiagnosticSupport,
};

pub const TS1029: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1029",
    number: Some(1029),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' modifier must precede '{1}' modifier.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS1281: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1281",
    number: Some(1281),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot access '{0}' from another file without qualification when '{1}' is enabled. Use '{2}' instead.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2411: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2411",
    number: Some(2411),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Property '{0}' of type '{1}' is not assignable to '{2}' index type '{3}'.",
    argument_count: 4,
    support: DiagnosticSupport::Emitted,
};

pub const TS2413: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2413",
    number: Some(2413),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' index type '{1}' is not assignable to '{2}' index type '{3}'.",
    argument_count: 4,
    support: DiagnosticSupport::Emitted,
};

pub const TS5112: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS5112",
    number: Some(5112),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "tsconfig.json is present but will not be loaded if files are specified on commandline. Use '--ignoreConfig' to skip this error.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS5102: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS5102",
    number: Some(5102),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Option '{0}' has been removed. Please remove it from your configuration.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS5097: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS5097",
    number: Some(5097),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An import path can only end with a '{0}' extension when 'allowImportingTsExtensions' is enabled.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS5108: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS5108",
    number: Some(5108),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Option '{0}={1}' has been removed. Please remove it from your configuration.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS6046: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS6046",
    number: Some(6046),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Argument for '{0}' option must be: {1}.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS1360: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1360",
    number: Some(1360),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type '{0}' does not satisfy the expected type '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS1379: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1379",
    number: Some(1379),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An import alias cannot reference a declaration that was exported using 'export type'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1380: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1380",
    number: Some(1380),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An import alias cannot reference a declaration that was imported using 'import type'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1361: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1361",
    number: Some(1361),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' cannot be used as a value because it was imported using 'import type'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1362: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1362",
    number: Some(1362),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' cannot be used as a value because it was exported using 'export type'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1063: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1063",
    number: Some(1063),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An export assignment cannot be used in a namespace.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1319: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1319",
    number: Some(1319),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A default export can only be used in an ECMAScript-style module.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2302: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2302",
    number: Some(2302),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Static members cannot reference class type parameters.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2304: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2304",
    number: Some(2304),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot find name '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2300: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2300",
    number: Some(2300),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Duplicate identifier '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2303: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2303",
    number: Some(2303),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Circular definition of import alias '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2706: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2706",
    number: Some(2706),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Required type parameters may not follow optional type parameters.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2717: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2717",
    number: Some(2717),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Subsequent property declarations must have the same type.  Property '{0}' must be of type '{1}', but here has type '{2}'.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2305: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2305",
    number: Some(2305),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Module '{0}' has no exported member '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2610: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2610",
    number: Some(2610),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' is defined as an accessor in class '{1}', but is overridden here in '{2}' as an instance property.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2611: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2611",
    number: Some(2611),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' is defined as a property in class '{1}', but is overridden here in '{2}' as an accessor.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2323: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2323",
    number: Some(2323),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot redeclare exported variable '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2484: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2484",
    number: Some(2484),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Export declaration conflicts with exported declaration of '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2341: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2341",
    number: Some(2341),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Property '{0}' is private and only accessible within class '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2445: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2445",
    number: Some(2445),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Property '{0}' is protected and only accessible within class '{1}' and its subclasses.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2440: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2440",
    number: Some(2440),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Import declaration conflicts with local declaration of '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2613: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2613",
    number: Some(2613),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Module '{0}' has no default export. Did you mean to use 'import {1} from {0}' instead?",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2595: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2595",
    number: Some(2595),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' can only be imported by using a default import.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2597: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2597",
    number: Some(2597),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' can only be imported by using a 'require' call or by using a default import.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2616: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2616",
    number: Some(2616),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' can only be imported by using 'import {1} = require({2})' or a default import.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2701: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2701",
    number: Some(2701),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The target of an object rest assignment must be a variable or a property access.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1288: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1288",
    number: Some(1288),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An import alias cannot resolve to a type or type-only declaration when 'verbatimModuleSyntax' is enabled.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2850: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2850",
    number: Some(2850),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The initializer of a 'using' declaration must be either an object with a '[Symbol.dispose]()' method, or be 'null' or 'undefined'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2851: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2851",
    number: Some(2851),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The initializer of an 'await using' declaration must be either an object with a '[Symbol.asyncDispose]()' or '[Symbol.dispose]()' method, or be 'null' or 'undefined'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2614: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2614",
    number: Some(2614),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Module '{0}' has no exported member '{1}'. Did you mean to use 'import {1} from {0}' instead?",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2306: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2306",
    number: Some(2306),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "File '{0}' is not a module.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2307: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2307",
    number: Some(2307),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot find module '{0}' or its corresponding type declarations.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2732: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2732",
    number: Some(2732),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot find module '{0}'. Consider using '--resolveJsonModule' to import module with '.json' extension.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2882: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2882",
    number: Some(2882),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot find module or type declarations for side-effect import of '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2314: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2314",
    number: Some(2314),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Generic type '{0}' requires {1} type argument(s).",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2707: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2707",
    number: Some(2707),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Generic type '{0}' requires between {1} and {2} type arguments.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2315: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2315",
    number: Some(2315),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type '{0}' is not generic.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2322: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2322",
    number: Some(2322),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type '{0}' is not assignable to type '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2418: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2418",
    number: Some(2418),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type of computed property's value is '{0}', which is not assignable to type '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2808: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2808",
    number: Some(2808),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A get accessor must be at least as accessible as the setter",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2820: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2820",
    number: Some(2820),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type '{0}' is not assignable to type '{1}'. Did you mean '{2}'?",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS18043: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18043",
    number: Some(18043),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Types cannot appear in export declarations in JavaScript files.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS18046: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18046",
    number: Some(18046),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' is of type 'unknown'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2532: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2532",
    number: Some(2532),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Object is possibly 'undefined'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2531: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2531",
    number: Some(2531),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Object is possibly 'null'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2533: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2533",
    number: Some(2533),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Object is possibly 'null' or 'undefined'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS18047: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18047",
    number: Some(18047),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' is possibly 'null'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS18048: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18048",
    number: Some(18048),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' is possibly 'undefined'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS18049: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18049",
    number: Some(18049),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' is possibly 'null' or 'undefined'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2571: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2571",
    number: Some(2571),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Object is of type 'unknown'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS18050: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18050",
    number: Some(18050),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The value '{0}' cannot be used here.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2339: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2339",
    number: Some(2339),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Property '{0}' does not exist on type '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2344: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2344",
    number: Some(2344),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type '{0}' does not satisfy the constraint '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2345: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2345",
    number: Some(2345),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Argument of type '{0}' is not assignable to parameter of type '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2347: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2347",
    number: Some(2347),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Untyped function calls may not accept type arguments.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2349: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2349",
    number: Some(2349),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This expression is not callable.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2351: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2351",
    number: Some(2351),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This expression is not constructable.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2352: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2352",
    number: Some(2352),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Conversion of type '{0}' to type '{1}' may be a mistake because neither type sufficiently overlaps with the other. If this was intentional, convert the expression to 'unknown' first.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2353: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2353",
    number: Some(2353),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Object literal may only specify known properties, and '{0}' does not exist in type '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2355: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2355",
    number: Some(2355),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A function whose declared type is neither 'undefined', 'void', nor 'any' must return a value.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2356: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2356",
    number: Some(2356),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An arithmetic operand must be of type 'any', 'number', 'bigint' or an enum type.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2362: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2362",
    number: Some(2362),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The left-hand side of an arithmetic operation must be of type 'any', 'number', 'bigint' or an enum type.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2363: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2363",
    number: Some(2363),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The right-hand side of an arithmetic operation must be of type 'any', 'number', 'bigint' or an enum type.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2365: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2365",
    number: Some(2365),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Operator '{0}' cannot be applied to types '{1}' and '{2}'.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2366: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2366",
    number: Some(2366),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Function lacks ending return statement and return type does not include 'undefined'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2367: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2367",
    number: Some(2367),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This comparison appears to be unintentional because the types '{0}' and '{1}' have no overlap.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2393: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2393",
    number: Some(2393),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Duplicate function implementation.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2394: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2394",
    number: Some(2394),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This overload signature is not compatible with its implementation signature.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2448: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2448",
    number: Some(2448),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Block-scoped variable '{0}' used before its declaration.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2451: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2451",
    number: Some(2451),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot redeclare block-scoped variable '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2714: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2714",
    number: Some(2714),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The expression of an export assignment must be an identifier or qualified name in an ambient context.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2454: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2454",
    number: Some(2454),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Variable '{0}' is used before being assigned.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2493: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2493",
    number: Some(2493),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Tuple type '{0}' of length '{1}' has no element at index '{2}'.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2540: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2540",
    number: Some(2540),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot assign to '{0}' because it is a read-only property.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2542: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2542",
    number: Some(2542),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Index signature in type '{0}' only permits reading.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2514: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2514",
    number: Some(2514),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A tuple type cannot be indexed with a negative value.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS7015: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7015",
    number: Some(7015),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Element implicitly has an 'any' type because index expression is not of type 'number'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2862: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2862",
    number: Some(2862),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type '{0}' is generic and can only be indexed for reading.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS4094: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS4094",
    number: Some(4094),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Property '{0}' of exported anonymous class type may not be private or protected.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS4104: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS4104",
    number: Some(4104),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The type '{0}' is 'readonly' and cannot be assigned to the mutable type '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2536: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2536",
    number: Some(2536),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type '{0}' cannot be used to index type '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2537: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2537",
    number: Some(2537),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type '{0}' has no matching index signature for type '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2538: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2538",
    number: Some(2538),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type '{0}' cannot be used as an index type.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2550: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2550",
    number: Some(2550),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Property '{0}' does not exist on type '{1}'. Do you need to change your target library? Try changing the 'lib' compiler option to '{2}' or later.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2551: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2551",
    number: Some(2551),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Property '{0}' does not exist on type '{1}'. Did you mean '{2}'?",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2812: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2812",
    number: Some(2812),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Property '{0}' does not exist on type '{1}'. Try changing the 'lib' compiler option to include 'dom'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2552: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2552",
    number: Some(2552),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot find name '{0}'. Did you mean '{1}'?",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2554: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2554",
    number: Some(2554),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Expected {0} arguments, but got {1}.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2555: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2555",
    number: Some(2555),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Expected at least {0} arguments, but got {1}.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2556: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2556",
    number: Some(2556),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A spread argument must either have a tuple type or be passed to a rest parameter.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2561: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2561",
    number: Some(2561),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Object literal may only specify known properties, but '{0}' does not exist in type '{1}'. Did you mean to write '{2}'?",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2576: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2576",
    number: Some(2576),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Property '{0}' does not exist on type '{1}'. Did you mean to access the static member '{2}' instead?",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2588: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2588",
    number: Some(2588),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot assign to '{0}' because it is a constant.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2580: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2580",
    number: Some(2580),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot find name '{0}'. Do you need to install type definitions for node? Try `npm i --save-dev @types/node`.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2591: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2591",
    number: Some(2591),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot find name '{0}'. Do you need to install type definitions for node? Try `npm i --save-dev @types/node` and then add 'node' to the types field in your tsconfig.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2688: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2688",
    number: Some(2688),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot find type definition file for '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2693: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2693",
    number: Some(2693),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' only refers to a type, but is being used as a value here.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2686: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2686",
    number: Some(2686),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' refers to a UMD global, but the current file is a module. Consider adding an import instead.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2741: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2741",
    number: Some(2741),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Property '{0}' is missing in type '{1}' but required in type '{2}'.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2745: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2745",
    number: Some(2745),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This JSX tag's '{0}' prop expects type '{1}' which requires multiple children, but only a single child was provided.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2746: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2746",
    number: Some(2746),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This JSX tag's '{0}' prop expects a single child of type '{1}', but multiple children were provided.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2747: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2747",
    number: Some(2747),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' components don't accept text as child elements. Text in JSX has the type 'string', but the expected type of '{1}' is '{2}'.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2875: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2875",
    number: Some(2875),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This JSX tag requires the module path '{0}' to exist, but none could be found. Make sure you have types for the appropriate package installed.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2754: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2754",
    number: Some(2754),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'super' may not use type arguments.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2749: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2749",
    number: Some(2749),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' refers to a value, but is being used as a type here. Did you mean 'typeof {0}'?",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2869: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2869",
    number: Some(2869),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Right operand of ?? is unreachable because the left operand is never nullish.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2447: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2447",
    number: Some(2447),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The '{0}' operator is not allowed for boolean types. Consider using '{1}' instead.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2577: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2577",
    number: Some(2577),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Return type annotation circularly references itself.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2857: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2857",
    number: Some(2857),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Import attributes cannot be used with type-only imports or exports.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS7012: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7012",
    number: Some(7012),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This overload implicitly returns the type '{0}' because it lacks a return type annotation.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2456: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2456",
    number: Some(2456),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type alias '{0}' circularly references itself.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2459: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2459",
    number: Some(2459),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Module '{0}' declares '{1}' locally, but it is not exported.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2469: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2469",
    number: Some(2469),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The '{0}' operator cannot be applied to type 'symbol'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2632: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2632",
    number: Some(2632),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot assign to '{0}' because it is an import.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2731: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2731",
    number: Some(2731),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Implicit conversion of a 'symbol' to a 'string' will fail at runtime. Consider wrapping this expression in 'String(...)'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2736: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2736",
    number: Some(2736),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Operator '{0}' cannot be applied to type '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2769: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2769",
    number: Some(2769),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "No overload matches this call.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2774: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2774",
    number: Some(2774),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This condition will always return true since this function is always defined. Did you mean to call it instead?",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2839: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2839",
    number: Some(2839),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This condition will always return '{0}' since JavaScript compares objects by reference, not value.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2845: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2845",
    number: Some(2845),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This condition will always return '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2871: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2871",
    number: Some(2871),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This expression is always nullish.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2872: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2872",
    number: Some(2872),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This kind of expression is always truthy.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2873: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2873",
    number: Some(2873),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This kind of expression is always falsy.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS7005: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7005",
    number: Some(7005),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Variable '{0}' implicitly has an '{1}' type.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS7006: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7006",
    number: Some(7006),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Parameter '{0}' implicitly has an 'any' type.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS7016: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7016",
    number: Some(7016),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Could not find a declaration file for module '{0}'. '{1}' implicitly has an 'any' type.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS7018: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7018",
    number: Some(7018),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Object literal's property '{0}' implicitly has an '{1}' type.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS7019: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7019",
    number: Some(7019),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Rest parameter '{0}' implicitly has an 'any[]' type.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS7022: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7022",
    number: Some(7022),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' implicitly has type 'any' because it does not have a type annotation and is referenced directly or indirectly in its own initializer.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS4111: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS4111",
    number: Some(4111),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Property '{0}' comes from an index signature, so it must be accessed with ['{0}'].",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1106: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1106",
    number: Some(1106),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The left-hand side of a 'for...of' statement may not be 'async'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1121: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1121",
    number: Some(1121),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Octal literals are not allowed. Use the syntax '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1489: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1489",
    number: Some(1489),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Decimals with leading zeros are not allowed.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS6133: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS6133",
    number: Some(6133),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' is declared but its value is never read.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS6142: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS6142",
    number: Some(6142),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Module '{0}' was resolved to '{1}', but '--jsx' is not set.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS6192: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS6192",
    number: Some(6192),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "All imports in import declaration are unused.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS6198: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS6198",
    number: Some(6198),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "All destructured elements are unused.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS6199: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS6199",
    number: Some(6199),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "All variables are unused.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS6196: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS6196",
    number: Some(6196),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' is declared but never used.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS4112: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS4112",
    number: Some(4112),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This member cannot have an 'override' modifier because its containing class '{0}' does not extend another class.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS4113: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS4113",
    number: Some(4113),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This member cannot have an 'override' modifier because it is not declared in the base class '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS4114: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS4114",
    number: Some(4114),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This member must have an 'override' modifier because it overrides a member in the base class '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS4115: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS4115",
    number: Some(4115),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This parameter property must have an 'override' modifier because it overrides a member in base class '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS4116: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS4116",
    number: Some(4116),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This member must have an 'override' modifier because it overrides an abstract method that is declared in the base class '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS4117: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS4117",
    number: Some(4117),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This member cannot have an 'override' modifier because it is not declared in the base class '{0}'. Did you mean '{1}'?",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS4119: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS4119",
    number: Some(4119),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This member must have a JSDoc comment with an '@override' tag because it overrides a member in the base class '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS4121: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS4121",
    number: Some(4121),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This member cannot have a JSDoc comment with an '@override' tag because its containing class '{0}' does not extend another class.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS4122: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS4122",
    number: Some(4122),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This member cannot have a JSDoc comment with an '@override' tag because it is not declared in the base class '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS4123: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS4123",
    number: Some(4123),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This member cannot have a JSDoc comment with an 'override' tag because it is not declared in the base class '{0}'. Did you mean '{1}'?",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS4127: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS4127",
    number: Some(4127),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This member cannot have an 'override' modifier because its name is dynamic.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS4128: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS4128",
    number: Some(4128),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This member cannot have a JSDoc comment with an '@override' tag because its name is dynamic.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS7023: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7023",
    number: Some(7023),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' implicitly has return type 'any' because it does not have a return type annotation and is referenced directly or indirectly in one of its return expressions.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS7029: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7029",
    number: Some(7029),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Fallthrough case in switch.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS7030: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7030",
    number: Some(7030),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Not all code paths return a value.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2534: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2534",
    number: Some(2534),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A function returning 'never' cannot have a reachable end point.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS7031: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7031",
    number: Some(7031),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Binding element '{0}' implicitly has an '{1}' type.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS7034: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7034",
    number: Some(7034),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Variable '{0}' implicitly has type '{1}' in some locations where its type cannot be determined.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS7051: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7051",
    number: Some(7051),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Parameter has a name but no type. Did you mean '{0}: {1}'?",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS7052: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7052",
    number: Some(7052),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Element implicitly has an 'any' type because type '{0}' has no index signature. Did you mean to call '{1}'?",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS7053: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7053",
    number: Some(7053),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Element implicitly has an 'any' type because expression of type '{0}' can't be used to index type '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS7054: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7054",
    number: Some(7054),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "No index signature with a parameter of type '{0}' was found on type '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::CatalogOnly,
};

pub const TS7055: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7055",
    number: Some(7055),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}', which lacks return-type annotation, implicitly has an '{1}' yield type.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS7025: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7025",
    number: Some(7025),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Generator implicitly has yield type '{0}'. Consider supplying a return type annotation.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS7056: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7056",
    number: Some(7056),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The inferred type of this node exceeds the maximum length the compiler will serialize. An explicit type annotation is needed.",
    argument_count: 0,
    support: DiagnosticSupport::CatalogOnly,
};

pub const TS7057: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7057",
    number: Some(7057),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'yield' expression implicitly results in an 'any' type because its containing generator lacks a return-type annotation.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS7058: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7058",
    number: Some(7058),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "If the '{0}' package actually exposes this module, try adding a new declaration (.d.ts) file containing `declare module '{1}';`",
    argument_count: 2,
    support: DiagnosticSupport::CatalogOnly,
};

pub const TS7059: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7059",
    number: Some(7059),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This syntax is reserved in files with the .mts or .cts extension. Use an `as` expression instead.",
    argument_count: 0,
    support: DiagnosticSupport::CatalogOnly,
};

pub const TS7060: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7060",
    number: Some(7060),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This syntax is reserved in files with the .mts or .cts extension. Add a trailing comma or explicit constraint.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS7061: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7061",
    number: Some(7061),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A mapped type may not declare properties or methods.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1117: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1117",
    number: Some(1117),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An object literal cannot have multiple properties with the same name.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1155: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1155",
    number: Some(1155),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' declarations must be initialized.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2378: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2378",
    number: Some(2378),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A 'get' accessor must return a value.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2390: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2390",
    number: Some(2390),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Constructor implementation is missing.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2391: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2391",
    number: Some(2391),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Function implementation is missing or not immediately following the declaration.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2528: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2528",
    number: Some(2528),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A module cannot have multiple default exports.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2739: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2739",
    number: Some(2739),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type '{0}' is missing the following properties from type '{1}': {2}",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2740: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2740",
    number: Some(2740),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type '{0}' is missing the following properties from type '{1}': {2}, and {3} more.",
    argument_count: 4,
    support: DiagnosticSupport::Emitted,
};

pub const TS7008: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7008",
    number: Some(7008),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Member '{0}' implicitly has an '{1}' type.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS7013: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7013",
    number: Some(7013),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Construct signature, which lacks return-type annotation, implicitly has an 'any' return type.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS7020: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7020",
    number: Some(7020),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Call signature, which lacks return-type annotation, implicitly has an 'any' return type.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS7010: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7010",
    number: Some(7010),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}', which lacks return-type annotation, implicitly has an '{1}' return type.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2377: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2377",
    number: Some(2377),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Constructors for derived classes must contain a 'super' call.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2392: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2392",
    number: Some(2392),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Multiple constructor implementations are not allowed.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2695: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2695",
    number: Some(2695),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Left side of comma operator is unused and has no side effects.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1015: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1015",
    number: Some(1015),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Parameter cannot have question mark and initializer.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1016: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1016",
    number: Some(1016),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A required parameter cannot follow an optional parameter.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1039: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1039",
    number: Some(1039),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Initializers are not allowed in ambient contexts.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1049: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1049",
    number: Some(1049),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A 'set' accessor must have exactly one parameter.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1095: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1095",
    number: Some(1095),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A 'set' accessor cannot have a return type annotation.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1119: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1119",
    number: Some(1119),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An object literal cannot have property and accessor with the same name.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1192: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1192",
    number: Some(1192),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Module '{0}' has no default export.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1244: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1244",
    number: Some(1244),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Abstract methods can only appear within an abstract class.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1253: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1253",
    number: Some(1253),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Abstract properties can only appear within an abstract class.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2369: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2369",
    number: Some(2369),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A parameter property is only allowed in a constructor implementation.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2371: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2371",
    number: Some(2371),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A parameter initializer is only allowed in a function or constructor implementation.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS17009: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS17009",
    number: Some(17009),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'super' must be called before accessing 'this' in the constructor of a derived class.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS17011: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS17011",
    number: Some(17011),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'super' must be called before accessing a property of 'super' in the constructor of a derived class.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS18004: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18004",
    number: Some(18004),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "No value exists in scope for the shorthand property '{0}'. Either declare one or provide an initializer.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2676: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2676",
    number: Some(2676),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Accessors must both be abstract or non-abstract.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2678: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2678",
    number: Some(2678),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type '{0}' is not comparable to type '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2515: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2515",
    number: Some(2515),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Non-abstract class '{0}' does not implement inherited abstract member {1} from class '{2}'.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2653: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2653",
    number: Some(2653),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Non-abstract class expression does not implement inherited abstract member '{0}' from class '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2654: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2654",
    number: Some(2654),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Non-abstract class '{0}' is missing implementations for the following members of '{1}': {2}.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2655: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2655",
    number: Some(2655),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Non-abstract class '{0}' is missing implementations for the following members of '{1}': {2} and {3} more.",
    argument_count: 4,
    support: DiagnosticSupport::Emitted,
};

pub const TS2511: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2511",
    number: Some(2511),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot create an instance of an abstract class.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2420: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2420",
    number: Some(2420),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Class '{0}' incorrectly implements interface '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const SURGE_PARSER_ERROR: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "surge::parser-error",
    number: None,
    source: DiagnosticSource::TypeScriptRust,
    category: DiagnosticCategory::Error,
    message_template: "{0}",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const SURGE_DUPLICATE_TYPE_PARAMETER: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "surge::duplicate-type-parameter",
    number: None,
    source: DiagnosticSource::TypeScriptRust,
    category: DiagnosticCategory::Error,
    message_template: "Duplicate type parameter '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const SURGE_UNSUPPORTED_MODULE_SYNTAX: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "surge::unsupported-module-syntax",
    number: None,
    source: DiagnosticSource::TypeScriptRust,
    category: DiagnosticCategory::Error,
    message_template: "Unsupported module syntax.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const SURGE_UNSUPPORTED_DECLARATION: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "surge::unsupported-declaration",
    number: None,
    source: DiagnosticSource::TypeScriptRust,
    category: DiagnosticCategory::Error,
    message_template: "Unsupported declaration syntax.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const SURGE_TYPE_ALIAS_CYCLE: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "surge::type-alias-cycle",
    number: None,
    source: DiagnosticSource::TypeScriptRust,
    category: DiagnosticCategory::Error,
    message_template: "Type alias '{0}' circularly references itself.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const SURGE_TYPE_DECLARATION_CYCLE: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "surge::type-declaration-cycle",
    number: None,
    source: DiagnosticSource::TypeScriptRust,
    category: DiagnosticCategory::Error,
    message_template: "Type declaration '{0}' circularly references itself.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2703: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2703",
    number: Some(2703),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The operand of a 'delete' operator must be a property reference.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2704: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2704",
    number: Some(2704),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The operand of a 'delete' operator cannot be a read-only property.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2791: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2791",
    number: Some(2791),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Exponentiation cannot be performed on 'bigint' values unless the 'target' option is set to 'es2016' or later.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2790: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2790",
    number: Some(2790),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The operand of a 'delete' operator must be optional.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS18011: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18011",
    number: Some(18011),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The operand of a 'delete' operator cannot be a private identifier.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1102: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1102",
    number: Some(1102),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'delete' cannot be called on an identifier in strict mode.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2358: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2358",
    number: Some(2358),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The left-hand side of an 'instanceof' expression must be of type 'any', an object type or a type parameter.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2474: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2474",
    number: Some(2474),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "const enum member initializers must be constant expressions.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2488: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2488",
    number: Some(2488),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type '{0}' must have a '[Symbol.iterator]()' method that returns an iterator.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2504: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2504",
    number: Some(2504),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type '{0}' must have a '[Symbol.asyncIterator]()' method that returns an async iterator.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2763: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2763",
    number: Some(2763),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot iterate value because the 'next' method of its iterator expects type '{1}', but for-of will always send '{0}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2764: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2764",
    number: Some(2764),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot iterate value because the 'next' method of its iterator expects type '{1}', but array spread will always send '{0}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2765: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2765",
    number: Some(2765),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot iterate value because the 'next' method of its iterator expects type '{1}', but array destructuring will always send '{0}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2766: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2766",
    number: Some(2766),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot delegate iteration to value because the 'next' method of its iterator expects type '{1}', but the containing generator will always send '{0}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2698: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2698",
    number: Some(2698),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Spread types may only be created from object types.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2416: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2416",
    number: Some(2416),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Property '{0}' in type '{1}' is not assignable to the same property in base type '{2}'.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2423: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2423",
    number: Some(2423),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Class '{0}' defines instance member function '{1}', but extended class '{2}' defines it as instance member accessor.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2425: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2425",
    number: Some(2425),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Class '{0}' defines instance member property '{1}', but extended class '{2}' defines it as instance member function.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2426: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2426",
    number: Some(2426),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Class '{0}' defines instance member accessor '{1}', but extended class '{2}' defines it as instance member function.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2312: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2312",
    number: Some(2312),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An interface can only extend an object type or intersection of object types with statically known members.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2636: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2636",
    number: Some(2636),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type '{0}' is not assignable to type '{1}' as implied by variance annotation.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2590: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2590",
    number: Some(2590),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Expression produces a union type that is too complex to represent.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2430: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2430",
    number: Some(2430),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Interface '{0}' incorrectly extends interface '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2558: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2558",
    number: Some(2558),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Expected {0} type arguments, but got {1}.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2564: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2564",
    number: Some(2564),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Property '{0}' has no initializer and is not definitely assigned in the constructor.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2565: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2565",
    number: Some(2565),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Property '{0}' is used before being assigned.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2403: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2403",
    number: Some(2403),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Subsequent variable declarations must have the same type.  Variable '{0}' must be of type '{1}', but here has type '{2}'.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2449: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2449",
    number: Some(2449),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Class '{0}' used before its declaration.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2650: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2650",
    number: Some(2650),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Non-abstract class expression is missing implementations for the following members of '{0}': {1} and {2} more.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2651: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2651",
    number: Some(2651),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A member initializer in a enum declaration cannot reference members declared after it, including members defined in other enums.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2721: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2721",
    number: Some(2721),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot invoke an object which is possibly 'null'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2722: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2722",
    number: Some(2722),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot invoke an object which is possibly 'undefined'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2723: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2723",
    number: Some(2723),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot invoke an object which is possibly 'null' or 'undefined'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS7009: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7009",
    number: Some(7009),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'new' expression, whose target lacks a construct signature, implicitly has an 'any' type.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2350: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2350",
    number: Some(2350),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Only a void function can be called with the 'new' keyword.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2407: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2407",
    number: Some(2407),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The right-hand side of a 'for...in' statement must be of type 'any', an object type or a type parameter, but here has type '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2724: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2724",
    number: Some(2724),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' has no exported member named '{1}'. Did you mean '{2}'?",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2729: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2729",
    number: Some(2729),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Property '{0}' is used before its initialization.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2683: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2683",
    number: Some(2683),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'this' implicitly has type 'any' because it does not have a type annotation.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1019: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1019",
    number: Some(1019),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An index signature parameter cannot have a question mark.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1021: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1021",
    number: Some(1021),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An index signature must have a type annotation.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1024: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1024",
    number: Some(1024),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'readonly' modifier can only appear on a property declaration or index signature.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1028: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1028",
    number: Some(1028),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Accessibility modifier already seen.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1047: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1047",
    number: Some(1047),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A rest parameter cannot be optional.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1051: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1051",
    number: Some(1051),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A 'set' accessor cannot have an optional parameter.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1061: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1061",
    number: Some(1061),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Enum member must have initializer.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1064: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1064",
    number: Some(1064),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The return type of an async function or method must be the global Promise<T> type. Did you mean to write 'Promise<{0}>'?",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1066: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1066",
    number: Some(1066),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "In ambient enum declarations member initializer must be constant expression.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1092: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1092",
    number: Some(1092),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type parameters cannot appear on a constructor declaration.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1093: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1093",
    number: Some(1093),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type annotation cannot appear on a constructor declaration.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1096: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1096",
    number: Some(1096),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An index signature must have exactly one parameter.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1098: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1098",
    number: Some(1098),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type parameter list cannot be empty.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1099: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1099",
    number: Some(1099),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type argument list cannot be empty.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1108: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1108",
    number: Some(1108),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A 'return' statement can only be used within a function body.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1141: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1141",
    number: Some(1141),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "String literal expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1164: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1164",
    number: Some(1164),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Computed property names are not allowed in enums.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1172: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1172",
    number: Some(1172),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'extends' clause already seen.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1173: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1173",
    number: Some(1173),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'extends' clause must precede 'implements' clause.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1174: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1174",
    number: Some(1174),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Classes can only extend a single class.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1175: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1175",
    number: Some(1175),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'implements' clause already seen.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1176: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1176",
    number: Some(1176),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Interface declaration cannot have 'implements' clause.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1183: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1183",
    number: Some(1183),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An implementation cannot be declared in ambient contexts.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1184: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1184",
    number: Some(1184),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Modifiers cannot appear here.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1187: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1187",
    number: Some(1187),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A parameter property may not be declared using a binding pattern.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1249: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1249",
    number: Some(1249),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A decorator can only decorate a method implementation, not an overload.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1257: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1257",
    number: Some(1257),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A required element cannot follow an optional element.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1263: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1263",
    number: Some(1263),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Declarations with initializers cannot also have definite assignment assertions.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1264: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1264",
    number: Some(1264),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Declarations with definite assignment assertions must also have type annotations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1265: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1265",
    number: Some(1265),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A rest element cannot follow another rest element.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1266: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1266",
    number: Some(1266),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An optional element cannot follow a rest element.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1276: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1276",
    number: Some(1276),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An 'accessor' property cannot be declared optional.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1318: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1318",
    number: Some(1318),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An abstract accessor cannot have an implementation.",
    argument_count: 0,
    support: DiagnosticSupport::CatalogOnly,
};

pub const TS1354: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1354",
    number: Some(1354),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'readonly' type modifier is only permitted on array and tuple literal types.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1363: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1363",
    number: Some(1363),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A type-only import can specify a default import or named bindings, but not both.",
    argument_count: 0,
    support: DiagnosticSupport::CatalogOnly,
};

pub const TS1477: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1477",
    number: Some(1477),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An instantiation expression cannot be followed by a property access.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1490: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1490",
    number: Some(1490),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "File appears to be binary.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1502: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1502",
    number: Some(1502),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The Unicode (u) flag and the Unicode Sets (v) flag cannot be set simultaneously.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1545: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1545",
    number: Some(1545),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'using' declarations are not allowed in ambient contexts.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1546: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1546",
    number: Some(1546),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'await using' declarations are not allowed in ambient contexts.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2206: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2206",
    number: Some(2206),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The 'type' modifier cannot be used on a named import when 'import type' is used on its import statement.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2207: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2207",
    number: Some(2207),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The 'type' modifier cannot be used on a named export when 'export type' is used on its export statement.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2452: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2452",
    number: Some(2452),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An enum member cannot have a numeric name.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2499: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2499",
    number: Some(2499),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An interface can only extend an identifier/qualified-name with optional type arguments.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2566: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2566",
    number: Some(2566),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A rest element cannot have a property name.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2681: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2681",
    number: Some(2681),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A constructor cannot have a 'this' parameter.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2730: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2730",
    number: Some(2730),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An arrow function cannot have a 'this' parameter.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2784: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2784",
    number: Some(2784),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'get' and 'set' accessors cannot declare 'this' parameters.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS5085: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS5085",
    number: Some(5085),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A tuple member cannot be both optional and rest.",
    argument_count: 0,
    support: DiagnosticSupport::CatalogOnly,
};

pub const TS5086: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS5086",
    number: Some(5086),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A labeled tuple element is declared as optional with a question mark after the name and before the colon, rather than after the type.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS5087: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS5087",
    number: Some(5087),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A labeled tuple element is declared as rest with a '...' before the name, rather than before the type.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS8002: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS8002",
    number: Some(8002),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'import ... =' can only be used in TypeScript files.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS8005: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS8005",
    number: Some(8005),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'implements' clauses can only be used in TypeScript files.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS8012: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS8012",
    number: Some(8012),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Parameter modifiers can only be used in TypeScript files.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS8016: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS8016",
    number: Some(8016),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type assertion expressions can only be used in TypeScript files.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS8037: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS8037",
    number: Some(8037),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type satisfaction expressions can only be used in TypeScript files.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS17000: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS17000",
    number: Some(17000),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "JSX attributes must only be assigned a non-empty 'expression'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS18010: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18010",
    number: Some(18010),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An accessibility modifier cannot be used with a private identifier.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS18055: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18055",
    number: Some(18055),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' has a string type, but must have syntactically recognizable string syntax when 'isolatedModules' is enabled.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS18058: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18058",
    number: Some(18058),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Default imports are not allowed in a deferred import.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS18059: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18059",
    number: Some(18059),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Named imports are not allowed in a deferred import.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2311: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2311",
    number: Some(2311),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot find name '{0}'. Did you mean to write this in an async function?",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2503: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2503",
    number: Some(2503),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot find namespace '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2581: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2581",
    number: Some(2581),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot find name '{0}'. Do you need to install type definitions for jQuery? Try `npm i --save-dev @types/jquery`.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2582: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2582",
    number: Some(2582),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot find name '{0}'. Do you need to install type definitions for a test runner? Try `npm i --save-dev @types/jest` or `npm i --save-dev @types/mocha`.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2583: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2583",
    number: Some(2583),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot find name '{0}'. Do you need to change your target library? Try changing the 'lib' compiler option to '{1}' or later.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2584: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2584",
    number: Some(2584),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot find name '{0}'. Do you need to change your target library? Try changing the 'lib' compiler option to include 'dom'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2585: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2585",
    number: Some(2585),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' only refers to a type, but is being used as a value here. Do you need to change your target library? Try changing the 'lib' compiler option to es2015 or later.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2592: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2592",
    number: Some(2592),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot find name '{0}'. Do you need to install type definitions for jQuery? Try `npm i --save-dev @types/jquery` and then add 'jquery' to the types field in your tsconfig.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2593: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2593",
    number: Some(2593),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot find name '{0}'. Do you need to install type definitions for a test runner? Try `npm i --save-dev @types/jest` or `npm i --save-dev @types/mocha` and then add 'jest' or 'mocha' to the types field in your tsconfig.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2661: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2661",
    number: Some(2661),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot export '{0}'. Only local declarations can be exported from a module.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2662: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2662",
    number: Some(2662),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot find name '{0}'. Did you mean the static member '{1}.{0}'?",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2663: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2663",
    number: Some(2663),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot find name '{0}'. Did you mean the instance member 'this.{0}'?",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2301: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2301",
    number: Some(2301),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Initializer of instance member variable '{0}' cannot reference identifier '{1}' declared in the constructor.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2844: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2844",
    number: Some(2844),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type of instance member variable '{0}' cannot reference identifier '{1}' declared in the constructor.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2708: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2708",
    number: Some(2708),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot use namespace '{0}' as a value.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2713: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2713",
    number: Some(2713),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot access '{0}.{1}' because '{0}' is a type, but not a namespace. Did you mean to retrieve the type of the property '{1}' in '{0}' with '{0}[\"{1}\"]'?",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2865: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2865",
    number: Some(2865),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Import '{0}' conflicts with local value, so must be declared with a type-only import when 'isolatedModules' is enabled.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2867: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2867",
    number: Some(2867),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot find name '{0}'. Do you need to install type definitions for Bun? Try `npm i --save-dev @types/bun`.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2868: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2868",
    number: Some(2868),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot find name '{0}'. Do you need to install type definitions for Bun? Try `npm i --save-dev @types/bun` and then add 'bun' to the types field in your tsconfig.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2709: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2709",
    number: Some(2709),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot use namespace '{0}' as a type.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2702: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2702",
    number: Some(2702),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' only refers to a type, but is being used as a namespace here.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2833: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2833",
    number: Some(2833),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot find namespace '{0}'. Did you mean '{1}'?",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS1194: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1194",
    number: Some(1194),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Export declarations are not permitted in a namespace.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2694: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2694",
    number: Some(2694),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Namespace '{0}' has no exported member '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2689: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2689",
    number: Some(2689),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot extend an interface '{0}'. Did you mean 'implements'?",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1312: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1312",
    number: Some(1312),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Did you mean to use a ':'? An '=' can only follow a property name when the containing object literal is part of a destructuring pattern.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1308: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1308",
    number: Some(1308),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'await' expressions are only allowed within async functions and at the top levels of modules.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS7026: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7026",
    number: Some(7026),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "JSX element implicitly has type 'any' because no interface 'JSX.{0}' exists.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS7017: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7017",
    number: Some(7017),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Element implicitly has an 'any' type because type '{0}' has no index signature.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1107: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1107",
    number: Some(1107),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Jump target cannot cross function boundary.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1115: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1115",
    number: Some(1115),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A 'continue' statement can only jump to a label of an enclosing iteration statement.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1116: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1116",
    number: Some(1116),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A 'break' statement can only jump to a label of an enclosing statement.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1105: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1105",
    number: Some(1105),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A 'break' statement can only be used within an enclosing iteration or switch statement.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1104: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1104",
    number: Some(1104),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A 'continue' statement can only be used within an enclosing iteration statement.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1111: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1111",
    number: Some(1111),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Private field '{0}' must be declared in an enclosing class.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1114: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1114",
    number: Some(1114),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Duplicate label '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1344: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1344",
    number: Some(1344),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A label is not allowed here.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1101: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1101",
    number: Some(1101),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'with' statements are not allowed in strict mode.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2410: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2410",
    number: Some(2410),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The 'with' statement is not supported. All symbols in a 'with' block will have type 'any'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1100: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1100",
    number: Some(1100),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Invalid use of '{0}' in strict mode.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1210: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1210",
    number: Some(1210),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Code contained in a class is evaluated in JavaScript's strict mode which does not allow this use of '{0}'. For more information, see https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Strict_mode.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1215: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1215",
    number: Some(1215),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Invalid use of '{0}'. Modules are automatically in strict mode.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1212: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1212",
    number: Some(1212),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Identifier expected. '{0}' is a reserved word in strict mode.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1213: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1213",
    number: Some(1213),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Identifier expected. '{0}' is a reserved word in strict mode. Class definitions are automatically in strict mode.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1214: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1214",
    number: Some(1214),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Identifier expected. '{0}' is a reserved word in strict mode. Modules are automatically in strict mode.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS17012: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS17012",
    number: Some(17012),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' is not a valid meta-property for keyword '{1}'. Did you mean '{2}'?",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS18061: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18061",
    number: Some(18061),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' is not a valid meta-property for keyword 'import'. Did you mean 'meta' or 'defer'?",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2795: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2795",
    number: Some(2795),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The 'intrinsic' keyword can only be used to declare compiler provided intrinsic types.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2838: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2838",
    number: Some(2838),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "All declarations of '{0}' must have identical constraints.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1017: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1017",
    number: Some(1017),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An index signature cannot have a rest parameter.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1018: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1018",
    number: Some(1018),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An index signature parameter cannot have an accessibility modifier.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1020: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1020",
    number: Some(1020),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An index signature parameter cannot have an initializer.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1022: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1022",
    number: Some(1022),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An index signature parameter must have a type annotation.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1491: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1491",
    number: Some(1491),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' modifier cannot appear on a 'using' declaration.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1495: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1495",
    number: Some(1495),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' modifier cannot appear on an 'await using' declaration.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1097: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1097",
    number: Some(1097),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' list cannot be empty.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1171: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1171",
    number: Some(1171),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A comma expression is not allowed in a computed property name.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2500: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2500",
    number: Some(2500),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A class can only implement an identifier/qualified-name with optional type arguments.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS18024: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18024",
    number: Some(18024),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An enum member cannot be named with a private identifier.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1484: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1484",
    number: Some(1484),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' is a type and must be imported using a type-only import when 'verbatimModuleSyntax' is enabled.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1485: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1485",
    number: Some(1485),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' resolves to a type-only declaration and must be imported using a type-only import when 'verbatimModuleSyntax' is enabled.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1205: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1205",
    number: Some(1205),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Re-exporting a type when '{0}' is enabled requires using 'export type'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1280: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1280",
    number: Some(1280),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Namespaces are not allowed in global script files when '{0}' is enabled. If this file is not intended to be a global script, set 'moduleDetection' to 'force' or add an empty 'export {}' statement.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1282: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1282",
    number: Some(1282),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An 'export =' declaration must reference a value when 'verbatimModuleSyntax' is enabled, but '{0}' only refers to a type.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1283: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1283",
    number: Some(1283),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An 'export =' declaration must reference a real value when 'verbatimModuleSyntax' is enabled, but '{0}' resolves to a type-only declaration.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1284: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1284",
    number: Some(1284),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An 'export default' must reference a value when 'verbatimModuleSyntax' is enabled, but '{0}' only refers to a type.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1285: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1285",
    number: Some(1285),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An 'export default' must reference a real value when 'verbatimModuleSyntax' is enabled, but '{0}' resolves to a type-only declaration.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1292: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1292",
    number: Some(1292),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' resolves to a type and must be marked type-only in this file before re-exporting when '{1}' is enabled. Consider using 'export type { {0} as default }'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS1448: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1448",
    number: Some(1448),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' resolves to a type-only declaration and must be re-exported using a type-only re-export when '{1}' is enabled.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS1453: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1453",
    number: Some(1453),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "`resolution-mode` should be either `require` or `import`.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1269: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1269",
    number: Some(1269),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot use 'export import' on a type or type-only namespace when '{0}' is enabled.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2855: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2855",
    number: Some(2855),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Class field '{0}' defined by the parent class is not accessible in the child class via super.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1048: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1048",
    number: Some(1048),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A rest parameter cannot have an initializer.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1186: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1186",
    number: Some(1186),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A rest element cannot have an initializer.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1493: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1493",
    number: Some(1493),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The left-hand side of a 'for...in' statement cannot be a 'using' declaration.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1494: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1494",
    number: Some(1494),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The left-hand side of a 'for...in' statement cannot be an 'await using' declaration.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1355: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1355",
    number: Some(1355),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A 'const' assertion can only be applied to references to enum members, or string, number, boolean, array, or object literals.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS17013: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS17013",
    number: Some(17013),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Meta-property '{0}' is only allowed in the body of a function declaration, function expression, or constructor.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2526: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2526",
    number: Some(2526),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A 'this' type is available only in a non-static member of a class or interface.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1338: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1338",
    number: Some(1338),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'infer' declarations are only permitted in the 'extends' clause of a conditional type.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2427: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2427",
    number: Some(2427),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Interface name cannot be '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2457: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2457",
    number: Some(2457),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type alias name cannot be '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2431: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2431",
    number: Some(2431),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Enum name cannot be '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2368: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2368",
    number: Some(2368),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type parameter name cannot be '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2414: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2414",
    number: Some(2414),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Class name cannot be '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2492: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2492",
    number: Some(2492),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot redeclare identifier '{0}' in catch clause.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2480: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2480",
    number: Some(2480),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'let' is not allowed to be used as a name in 'let' or 'const' declarations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1163: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1163",
    number: Some(1163),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A 'yield' expression is only allowed in a generator body.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2523: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2523",
    number: Some(2523),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'yield' expressions cannot be used in a parameter initializer.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2524: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2524",
    number: Some(2524),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'await' expressions cannot be used in a parameter initializer.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS18037: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18037",
    number: Some(18037),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'await' expression cannot be used inside a class static block.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1103: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1103",
    number: Some(1103),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'for await' loops are only allowed within async functions and at the top levels of modules.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2335: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2335",
    number: Some(2335),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'super' can only be referenced in a derived class.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2337: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2337",
    number: Some(2337),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Super calls are not permitted outside constructors or in nested functions inside constructors.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2660: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2660",
    number: Some(2660),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'super' can only be referenced in members of derived classes or object literal expressions.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2466: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2466",
    number: Some(2466),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'super' cannot be referenced in a computed property name.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2465: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2465",
    number: Some(2465),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'this' cannot be referenced in a computed property name.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2331: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2331",
    number: Some(2331),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'this' cannot be referenced in a module or namespace body.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2332: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2332",
    number: Some(2332),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'this' cannot be referenced in current location.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1036: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1036",
    number: Some(1036),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Statements are not allowed in ambient contexts.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1038: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1038",
    number: Some(1038),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A 'declare' modifier cannot be used in an already ambient context.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1337: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1337",
    number: Some(1337),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An index signature parameter type cannot be a literal type or generic type. Consider using a mapped object type instead.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1268: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1268",
    number: Some(1268),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An index signature parameter type must be 'string', 'number', 'symbol', or a template literal type.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2374: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2374",
    number: Some(2374),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Duplicate index signature for type '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2567: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2567",
    number: Some(2567),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Enum declarations can only merge with namespace or other enum declarations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1182: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1182",
    number: Some(1182),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A destructuring declaration must have an initializer.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1070: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1070",
    number: Some(1070),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' modifier cannot appear on a type member.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1071: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1071",
    number: Some(1071),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' modifier cannot appear on an index signature.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1090: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1090",
    number: Some(1090),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' modifier cannot appear on a parameter.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1273: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1273",
    number: Some(1273),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' modifier cannot appear on a type parameter",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1031: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1031",
    number: Some(1031),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' modifier cannot appear on class elements of this kind.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1248: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1248",
    number: Some(1248),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A class member cannot have the '{0}' keyword.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2628: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2628",
    number: Some(2628),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot assign to '{0}' because it is an enum.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2629: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2629",
    number: Some(2629),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot assign to '{0}' because it is a class.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2630: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2630",
    number: Some(2630),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot assign to '{0}' because it is a function.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2631: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2631",
    number: Some(2631),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot assign to '{0}' because it is a namespace.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1345: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1345",
    number: Some(1345),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An expression of type 'void' cannot be tested for truthiness.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2783: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2783",
    number: Some(2783),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' is specified more than once, so this usage will be overwritten.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2464: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2464",
    number: Some(2464),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A computed property name must be of type 'string', 'number', 'symbol', or 'any'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2359: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2359",
    number: Some(2359),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The right-hand side of an 'instanceof' expression must be either of type 'any', a class, function, or other type assignable to the 'Function' interface type, or an object type with a 'Symbol.hasInstance' method.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2383: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2383",
    number: Some(2383),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Overload signatures must all be exported or non-exported.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2384: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2384",
    number: Some(2384),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Overload signatures must all be ambient or non-ambient.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2385: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2385",
    number: Some(2385),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Overload signatures must all be public, private or protected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2516: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2516",
    number: Some(2516),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "All declarations of an abstract method must be consecutive.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2386: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2386",
    number: Some(2386),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Overload signatures must all be optional or required.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1341: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1341",
    number: Some(1341),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Class constructor may not be an accessor.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2387: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2387",
    number: Some(2387),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Function overload must be static.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2388: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2388",
    number: Some(2388),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Function overload must not be static.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2389: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2389",
    number: Some(2389),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Function implementation name must be '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1245: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1245",
    number: Some(1245),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Method '{0}' cannot have an implementation because it is marked abstract.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2512: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2512",
    number: Some(2512),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Overload signatures must all be abstract or non-abstract.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2308: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2308",
    number: Some(2308),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Module {0} has already exported a member named '{1}'. Consider explicitly re-exporting to resolve the ambiguity.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2309: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2309",
    number: Some(2309),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An export assignment cannot be used in a module with other exported elements.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2395: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2395",
    number: Some(2395),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Individual declarations in merged declaration '{0}' must be all exported or all local.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2428: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2428",
    number: Some(2428),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "All declarations of '{0}' must have identical type parameters.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2370: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2370",
    number: Some(2370),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A rest parameter must be of an array type.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1030: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1030",
    number: Some(1030),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' modifier already seen.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2450: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2450",
    number: Some(2450),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Enum '{0}' used before its declaration.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1118: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1118",
    number: Some(1118),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An object literal cannot have multiple get/set accessors with the same name.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1255: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1255",
    number: Some(1255),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A definite assignment assertion '!' is not permitted in this context.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1156: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1156",
    number: Some(1156),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' declarations can only be declared inside a block.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2842: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2842",
    number: Some(2842),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' is an unused renaming of '{1}'. Did you intend to use it as a type annotation?",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2372: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2372",
    number: Some(2372),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Parameter '{0}' cannot reference itself.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2373: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2373",
    number: Some(2373),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Parameter '{0}' cannot reference identifier '{1}' declared after it.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2804: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2804",
    number: Some(2804),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Duplicate identifier '{0}'. Static and instance elements cannot share the same private name.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2502: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2502",
    number: Some(2502),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' is referenced directly or indirectly in its own type annotation.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2313: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2313",
    number: Some(2313),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type parameter '{0}' has a circular constraint.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1330: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1330",
    number: Some(1330),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A property of an interface or type literal whose type is a 'unique symbol' type must be 'readonly'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1331: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1331",
    number: Some(1331),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A property of a class whose type is a 'unique symbol' type must be both 'static' and 'readonly'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1332: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1332",
    number: Some(1332),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A variable whose type is a 'unique symbol' type must be 'const'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1335: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1335",
    number: Some(1335),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'unique symbol' types are not allowed here.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1206: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1206",
    number: Some(1206),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Decorators are not valid here.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1147: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1147",
    number: Some(1147),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Import declarations in a namespace cannot reference a module.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2880: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2880",
    number: Some(2880),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Import assertions have been replaced by import attributes. Use 'with' instead of 'assert'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2348: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2348",
    number: Some(2348),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Value of type '{0}' is not callable. Did you mean to include 'new'?",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2539: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2539",
    number: Some(2539),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot assign to '{0}' because it is not a variable.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1089: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1089",
    number: Some(1089),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' modifier cannot appear on a constructor declaration.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1202: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1202",
    number: Some(1202),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Import assignment cannot be used when targeting ECMAScript modules. Consider using 'import * as ns from \"mod\"', 'import {a} from \"mod\"', 'import d from \"mod\"', or another module format instead.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1203: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1203",
    number: Some(1203),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Export assignment cannot be used when targeting ECMAScript modules. Consider using 'export default' or another module format instead.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2699: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2699",
    number: Some(2699),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Static property '{0}' conflicts with built-in property 'Function.{0}' of constructor function '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS1274: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1274",
    number: Some(1274),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' modifier can only appear on a type parameter of a class, interface or type alias",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2637: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2637",
    number: Some(2637),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Variance annotations are only supported in type aliases for object, function, constructor, and mapped types.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1333: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1333",
    number: Some(1333),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'unique symbol' types may not be used on a variable declaration with a binding name.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1334: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1334",
    number: Some(1334),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'unique symbol' types are only allowed on variables in a variable statement.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1091: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1091",
    number: Some(1091),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Only a single variable declaration is allowed in a 'for...in' statement.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1188: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1188",
    number: Some(1188),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Only a single variable declaration is allowed in a 'for...of' statement.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1189: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1189",
    number: Some(1189),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The variable declaration of a 'for...in' statement cannot have an initializer.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1190: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1190",
    number: Some(1190),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The variable declaration of a 'for...of' statement cannot have an initializer.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2404: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2404",
    number: Some(2404),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The left-hand side of a 'for...in' statement cannot use a type annotation.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2483: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2483",
    number: Some(2483),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The left-hand side of a 'for...of' statement cannot use a type annotation.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2364: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2364",
    number: Some(2364),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The left-hand side of an assignment expression must be a variable or a property access.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2357: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2357",
    number: Some(2357),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The operand of an increment or decrement operator must be a variable or a property access.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2779: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2779",
    number: Some(2779),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The left-hand side of an assignment expression may not be an optional property access.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2777: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2777",
    number: Some(2777),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The operand of an increment or decrement operator may not be an optional property access.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1014: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1014",
    number: Some(1014),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A rest parameter must be last in a parameter list.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2462: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2462",
    number: Some(2462),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A rest element must be last in a destructuring pattern.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1166: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1166",
    number: Some(1166),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A computed property name in a class property declaration must have a simple literal type or a 'unique symbol' type.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1169: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1169",
    number: Some(1169),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A computed property name in an interface must refer to an expression whose type is a literal type or a 'unique symbol' type.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1170: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1170",
    number: Some(1170),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A computed property name in a type literal must refer to an expression whose type is a literal type or a 'unique symbol' type.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2834: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2834",
    number: Some(2834),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Relative import paths need explicit file extensions in ECMAScript imports when '--moduleResolution' is 'node16' or 'nodenext'. Consider adding an extension to the import path.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2835: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2835",
    number: Some(2835),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Relative import paths need explicit file extensions in ECMAScript imports when '--moduleResolution' is 'node16' or 'nodenext'. Did you mean '{0}'?",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1044: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1044",
    number: Some(1044),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' modifier cannot appear on a module or namespace element.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1040: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1040",
    number: Some(1040),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' modifier cannot be used in an ambient context.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1042: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1042",
    number: Some(1042),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' modifier cannot be used here.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1243: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1243",
    number: Some(1243),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' modifier cannot be used with '{1}' modifier.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS1277: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1277",
    number: Some(1277),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' modifier can only appear on a type parameter of a function, method or class",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1191: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1191",
    number: Some(1191),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An import declaration cannot have modifiers.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1231: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1231",
    number: Some(1231),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An export assignment must be at the top level of a file or module declaration.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1232: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1232",
    number: Some(1232),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An import declaration can only be used at the top level of a namespace or module.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1233: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1233",
    number: Some(1233),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An export declaration can only be used at the top level of a namespace or module.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1234: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1234",
    number: Some(1234),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An ambient module declaration is only allowed at the top level in a file.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1235: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1235",
    number: Some(1235),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A namespace declaration is only allowed at the top level of a namespace or module.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1258: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1258",
    number: Some(1258),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A default export must be at the top level of a file or module declaration.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2439: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2439",
    number: Some(2439),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Import or export declaration in an ambient module declaration cannot reference module through relative module name.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2435: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2435",
    number: Some(2435),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Ambient modules cannot be nested in other modules or namespaces.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1540: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1540",
    number: Some(1540),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A 'namespace' declaration should not be declared using the 'module' keyword. Please use the 'namespace' keyword instead.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2669: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2669",
    number: Some(2669),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Augmentations for the global scope can only be directly nested in external modules or ambient module declarations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2670: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2670",
    number: Some(2670),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Augmentations for the global scope should have 'declare' modifier unless they appear in already ambient context.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2666: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2666",
    number: Some(2666),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Exports and export assignments are not permitted in module augmentations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2667: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2667",
    number: Some(2667),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Imports are not permitted in module augmentations. Consider moving them to the enclosing external module.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2668: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2668",
    number: Some(2668),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'export' modifier cannot be applied to ambient modules and module augmentations since they are always visible.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1046: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1046",
    number: Some(1046),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Top-level declarations in .d.ts files must start with either a 'declare' or 'export' modifier.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1346: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1346",
    number: Some(1346),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This parameter is not allowed with 'use strict' directive.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1347: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1347",
    number: Some(1347),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'use strict' directive cannot be used with non-simple parameter list.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1162: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1162",
    number: Some(1162),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An object member cannot be declared optional.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2848: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2848",
    number: Some(2848),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The right-hand side of an 'instanceof' expression must not be an instantiation expression.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS18016: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18016",
    number: Some(18016),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Private identifiers are not allowed outside class bodies.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS18019: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18019",
    number: Some(18019),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' modifier cannot be used with a private identifier.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1034: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1034",
    number: Some(1034),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'super' must be followed by an argument list or member access.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1275: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1275",
    number: Some(1275),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'accessor' modifier can only appear on a property declaration.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1228: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1228",
    number: Some(1228),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A type predicate is only allowed in return type position for functions and methods.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2815: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2815",
    number: Some(2815),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'arguments' cannot be referenced in property initializers or class static initialization blocks.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2737: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2737",
    number: Some(2737),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "BigInt literals are not available when targeting lower than ES2020.",
    argument_count: 0,
    support: DiagnosticSupport::CatalogOnly,
};

pub const TS1433: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1433",
    number: Some(1433),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Neither decorators nor modifiers may be applied to 'this' parameters.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1196: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1196",
    number: Some(1196),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Catch clause variable type annotation must be 'any' or 'unknown' if specified.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2463: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2463",
    number: Some(2463),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A binding pattern parameter cannot be optional in an implementation signature.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2491: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2491",
    number: Some(2491),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The left-hand side of a 'for...in' statement cannot be a destructuring pattern.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2406: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2406",
    number: Some(2406),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The left-hand side of a 'for...in' statement must be a variable or a property access.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2780: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2780",
    number: Some(2780),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The left-hand side of a 'for...in' statement may not be an optional property access.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2781: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2781",
    number: Some(2781),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The left-hand side of a 'for...of' statement may not be an optional property access.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2778: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2778",
    number: Some(2778),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The target of an object rest assignment may not be an optional property access.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2408: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2408",
    number: Some(2408),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Setters cannot return a value.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2409: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2409",
    number: Some(2409),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Return type of constructor signature must be assignable to the instance type of the class.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS18041: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18041",
    number: Some(18041),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A 'return' statement cannot be used inside a class static block.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS17005: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS17005",
    number: Some(17005),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A constructor cannot contain a 'super' call when its class extends 'null'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1053: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1053",
    number: Some(1053),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A 'set' accessor cannot have rest parameter.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1052: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1052",
    number: Some(1052),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A 'set' accessor parameter cannot have an initializer.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1054: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1054",
    number: Some(1054),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A 'get' accessor cannot have parameters.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1267: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1267",
    number: Some(1267),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Property '{0}' cannot have an initializer because it is marked abstract.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1242: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1242",
    number: Some(1242),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'abstract' modifier can only appear on a class, method, or property declaration.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS18006: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18006",
    number: Some(18006),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Classes may not have a field named 'constructor'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2680: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2680",
    number: Some(2680),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A '{0}' parameter must be the first parameter.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1358: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1358",
    number: Some(1358),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Tagged template expressions are not permitted in an optional chain.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1317: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1317",
    number: Some(1317),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A parameter property cannot be declared using a rest parameter.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2467: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2467",
    number: Some(2467),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A computed property name cannot reference a type parameter from its containing type.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1225: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1225",
    number: Some(1225),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot find parameter '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1230: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1230",
    number: Some(1230),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A type predicate cannot reference element '{0}' in a binding pattern.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2415: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2415",
    number: Some(2415),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Class '{0}' incorrectly extends base class '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2417: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2417",
    number: Some(2417),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Class static side '{0}' incorrectly extends base class static side '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2507: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2507",
    number: Some(2507),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type '{0}' is not a constructor function type.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2863: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2863",
    number: Some(2863),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A class cannot extend a primitive type like '{0}'. Classes can only extend constructable values.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2864: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2864",
    number: Some(2864),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A class cannot implement a primitive type like '{0}'. It can only implement other named object types.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2446: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2446",
    number: Some(2446),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Property '{0}' is protected and only accessible through an instance of class '{1}'. This is an instance of class '{2}'.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS18013: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18013",
    number: Some(18013),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Property '{0}' is not accessible outside class '{1}' because it has a private identifier.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS18014: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18014",
    number: Some(18014),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The property '{0}' cannot be accessed on type '{1}' within this class because it is shadowed by another private identifier with the same spelling.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2673: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2673",
    number: Some(2673),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Constructor of class '{0}' is private and only accessible within the class declaration.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2674: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2674",
    number: Some(2674),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Constructor of class '{0}' is protected and only accessible within the class declaration.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2675: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2675",
    number: Some(2675),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot extend a class '{0}'. Class constructor is marked as private.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS4105: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS4105",
    number: Some(4105),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Private or protected member '{0}' cannot be accessed on a type parameter.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2376: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2376",
    number: Some(2376),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A 'super' call must be the first statement in the constructor to refer to 'super' or 'this' when a derived class contains initialized properties, parameter properties, or private identifiers.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2401: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2401",
    number: Some(2401),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A 'super' call must be a root-level statement within a constructor of a derived class that contains initialized properties, parameter properties, or private identifiers.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2803: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2803",
    number: Some(2803),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot assign to private method '{0}'. Private methods are not writable.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2806: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2806",
    number: Some(2806),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Private accessor was defined without a getter.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2725: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2725",
    number: Some(2725),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Class name cannot be 'Object' when targeting ES5 and above with module {0}.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2397: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2397",
    number: Some(2397),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Declaration name conflicts with built-in global identifier '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1216: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1216",
    number: Some(1216),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Identifier expected. '__esModule' is reserved as an exported marker when transforming ECMAScript modules.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2438: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2438",
    number: Some(2438),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Import name cannot be '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2441: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2441",
    number: Some(2441),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Duplicate identifier '{0}'. Compiler reserves name '{1}' in top level scope of a module.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2818: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2818",
    number: Some(2818),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Duplicate identifier '{0}'. Compiler reserves name '{1}' when emitting 'super' references in static initializers.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2578: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2578",
    number: Some(2578),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unused '@ts-expect-error' directive.",
    argument_count: 0,
    support: DiagnosticSupport::CatalogOnly,
};

pub const TS2664: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2664",
    number: Some(2664),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Invalid module name in augmentation, module '{0}' cannot be found.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2671: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2671",
    number: Some(2671),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot augment module '{0}' because it resolves to a non-module entity.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2436: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2436",
    number: Some(2436),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Ambient module declaration cannot specify relative module name.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2432: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2432",
    number: Some(2432),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "In an enum with multiple declarations, only one declaration can omit an initializer for its first enum element.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2477: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2477",
    number: Some(2477),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'const' enum member initializer was evaluated to a non-finite value.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2478: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2478",
    number: Some(2478),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'const' enum member initializer was evaluated to disallowed value 'NaN'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2476: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2476",
    number: Some(2476),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A const enum member can only be accessed using a string literal.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2475: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2475",
    number: Some(2475),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'const' enums can only be used in property or index access expressions or the right hand side of an import declaration or export assignment or type query.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2748: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2748",
    number: Some(2748),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot access ambient const enums when '{0}' is enabled.",
    argument_count: 1,
    support: DiagnosticSupport::CatalogOnly,
};

pub const TS17004: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS17004",
    number: Some(17004),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot use JSX unless the '--jsx' flag is provided.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS7027: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7027",
    number: Some(7027),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unreachable code detected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2687: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2687",
    number: Some(2687),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "All declarations of '{0}' must have identical modifiers.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2814: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2814",
    number: Some(2814),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Function with bodies can only merge with classes that are ambient.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2813: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2813",
    number: Some(2813),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Class declaration cannot implement overload list for '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2434: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2434",
    number: Some(2434),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A namespace declaration cannot be located prior to a class or function with which it is merged.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2433: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2433",
    number: Some(2433),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A namespace declaration cannot be in a different file from a class or function with which it is merged.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2652: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2652",
    number: Some(2652),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Merged declaration '{0}' cannot include a default export declaration. Consider adding a separate 'export default {0}' declaration instead.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2481: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2481",
    number: Some(2481),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot initialize outer scoped variable '{0}' in the same scope as block scoped declaration '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2744: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2744",
    number: Some(2744),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type parameter defaults can only reference previously declared type parameters.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2700: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2700",
    number: Some(2700),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Rest types may only be created from object types.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2574: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2574",
    number: Some(2574),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A rest element type must be an array type.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2495: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2495",
    number: Some(2495),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type '{0}' is not an array type or a string type.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2505: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2505",
    number: Some(2505),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A generator cannot have a 'void' type annotation.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS6234: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS6234",
    number: Some(6234),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This expression is not callable because it is a 'get' accessor. Did you mean to use it without '()'?",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2690: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2690",
    number: Some(2690),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' only refers to a type, but is being used as a value here. Did you mean to use '{1} in {0}'?",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2560: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2560",
    number: Some(2560),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Value of type '{0}' has no properties in common with type '{1}'. Did you mean to call it?",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS7032: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7032",
    number: Some(7032),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Property '{0}' implicitly has type 'any', because its set accessor lacks a parameter type annotation.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS7041: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7041",
    number: Some(7041),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The containing arrow function captures the global value of 'this'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS7028: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7028",
    number: Some(7028),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unused label.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2677: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2677",
    number: Some(2677),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A type predicate's type must be assignable to its parameter's type.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2405: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2405",
    number: Some(2405),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The left-hand side of a 'for...in' statement must be of type 'string' or 'any'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1011: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1011",
    number: Some(1011),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An element access expression should take an argument.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1221: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1221",
    number: Some(1221),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Generators are not allowed in an ambient context.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1222: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1222",
    number: Some(1222),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An overload signature cannot be declared as a generator.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1368: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1368",
    number: Some(1368),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Class constructor may not be a generator.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS18038: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18038",
    number: Some(18038),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'for await' loops cannot be used inside a class static block.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2487: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2487",
    number: Some(2487),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The left-hand side of a 'for...of' statement must be a variable or a property access.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1005: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1005",
    number: Some(1005),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' expected.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2846: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2846",
    number: Some(2846),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A declaration file cannot be imported without 'import type'. Did you mean to import an implementation file '{0}' instead?",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS6263: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS6263",
    number: Some(6263),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Module '{0}' was resolved to '{1}', but '--allowArbitraryExtensions' is not set.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS1254: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1254",
    number: Some(1254),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A 'const' initializer in an ambient context must be a string or numeric literal or literal enum reference.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS17019: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS17019",
    number: Some(17019),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' at the end of a type is not valid TypeScript syntax. Did you mean to write '{1}'?",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS17020: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS17020",
    number: Some(17020),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' at the start of a type is not valid TypeScript syntax. Did you mean to write '{1}'?",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS1110: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1110",
    number: Some(1110),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS6053: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS6053",
    number: Some(6053),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "File '{0}' not found.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS6054: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS6054",
    number: Some(6054),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "File '{0}' has an unsupported extension. The only supported extensions are {1}.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS6231: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS6231",
    number: Some(6231),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Could not resolve the path '{0}' with the extensions: {1}.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS6504: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS6504",
    number: Some(6504),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "File '{0}' is a JavaScript file. Did you mean to enable the 'allowJs' option?",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1006: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1006",
    number: Some(1006),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A file cannot have a reference to itself.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1109: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1109",
    number: Some(1109),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Expression expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1002: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1002",
    number: Some(1002),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unterminated string literal.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1003: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1003",
    number: Some(1003),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Identifier expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1010: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1010",
    number: Some(1010),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'*/' expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1012: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1012",
    number: Some(1012),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unexpected token.",
    argument_count: 0,
    support: DiagnosticSupport::CatalogOnly,
};

pub const TS1068: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1068",
    number: Some(1068),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unexpected token. A constructor, method, accessor, or property was expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1124: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1124",
    number: Some(1124),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Digit expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1125: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1125",
    number: Some(1125),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Hexadecimal digit expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1126: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1126",
    number: Some(1126),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unexpected end of text.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1127: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1127",
    number: Some(1127),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Invalid character.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1128: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1128",
    number: Some(1128),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Declaration or statement expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1129: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1129",
    number: Some(1129),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Statement expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1130: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1130",
    number: Some(1130),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'case' or 'default' expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1131: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1131",
    number: Some(1131),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Property or signature expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1132: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1132",
    number: Some(1132),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Enum member expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1134: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1134",
    number: Some(1134),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Variable declaration expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1135: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1135",
    number: Some(1135),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Argument expression expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1136: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1136",
    number: Some(1136),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Property assignment expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1137: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1137",
    number: Some(1137),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Expression or comma expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1138: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1138",
    number: Some(1138),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Parameter declaration expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1139: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1139",
    number: Some(1139),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type parameter declaration expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1140: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1140",
    number: Some(1140),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type argument expected.",
    argument_count: 0,
    support: DiagnosticSupport::CatalogOnly,
};

pub const TS1142: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1142",
    number: Some(1142),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Line break not permitted here.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1144: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1144",
    number: Some(1144),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{' or ';' expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1145: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1145",
    number: Some(1145),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{' or JSX element expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1146: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1146",
    number: Some(1146),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Declaration expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1160: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1160",
    number: Some(1160),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unterminated template literal.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1161: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1161",
    number: Some(1161),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unterminated regular expression literal.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1177: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1177",
    number: Some(1177),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Binary digit expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1178: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1178",
    number: Some(1178),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Octal digit expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1179: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1179",
    number: Some(1179),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unexpected token. '{' expected.",
    argument_count: 0,
    support: DiagnosticSupport::CatalogOnly,
};

pub const TS1180: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1180",
    number: Some(1180),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Property destructuring pattern expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1181: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1181",
    number: Some(1181),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Array element destructuring pattern expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1185: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1185",
    number: Some(1185),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Merge conflict marker encountered.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1198: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1198",
    number: Some(1198),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An extended Unicode escape value must be between 0x0 and 0x10FFFF inclusive.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1199: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1199",
    number: Some(1199),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unterminated Unicode escape sequence.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1209: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1209",
    number: Some(1209),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Invalid optional chain from new expression. Did you mean to call '{0}()'?",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1260: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1260",
    number: Some(1260),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Keywords cannot contain escape characters.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1351: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1351",
    number: Some(1351),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An identifier or keyword cannot immediately follow a numeric literal.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1352: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1352",
    number: Some(1352),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A bigint literal cannot use exponential notation.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1353: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1353",
    number: Some(1353),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A bigint literal must be an integer.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1357: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1357",
    number: Some(1357),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An enum member name must be followed by a ',', '=', or '}'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1359: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1359",
    number: Some(1359),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Identifier expected. '{0}' is a reserved word that cannot be used here.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1381: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1381",
    number: Some(1381),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unexpected token. Did you mean `{'}'}` or `&rbrace;`?",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1382: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1382",
    number: Some(1382),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unexpected token. Did you mean `{'>'}` or `&gt;`?",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1385: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1385",
    number: Some(1385),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Function type notation must be parenthesized when used in a union type.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1386: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1386",
    number: Some(1386),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Constructor type notation must be parenthesized when used in a union type.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1387: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1387",
    number: Some(1387),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Function type notation must be parenthesized when used in an intersection type.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1388: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1388",
    number: Some(1388),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Constructor type notation must be parenthesized when used in an intersection type.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1389: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1389",
    number: Some(1389),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' is not allowed as a variable declaration name.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1390: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1390",
    number: Some(1390),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' is not allowed as a parameter name.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1434: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1434",
    number: Some(1434),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unexpected keyword or identifier.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1435: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1435",
    number: Some(1435),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unknown keyword or identifier. Did you mean '{0}'?",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1436: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1436",
    number: Some(1436),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Decorators must precede the name and all keywords of property declarations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1437: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1437",
    number: Some(1437),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Namespace must be given a name.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1438: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1438",
    number: Some(1438),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Interface must be given a name.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1439: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1439",
    number: Some(1439),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type alias must be given a name.",
    argument_count: 0,
    support: DiagnosticSupport::CatalogOnly,
};

pub const TS1440: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1440",
    number: Some(1440),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Variable declaration not allowed at this location.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1441: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1441",
    number: Some(1441),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot start a function call in a type annotation.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1442: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1442",
    number: Some(1442),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Expected '=' for property initializer.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1443: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1443",
    number: Some(1443),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Module declaration names may only use ' or \" quoted strings.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1472: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1472",
    number: Some(1472),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'catch' or 'finally' expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1473: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1473",
    number: Some(1473),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An import declaration can only be used at the top level of a module.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1474: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1474",
    number: Some(1474),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An export declaration can only be used at the top level of a module.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1478: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1478",
    number: Some(1478),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Identifier or string literal expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1487: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1487",
    number: Some(1487),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Octal escape sequences are not allowed. Use the syntax '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1488: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1488",
    number: Some(1488),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Escape sequence '{0}' is not allowed.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2656: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2656",
    number: Some(2656),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Non-abstract class expression is missing implementations for the following members of '{0}': {1}.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2657: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2657",
    number: Some(2657),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "JSX expressions must have one parent element.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2809: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2809",
    number: Some(2809),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Declaration or statement expected. This '=' follows a block of statements, so if you intended to write a destructuring assignment, you might need to wrap the whole assignment in parentheses.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2819: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2819",
    number: Some(2819),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Namespace name cannot be '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS6188: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS6188",
    number: Some(6188),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Numeric separators are not allowed here.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS6189: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS6189",
    number: Some(6189),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Multiple consecutive numeric separators are not permitted.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS8003: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS8003",
    number: Some(8003),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'export =' can only be used in TypeScript files.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS8004: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS8004",
    number: Some(8004),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type parameter declarations can only be used in TypeScript files.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS8006: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS8006",
    number: Some(8006),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' declarations can only be used in TypeScript files.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS8008: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS8008",
    number: Some(8008),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type aliases can only be used in TypeScript files.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS8009: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS8009",
    number: Some(8009),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The '{0}' modifier can only be used in TypeScript files.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS8010: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS8010",
    number: Some(8010),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type annotations can only be used in TypeScript files.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS8011: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS8011",
    number: Some(8011),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type arguments can only be used in TypeScript files.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS8013: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS8013",
    number: Some(8013),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Non-null assertions can only be used in TypeScript files.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS8017: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS8017",
    number: Some(8017),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Signature declarations can only be used in TypeScript files.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS8038: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS8038",
    number: Some(8038),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Decorators may not appear after 'export' or 'export default' if they also appear before 'export'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS17002: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS17002",
    number: Some(17002),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Expected corresponding JSX closing tag for '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS17006: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS17006",
    number: Some(17006),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An unary expression with the '{0}' operator is not allowed in the left-hand side of an exponentiation expression. Consider enclosing the expression in parentheses.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS17007: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS17007",
    number: Some(17007),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A type assertion expression is not allowed in the left-hand side of an exponentiation expression. Consider enclosing the expression in parentheses.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS17008: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS17008",
    number: Some(17008),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "JSX element '{0}' has no corresponding closing tag.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS17014: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS17014",
    number: Some(17014),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "JSX fragment has no corresponding closing tag.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS17015: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS17015",
    number: Some(17015),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Expected corresponding closing tag for JSX fragment.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS17021: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS17021",
    number: Some(17021),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unicode escape sequence cannot appear here.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS18007: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18007",
    number: Some(18007),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "JSX expressions may not use the comma operator. Did you mean to write an array?",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS18009: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18009",
    number: Some(18009),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Private identifiers cannot be used as parameters.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS18026: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18026",
    number: Some(18026),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'#!' can only be used at the start of a file.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS18029: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18029",
    number: Some(18029),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Private identifiers are not allowed in variable declarations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS18030: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18030",
    number: Some(18030),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An optional chain cannot contain private identifiers.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1262: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1262",
    number: Some(1262),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Identifier expected. '{0}' is a reserved word at the top-level of a module.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1314: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1314",
    number: Some(1314),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Global module exports may only appear in module files.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1315: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1315",
    number: Some(1315),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Global module exports may only appear in declaration files.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1316: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1316",
    number: Some(1316),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Global module exports may only appear at top level.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS5061: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS5061",
    number: Some(5061),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Pattern '{0}' can have at most one '*' character.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS18012: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18012",
    number: Some(18012),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'#constructor' is a reserved word.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1499: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1499",
    number: Some(1499),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unknown regular expression flag.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1500: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1500",
    number: Some(1500),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Duplicate regular expression flag.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1501: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1501",
    number: Some(1501),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This regular expression flag is only available when targeting '{0}' or later.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1503: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1503",
    number: Some(1503),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Named capturing groups are only available when targeting 'ES2018' or later.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1504: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1504",
    number: Some(1504),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Subpattern flags must be present when there is a minus sign.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1505: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1505",
    number: Some(1505),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Incomplete quantifier. Digit expected.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1506: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1506",
    number: Some(1506),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Numbers out of order in quantifier.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1507: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1507",
    number: Some(1507),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "There is nothing available for repetition.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1508: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1508",
    number: Some(1508),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unexpected '{0}'. Did you mean to escape it with backslash?",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1509: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1509",
    number: Some(1509),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This regular expression flag cannot be toggled within a subpattern.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1510: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1510",
    number: Some(1510),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'\\k' must be followed by a capturing group name enclosed in angle brackets.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1511: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1511",
    number: Some(1511),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'\\q' is only available inside character class.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1512: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1512",
    number: Some(1512),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'\\c' must be followed by an ASCII letter.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1513: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1513",
    number: Some(1513),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Undetermined character escape.",
    argument_count: 0,
    support: DiagnosticSupport::CatalogOnly,
};

pub const TS1514: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1514",
    number: Some(1514),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Expected a capturing group name.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1515: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1515",
    number: Some(1515),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Named capturing groups with the same name must be mutually exclusive to each other.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1516: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1516",
    number: Some(1516),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A character class range must not be bounded by another character class.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1517: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1517",
    number: Some(1517),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Range out of order in character class.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1518: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1518",
    number: Some(1518),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Anything that would possibly match more than a single character is invalid inside a negated character class.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1519: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1519",
    number: Some(1519),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Operators must not be mixed within a character class. Wrap it in a nested class instead.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1520: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1520",
    number: Some(1520),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Expected a class set operand.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1521: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1521",
    number: Some(1521),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'\\q' must be followed by string alternatives enclosed in braces.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1522: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1522",
    number: Some(1522),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A character class must not contain a reserved double punctuator. Did you mean to escape it with backslash?",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1523: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1523",
    number: Some(1523),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Expected a Unicode property name.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1524: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1524",
    number: Some(1524),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unknown Unicode property name.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1525: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1525",
    number: Some(1525),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Expected a Unicode property value.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1526: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1526",
    number: Some(1526),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unknown Unicode property value.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1527: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1527",
    number: Some(1527),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Expected a Unicode property name or value.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1528: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1528",
    number: Some(1528),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Any Unicode property that would possibly match more than a single character is only available when the Unicode Sets (v) flag is set.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1529: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1529",
    number: Some(1529),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unknown Unicode property name or value.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1530: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1530",
    number: Some(1530),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unicode property value expressions are only available when the Unicode (u) flag or the Unicode Sets (v) flag is set.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1531: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1531",
    number: Some(1531),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'\\{0}' must be followed by a Unicode property value expression enclosed in braces.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1532: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1532",
    number: Some(1532),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "There is no capturing group named '{0}' in this regular expression.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1533: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1533",
    number: Some(1533),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This backreference refers to a group that does not exist. There are only {0} capturing groups in this regular expression.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1534: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1534",
    number: Some(1534),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This backreference refers to a group that does not exist. There are no capturing groups in this regular expression.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1535: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1535",
    number: Some(1535),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This character cannot be escaped in a regular expression.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1536: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1536",
    number: Some(1536),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Octal escape sequences and backreferences are not allowed in a character class. If this was intended as an escape sequence, use the syntax '{0}' instead.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1537: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1537",
    number: Some(1537),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Decimal escape sequences and backreferences are not allowed in a character class.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1538: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1538",
    number: Some(1538),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unicode escape sequences are only available when the Unicode (u) flag or the Unicode Sets (v) flag is set.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1497: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1497",
    number: Some(1497),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Expression must be enclosed in parentheses to be used as a decorator.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1270: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1270",
    number: Some(1270),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Decorator function return type '{0}' is not assignable to type '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS1271: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1271",
    number: Some(1271),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Decorator function return type is '{0}' but is expected to be 'void' or 'any'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1329: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1329",
    number: Some(1329),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' accepts too few arguments to be used as a decorator here. Did you mean to call it first and write '@{0}()'?",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1238: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1238",
    number: Some(1238),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unable to resolve signature of class decorator when called as an expression.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1239: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1239",
    number: Some(1239),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unable to resolve signature of parameter decorator when called as an expression.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1240: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1240",
    number: Some(1240),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unable to resolve signature of property decorator when called as an expression.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1241: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1241",
    number: Some(1241),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unable to resolve signature of method decorator when called as an expression.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1200: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1200",
    number: Some(1200),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Line terminator not permitted before arrow.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1294: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1294",
    number: Some(1294),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This syntax is not allowed when 'erasableSyntaxOnly' is enabled.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS6807: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS6807",
    number: Some(6807),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This operation can be simplified. This shift is identical to `{0} {1} {2}`.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2823: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2823",
    number: Some(2823),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Import attributes are only supported when the '--module' option is set to 'esnext', 'node18', 'node20', 'nodenext', or 'preserve'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1323: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1323",
    number: Some(1323),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Dynamic imports are only supported when the '--module' flag is set to 'es2020', 'es2022', 'esnext', 'commonjs', 'amd', 'system', 'umd', 'node16', 'node18', 'node20', or 'nodenext'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2513: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2513",
    number: Some(2513),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Abstract method '{0}' in class '{1}' cannot be accessed via super expression.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS6138: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS6138",
    number: Some(6138),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Property '{0}' is declared but its value is never read.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS6205: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS6205",
    number: Some(6205),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "All type parameters are unused.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2506: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2506",
    number: Some(2506),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' is referenced directly or indirectly in its own base expression.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2310: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2310",
    number: Some(2310),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type '{0}' recursively references itself as a base type.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1123: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1123",
    number: Some(1123),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Variable declaration list cannot be empty.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1009: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1009",
    number: Some(1009),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Trailing comma not allowed.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2320: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2320",
    number: Some(2320),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Interface '{0}' cannot simultaneously extend types '{1}' and '{2}'.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS9007: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS9007",
    number: Some(9007),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Function must have an explicit return type annotation with --isolatedDeclarations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS9008: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS9008",
    number: Some(9008),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Method must have an explicit return type annotation with --isolatedDeclarations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS9009: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS9009",
    number: Some(9009),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "At least one accessor must have an explicit type annotation with --isolatedDeclarations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS9010: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS9010",
    number: Some(9010),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Variable must have an explicit type annotation with --isolatedDeclarations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS9011: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS9011",
    number: Some(9011),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Parameter must have an explicit type annotation with --isolatedDeclarations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS9012: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS9012",
    number: Some(9012),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Property must have an explicit type annotation with --isolatedDeclarations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS9013: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS9013",
    number: Some(9013),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Expression type can't be inferred with --isolatedDeclarations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS9015: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS9015",
    number: Some(9015),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Objects that contain spread assignments can't be inferred with --isolatedDeclarations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS9016: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS9016",
    number: Some(9016),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Objects that contain shorthand properties can't be inferred with --isolatedDeclarations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS9017: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS9017",
    number: Some(9017),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Only const arrays can be inferred with --isolatedDeclarations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS9018: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS9018",
    number: Some(9018),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Arrays with spread elements can't inferred with --isolatedDeclarations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS9019: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS9019",
    number: Some(9019),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Binding elements can't be exported directly with --isolatedDeclarations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS9020: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS9020",
    number: Some(9020),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Enum member initializers must be computable without references to external symbols with --isolatedDeclarations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS9021: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS9021",
    number: Some(9021),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Extends clause can't contain an expression with --isolatedDeclarations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS9022: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS9022",
    number: Some(9022),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Inference from class expressions is not supported with --isolatedDeclarations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS9023: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS9023",
    number: Some(9023),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Assigning properties to functions without declaring them is not supported with --isolatedDeclarations. Add an explicit declaration for the properties assigned to this function.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS9025: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS9025",
    number: Some(9025),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Declaration emit for this parameter requires implicitly adding undefined to its type. This is not supported with --isolatedDeclarations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS9037: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS9037",
    number: Some(9037),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Default exports can't be inferred with --isolatedDeclarations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS9038: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS9038",
    number: Some(9038),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Computed property names on class or object literals cannot be inferred with --isolatedDeclarations.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2743: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2743",
    number: Some(2743),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "No overload expects {0} type arguments, but overloads do exist that expect either {1} or {2} type arguments.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS2874: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2874",
    number: Some(2874),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This JSX tag requires '{0}' to be in scope, but it could not be found.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2879: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2879",
    number: Some(2879),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Using JSX fragments requires fragment factory '{0}' to be in scope, but it could not be found.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1471: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1471",
    number: Some(1471),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Module '{0}' cannot be imported using this construct. The specifier only resolves to an ES module, which cannot be imported with 'require'. Use an ECMAScript import instead.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1479: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1479",
    number: Some(1479),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The current file is a CommonJS module whose imports will produce 'require' calls; however, the referenced file is an ECMAScript module and cannot be imported with 'require'. Consider writing a dynamic 'import(\"{0}\")' call instead.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1541: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1541",
    number: Some(1541),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type-only import of an ECMAScript module from a CommonJS module must have a 'resolution-mode' attribute.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2720: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2720",
    number: Some(2720),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Class '{0}' incorrectly implements class '{1}'. Did you mean to extend '{1}' and inherit its members as a subclass?",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS1309: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1309",
    number: Some(1309),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The current file is a CommonJS module and cannot use 'await' at the top level.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1470: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1470",
    number: Some(1470),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The 'import.meta' meta-property is not allowed in files which will build into CommonJS output.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS8026: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS8026",
    number: Some(8026),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Expected {0} type arguments; provide these with an '@extends' tag.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS8027: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS8027",
    number: Some(8027),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Expected {0}-{1} type arguments; provide these with an '@extends' tag.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS1340: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1340",
    number: Some(1340),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Module '{0}' does not refer to a type, but is used as a type here. Did you mean 'typeof import('{0}')'?",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS8023: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS8023",
    number: Some(8023),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "JSDoc '@{0} {1}' does not match the 'extends {2}' clause.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS8024: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS8024",
    number: Some(8024),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "JSDoc '@param' tag has name '{0}', but there is no parameter with that name.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS8030: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS8030",
    number: Some(8030),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A JSDoc '@type' tag on a function must have a signature with the correct number of arguments.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS8032: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS8032",
    number: Some(8032),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Qualified name '{0}' is not allowed without a leading '@param {object} {1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS8033: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS8033",
    number: Some(8033),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A JSDoc '@typedef' comment may not contain multiple '@type' tags.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS8039: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS8039",
    number: Some(8039),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A JSDoc '@template' tag may not follow a '@typedef', '@callback', or '@overload' tag",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1069: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1069",
    number: Some(1069),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Unexpected token. A type parameter name was expected without curly braces.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2559: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2559",
    number: Some(2559),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type '{0}' has no properties in common with type '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS1286: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1286",
    number: Some(1286),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "ECMAScript imports and exports cannot be written in a CommonJS file under 'verbatimModuleSyntax'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1287: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1287",
    number: Some(1287),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A top-level 'export' modifier cannot be used on value declarations in a CommonJS module when 'verbatimModuleSyntax' is enabled.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1295: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1295",
    number: Some(1295),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "ECMAScript imports and exports cannot be written in a CommonJS file under 'verbatimModuleSyntax'. Adjust the 'type' field in the nearest 'package.json' to make this file an ECMAScript module, or adjust your 'verbatimModuleSyntax', 'module', and 'moduleResolution' settings in TypeScript.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1324: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1324",
    number: Some(1324),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Dynamic imports only support a second argument when the '--module' option is set to 'esnext', 'node16', 'node18', 'node20', 'nodenext', or 'preserve'.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1325: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1325",
    number: Some(1325),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Argument of dynamic import cannot be spread element.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1326: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1326",
    number: Some(1326),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This use of 'import' is invalid. 'import()' calls can be written, but they must have parentheses and cannot have type arguments.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2712: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2712",
    number: Some(2712),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A dynamic import call in ES5 requires the 'Promise' constructor.  Make sure you have a declaration for the 'Promise' constructor or include 'ES2015' in your '--lib' option.",
    argument_count: 0,
    support: DiagnosticSupport::CatalogOnly,
};

pub const TS2858: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2858",
    number: Some(2858),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Import attribute values must be string literal expressions.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS7036: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS7036",
    number: Some(7036),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Dynamic import's specifier must be of type 'string', but here has type '{0}'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2604: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2604",
    number: Some(2604),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "JSX element type '{0}' does not have any construct or call signatures.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2607: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2607",
    number: Some(2607),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "JSX element class does not support attributes because it does not have a '{0}' property.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2609: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2609",
    number: Some(2609),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "JSX spread child must be an array type.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2786: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2786",
    number: Some(2786),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' cannot be used as a JSX component.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS17016: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS17016",
    number: Some(17016),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The 'jsxFragmentFactory' compiler option must be provided to use JSX fragments with the 'jsxFactory' compiler option.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS17017: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS17017",
    number: Some(17017),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "An @jsxFrag pragma is required when using an @jsx pragma with JSX fragments.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2639: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2639",
    number: Some(2639),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "React components cannot include JSX namespace names",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS17001: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS17001",
    number: Some(17001),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "JSX elements cannot have multiple attributes with the same name.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2816: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2816",
    number: Some(2816),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Cannot use 'this' in a static property initializer of a decorated class.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2498: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2498",
    number: Some(2498),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Module '{0}' uses 'export =' and cannot be used with 'export *'.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2375: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2375",
    number: Some(2375),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type '{0}' is not assignable to type '{1}' with 'exactOptionalPropertyTypes: true'. Consider adding 'undefined' to the types of the target's properties.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2379: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2379",
    number: Some(2379),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Argument of type '{0}' is not assignable to parameter of type '{1}' with 'exactOptionalPropertyTypes: true'. Consider adding 'undefined' to the types of the target's properties.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2412: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2412",
    number: Some(2412),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type '{0}' is not assignable to type '{1}' with 'exactOptionalPropertyTypes: true'. Consider adding 'undefined' to the type of the target.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2635: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2635",
    number: Some(2635),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type '{0}' has no signatures for which the type argument list is applicable.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS18033: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18033",
    number: Some(18033),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Type '{0}' is not assignable to type '{1}' as required for computed enum member values.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2354: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2354",
    number: Some(2354),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This syntax requires an imported helper but module '{0}' cannot be found.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS2343: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2343",
    number: Some(2343),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This syntax requires an imported helper named '{1}' which does not exist in '{0}'. Consider upgrading your version of '{0}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS2807: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2807",
    number: Some(2807),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "This syntax requires an imported helper named '{1}' with {2} parameters, which is not compatible with the one in '{0}'. Consider upgrading your version of '{0}'.",
    argument_count: 3,
    support: DiagnosticSupport::Emitted,
};

pub const TS1492: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1492",
    number: Some(1492),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'{0}' declarations may not have binding patterns.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS1547: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1547",
    number: Some(1547),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'using' declarations are not allowed in 'case' or 'default' clauses unless contained within a block.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1548: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1548",
    number: Some(1548),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'await using' declarations are not allowed in 'case' or 'default' clauses unless contained within a block.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS18054: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18054",
    number: Some(18054),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'await using' statements cannot be used inside a class static block.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2852: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2852",
    number: Some(2852),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'await using' statements are only allowed within async functions and at the top levels of modules.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1375: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1375",
    number: Some(1375),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'await' expressions are only allowed at the top level of a file when that file is a module, but this file has no imports or exports. Consider adding an empty 'export {}' to make this file a module.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1378: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1378",
    number: Some(1378),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Top-level 'await' expressions are only allowed when the 'module' option is set to 'es2022', 'esnext', 'system', 'node16', 'node18', 'node20', 'nodenext', or 'preserve', and the 'target' option is set to 'es2017' or higher.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1431: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1431",
    number: Some(1431),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'for await' loops are only allowed at the top level of a file when that file is a module, but this file has no imports or exports. Consider adding an empty 'export {}' to make this file a module.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1432: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1432",
    number: Some(1432),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Top-level 'for await' loops are only allowed when the 'module' option is set to 'es2022', 'esnext', 'system', 'node16', 'node18', 'node20', 'nodenext', or 'preserve', and the 'target' option is set to 'es2017' or higher.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2853: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2853",
    number: Some(2853),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "'await using' statements are only allowed at the top level of a file when that file is a module, but this file has no imports or exports. Consider adding an empty 'export {}' to make this file a module.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2854: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2854",
    number: Some(2854),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Top-level 'await using' statements are only allowed when the 'module' option is set to 'es2022', 'esnext', 'system', 'node16', 'node18', 'node20', 'nodenext', or 'preserve', and the 'target' option is set to 'es2017' or higher.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS1013: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS1013",
    number: Some(1013),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "A rest parameter or binding pattern may not have a trailing comma.",
    argument_count: 0,
    support: DiagnosticSupport::Emitted,
};

pub const TS2684: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS2684",
    number: Some(2684),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "The 'this' context of type '{0}' is not assignable to method's 'this' of type '{1}'.",
    argument_count: 2,
    support: DiagnosticSupport::Emitted,
};

pub const TS5059: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS5059",
    number: Some(5059),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Invalid value for '--reactNamespace'. '{0}' is not a valid identifier.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS5067: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS5067",
    number: Some(5067),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Invalid value for 'jsxFactory'. '{0}' is not a valid identifier or qualified-name.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const TS18035: DiagnosticDescriptor = DiagnosticDescriptor {
    code: "TS18035",
    number: Some(18035),
    source: DiagnosticSource::TypeScript,
    category: DiagnosticCategory::Error,
    message_template: "Invalid value for 'jsxFragmentFactory'. '{0}' is not a valid identifier or qualified-name.",
    argument_count: 1,
    support: DiagnosticSupport::Emitted,
};

pub const DIAGNOSTIC_CATALOG: &[DiagnosticDescriptor] = &[
    TS1029,
    TS1281,
    TS2411,
    TS2413,
    TS5112,
    TS5102,
    TS5097,
    TS5108,
    TS6046,
    TS1360,
    TS1379,
    TS1380,
    TS1361,
    TS1362,
    TS1063,
    TS1319,
    TS2302,
    TS2304,
    TS2300,
    TS2303,
    TS2706,
    TS2717,
    TS2305,
    TS2610,
    TS2611,
    TS2323,
    TS2484,
    TS2341,
    TS2445,
    TS2440,
    TS2613,
    TS2595,
    TS2597,
    TS2616,
    TS2701,
    TS1288,
    TS2850,
    TS2851,
    TS2614,
    TS2306,
    TS2307,
    TS2732,
    TS2882,
    TS2314,
    TS2707,
    TS2315,
    TS2322,
    TS2418,
    TS2808,
    TS2820,
    TS18043,
    TS18046,
    TS2532,
    TS2531,
    TS2533,
    TS18047,
    TS18048,
    TS18049,
    TS2571,
    TS18050,
    TS2339,
    TS2344,
    TS2345,
    TS2347,
    TS2349,
    TS2351,
    TS2352,
    TS2353,
    TS2355,
    TS2356,
    TS2362,
    TS2363,
    TS2365,
    TS2366,
    TS2367,
    TS2393,
    TS2394,
    TS2448,
    TS2451,
    TS2714,
    TS2454,
    TS2493,
    TS2540,
    TS2542,
    TS2514,
    TS7015,
    TS2862,
    TS4094,
    TS4104,
    TS2536,
    TS2537,
    TS2538,
    TS2550,
    TS2551,
    TS2812,
    TS2552,
    TS2554,
    TS2555,
    TS2556,
    TS2561,
    TS2576,
    TS2588,
    TS2580,
    TS2591,
    TS2688,
    TS2693,
    TS2686,
    TS2741,
    TS2745,
    TS2746,
    TS2747,
    TS2875,
    TS2754,
    TS2749,
    TS2869,
    TS2447,
    TS2577,
    TS2857,
    TS7012,
    TS2456,
    TS2459,
    TS2469,
    TS2632,
    TS2731,
    TS2736,
    TS2769,
    TS2774,
    TS2839,
    TS2845,
    TS2871,
    TS2872,
    TS2873,
    TS7005,
    TS7006,
    TS7016,
    TS7018,
    TS7019,
    TS7022,
    TS4111,
    TS1106,
    TS1121,
    TS1489,
    TS6133,
    TS6142,
    TS6192,
    TS6198,
    TS6199,
    TS6196,
    TS4112,
    TS4113,
    TS4114,
    TS4115,
    TS4116,
    TS4117,
    TS4119,
    TS4121,
    TS4122,
    TS4123,
    TS4127,
    TS4128,
    TS7023,
    TS7029,
    TS7030,
    TS2534,
    TS7031,
    TS7034,
    TS7051,
    TS7052,
    TS7053,
    TS7054,
    TS7055,
    TS7025,
    TS7056,
    TS7057,
    TS7058,
    TS7059,
    TS7060,
    TS7061,
    TS1117,
    TS1155,
    TS2378,
    TS2390,
    TS2391,
    TS2528,
    TS2739,
    TS2740,
    TS7008,
    TS7013,
    TS7020,
    TS7010,
    TS2377,
    TS2392,
    TS2695,
    TS1015,
    TS1016,
    TS1039,
    TS1049,
    TS1095,
    TS1119,
    TS1192,
    TS1244,
    TS1253,
    TS2369,
    TS2371,
    TS17009,
    TS17011,
    TS18004,
    TS2676,
    TS2678,
    TS2515,
    TS2653,
    TS2654,
    TS2655,
    TS2511,
    TS2420,
    SURGE_PARSER_ERROR,
    SURGE_DUPLICATE_TYPE_PARAMETER,
    SURGE_UNSUPPORTED_MODULE_SYNTAX,
    SURGE_UNSUPPORTED_DECLARATION,
    SURGE_TYPE_ALIAS_CYCLE,
    SURGE_TYPE_DECLARATION_CYCLE,
    TS2703,
    TS2704,
    TS2791,
    TS2790,
    TS18011,
    TS1102,
    TS2358,
    TS2474,
    TS2488,
    TS2504,
    TS2763,
    TS2764,
    TS2765,
    TS2766,
    TS2698,
    TS2416,
    TS2423,
    TS2425,
    TS2426,
    TS2312,
    TS2636,
    TS2590,
    TS2430,
    TS2558,
    TS2564,
    TS2565,
    TS2403,
    TS2449,
    TS2650,
    TS2651,
    TS2721,
    TS2722,
    TS2723,
    TS7009,
    TS2350,
    TS2407,
    TS2724,
    TS2729,
    TS2683,
    TS1019,
    TS1021,
    TS1024,
    TS1028,
    TS1047,
    TS1051,
    TS1061,
    TS1064,
    TS1066,
    TS1092,
    TS1093,
    TS1096,
    TS1098,
    TS1099,
    TS1108,
    TS1141,
    TS1164,
    TS1172,
    TS1173,
    TS1174,
    TS1175,
    TS1176,
    TS1183,
    TS1184,
    TS1187,
    TS1249,
    TS1257,
    TS1263,
    TS1264,
    TS1265,
    TS1266,
    TS1276,
    TS1318,
    TS1354,
    TS1363,
    TS1477,
    TS1490,
    TS1502,
    TS1545,
    TS1546,
    TS2206,
    TS2207,
    TS2452,
    TS2499,
    TS2566,
    TS2681,
    TS2730,
    TS2784,
    TS5085,
    TS5086,
    TS5087,
    TS8002,
    TS8005,
    TS8012,
    TS8016,
    TS8037,
    TS17000,
    TS18010,
    TS18055,
    TS18058,
    TS18059,
    TS2311,
    TS2503,
    TS2581,
    TS2582,
    TS2583,
    TS2584,
    TS2585,
    TS2592,
    TS2593,
    TS2661,
    TS2662,
    TS2663,
    TS2301,
    TS2844,
    TS2708,
    TS2713,
    TS2865,
    TS2867,
    TS2868,
    TS2709,
    TS2702,
    TS2833,
    TS1194,
    TS2694,
    TS2689,
    TS1312,
    TS1308,
    TS7026,
    TS7017,
    TS1107,
    TS1115,
    TS1116,
    TS1105,
    TS1104,
    TS1111,
    TS1114,
    TS1344,
    TS1101,
    TS2410,
    TS1100,
    TS1210,
    TS1215,
    TS1212,
    TS1213,
    TS1214,
    TS17012,
    TS18061,
    TS2795,
    TS2838,
    TS1017,
    TS1018,
    TS1020,
    TS1022,
    TS1491,
    TS1495,
    TS1097,
    TS1171,
    TS2500,
    TS18024,
    TS1484,
    TS1485,
    TS1205,
    TS1280,
    TS1282,
    TS1283,
    TS1284,
    TS1285,
    TS1292,
    TS1448,
    TS1453,
    TS1269,
    TS2855,
    TS1048,
    TS1186,
    TS1493,
    TS1494,
    TS1355,
    TS17013,
    TS2526,
    TS1338,
    TS2427,
    TS2457,
    TS2431,
    TS2368,
    TS2414,
    TS2492,
    TS2480,
    TS1163,
    TS2523,
    TS2524,
    TS18037,
    TS1103,
    TS2335,
    TS2337,
    TS2660,
    TS2466,
    TS2465,
    TS2331,
    TS2332,
    TS1036,
    TS1038,
    TS1337,
    TS1268,
    TS2374,
    TS2567,
    TS1182,
    TS1070,
    TS1071,
    TS1090,
    TS1273,
    TS1031,
    TS1248,
    TS2628,
    TS2629,
    TS2630,
    TS2631,
    TS1345,
    TS2783,
    TS2464,
    TS2359,
    TS2383,
    TS2384,
    TS2385,
    TS2516,
    TS2386,
    TS1341,
    TS2387,
    TS2388,
    TS2389,
    TS1245,
    TS2512,
    TS2308,
    TS2309,
    TS2395,
    TS2428,
    TS2370,
    TS1030,
    TS2450,
    TS1118,
    TS1255,
    TS1156,
    TS2842,
    TS2372,
    TS2373,
    TS2804,
    TS2502,
    TS2313,
    TS1330,
    TS1331,
    TS1332,
    TS1335,
    TS1206,
    TS1147,
    TS2880,
    TS2348,
    TS2539,
    TS1089,
    TS1202,
    TS1203,
    TS2699,
    TS1274,
    TS2637,
    TS1333,
    TS1334,
    TS1091,
    TS1188,
    TS1189,
    TS1190,
    TS2404,
    TS2483,
    TS2364,
    TS2357,
    TS2779,
    TS2777,
    TS1014,
    TS2462,
    TS1166,
    TS1169,
    TS1170,
    TS2834,
    TS2835,
    TS1044,
    TS1040,
    TS1042,
    TS1243,
    TS1277,
    TS1191,
    TS1231,
    TS1232,
    TS1233,
    TS1234,
    TS1235,
    TS1258,
    TS2439,
    TS2435,
    TS1540,
    TS2669,
    TS2670,
    TS2666,
    TS2667,
    TS2668,
    TS1046,
    TS1346,
    TS1347,
    TS1162,
    TS2848,
    TS18016,
    TS18019,
    TS1034,
    TS1275,
    TS1228,
    TS2815,
    TS2737,
    TS1433,
    TS1196,
    TS2463,
    TS2491,
    TS2406,
    TS2780,
    TS2781,
    TS2778,
    TS2408,
    TS2409,
    TS18041,
    TS17005,
    TS1053,
    TS1052,
    TS1054,
    TS1267,
    TS1242,
    TS18006,
    TS2680,
    TS1358,
    TS1317,
    TS2467,
    TS1225,
    TS1230,
    TS2415,
    TS2417,
    TS2507,
    TS2863,
    TS2864,
    TS2446,
    TS18013,
    TS18014,
    TS2673,
    TS2674,
    TS2675,
    TS4105,
    TS2376,
    TS2401,
    TS2803,
    TS2806,
    TS2725,
    TS2397,
    TS1216,
    TS2438,
    TS2441,
    TS2818,
    TS2578,
    TS2664,
    TS2671,
    TS2436,
    TS2432,
    TS2477,
    TS2478,
    TS2476,
    TS2475,
    TS2748,
    TS17004,
    TS7027,
    TS2687,
    TS2814,
    TS2813,
    TS2434,
    TS2433,
    TS2652,
    TS2481,
    TS2744,
    TS2700,
    TS2574,
    TS2495,
    TS2505,
    TS6234,
    TS2690,
    TS2560,
    TS7032,
    TS7041,
    TS7028,
    TS2677,
    TS2405,
    TS1011,
    TS1221,
    TS1222,
    TS1368,
    TS18038,
    TS2487,
    TS1005,
    TS2846,
    TS6263,
    TS1254,
    TS17019,
    TS17020,
    TS1110,
    TS6053,
    TS6054,
    TS6231,
    TS6504,
    TS1006,
    TS1109,
    TS1002,
    TS1003,
    TS1010,
    TS1012,
    TS1068,
    TS1124,
    TS1125,
    TS1126,
    TS1127,
    TS1128,
    TS1129,
    TS1130,
    TS1131,
    TS1132,
    TS1134,
    TS1135,
    TS1136,
    TS1137,
    TS1138,
    TS1139,
    TS1140,
    TS1142,
    TS1144,
    TS1145,
    TS1146,
    TS1160,
    TS1161,
    TS1177,
    TS1178,
    TS1179,
    TS1180,
    TS1181,
    TS1185,
    TS1198,
    TS1199,
    TS1209,
    TS1260,
    TS1351,
    TS1352,
    TS1353,
    TS1357,
    TS1359,
    TS1381,
    TS1382,
    TS1385,
    TS1386,
    TS1387,
    TS1388,
    TS1389,
    TS1390,
    TS1434,
    TS1435,
    TS1436,
    TS1437,
    TS1438,
    TS1439,
    TS1440,
    TS1441,
    TS1442,
    TS1443,
    TS1472,
    TS1473,
    TS1474,
    TS1478,
    TS1487,
    TS1488,
    TS2656,
    TS2657,
    TS2809,
    TS2819,
    TS6188,
    TS6189,
    TS8003,
    TS8004,
    TS8006,
    TS8008,
    TS8009,
    TS8010,
    TS8011,
    TS8013,
    TS8017,
    TS8038,
    TS17002,
    TS17006,
    TS17007,
    TS17008,
    TS17014,
    TS17015,
    TS17021,
    TS18007,
    TS18009,
    TS18026,
    TS18029,
    TS18030,
    TS1262,
    TS1314,
    TS1315,
    TS1316,
    TS5061,
    TS18012,
    TS1499,
    TS1500,
    TS1501,
    TS1503,
    TS1504,
    TS1505,
    TS1506,
    TS1507,
    TS1508,
    TS1509,
    TS1510,
    TS1511,
    TS1512,
    TS1513,
    TS1514,
    TS1515,
    TS1516,
    TS1517,
    TS1518,
    TS1519,
    TS1520,
    TS1521,
    TS1522,
    TS1523,
    TS1524,
    TS1525,
    TS1526,
    TS1527,
    TS1528,
    TS1529,
    TS1530,
    TS1531,
    TS1532,
    TS1533,
    TS1534,
    TS1535,
    TS1536,
    TS1537,
    TS1538,
    TS1497,
    TS1270,
    TS1271,
    TS1329,
    TS1238,
    TS1239,
    TS1240,
    TS1241,
    TS1200,
    TS1294,
    TS6807,
    TS2823,
    TS1323,
    TS2513,
    TS6138,
    TS6205,
    TS2506,
    TS2310,
    TS1123,
    TS1009,
    TS2320,
    TS9007,
    TS9008,
    TS9009,
    TS9010,
    TS9011,
    TS9012,
    TS9013,
    TS9015,
    TS9016,
    TS9017,
    TS9018,
    TS9019,
    TS9020,
    TS9021,
    TS9022,
    TS9023,
    TS9025,
    TS9037,
    TS9038,
    TS2743,
    TS2874,
    TS2879,
    TS1471,
    TS1479,
    TS1541,
    TS2720,
    TS1309,
    TS1470,
    TS8026,
    TS8027,
    TS1340,
    TS8023,
    TS8024,
    TS8030,
    TS8032,
    TS8033,
    TS8039,
    TS1069,
    TS2559,
    TS1286,
    TS1287,
    TS1295,
    TS1324,
    TS1325,
    TS1326,
    TS2712,
    TS2858,
    TS7036,
    TS2604,
    TS2607,
    TS2609,
    TS2786,
    TS17016,
    TS17017,
    TS2639,
    TS17001,
    TS2816,
    TS2498,
    TS2375,
    TS2379,
    TS2412,
    TS2635,
    TS18033,
    TS2354,
    TS2343,
    TS2807,
    TS1492,
    TS1547,
    TS1548,
    TS18054,
    TS2852,
    TS1375,
    TS1378,
    TS1431,
    TS1432,
    TS2853,
    TS2854,
    TS1013,
    TS2684,
    TS5059,
    TS5067,
    TS18035,
];

impl Diagnostic {
    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1029(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1029,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1281(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS1281,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2411(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        arg3: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2411,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
                DiagnosticArg::from(arg3.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2413(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        arg3: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2413,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
                DiagnosticArg::from(arg3.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts5112(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS5112, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts5102(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS5102,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts5097(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS5097,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts5108(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS5108,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts6046(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS6046,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1360(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1360,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1379(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1379, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1380(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1380, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1361(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1361,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1362(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1362,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1063(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1063, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1319(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1319, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2302(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2302, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2304(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2304,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2300(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2300,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2303(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2303,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2706(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2706, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2717(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2717,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2305(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2305,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2610(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2610,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2611(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2611,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2323(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2323,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2484(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2484,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2341(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2341,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2445(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2445,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2440(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2440,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2613(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2613,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2595(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2595,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2597(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2597,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2616(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2616,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2701(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2701, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1288(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1288, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2850(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2850, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2851(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2851, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2614(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2614,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2306(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2306,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2307(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2307,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2732(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2732,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2882(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2882,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2314(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2314,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2707(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2707,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2315(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2315,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2322(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2322,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2418(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2418,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2808(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2808, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2820(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2820,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18043(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS18043, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18046(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS18046,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2532(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2532, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2531(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2531, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2533(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2533, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18047(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS18047,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18048(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS18048,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18049(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS18049,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2571(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2571, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18050(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS18050,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2339(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2339,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2344(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2344,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2345(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2345,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2347(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2347, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2349(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2349, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2351(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2351, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2352(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2352,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2353(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2353,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2355(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2355, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2356(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2356, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2362(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2362, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2363(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2363, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2365(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2365,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2366(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2366, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2367(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2367,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2393(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2393, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2394(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2394, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2448(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2448,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2451(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2451,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2714(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2714, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2454(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2454,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2493(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2493,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2540(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2540,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2542(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2542,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2514(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2514, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7015(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS7015, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2862(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2862,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts4094(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS4094,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts4104(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS4104,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2536(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2536,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2537(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2537,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2538(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2538,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2550(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2550,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2551(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2551,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2812(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2812,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2552(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2552,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2554(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2554,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2555(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2555,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2556(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2556, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2561(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2561,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2576(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2576,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2588(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2588,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2580(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2580,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2591(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2591,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2688(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2688,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2693(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2693,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2686(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2686,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2741(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2741,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2745(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2745,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2746(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2746,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2747(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2747,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2875(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2875,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2754(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2754, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2749(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2749,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2869(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2869, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2447(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2447,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2577(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2577, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2857(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2857, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7012(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS7012,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2456(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2456,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2459(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2459,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2469(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2469,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2632(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2632,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2731(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2731, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2736(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2736,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2769(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2769, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2774(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2774, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2839(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2839,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2845(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2845,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2871(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2871, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2872(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2872, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2873(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2873, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7005(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS7005,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7006(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS7006,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7016(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS7016,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7018(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS7018,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7019(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS7019,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7022(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS7022,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts4111(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS4111,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1106(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1106, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1121(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1121,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1489(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1489, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts6133(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS6133,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts6142(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS6142,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts6192(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS6192, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts6198(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS6198, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts6199(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS6199, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts6196(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS6196,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts4112(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS4112,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts4113(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS4113,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts4114(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS4114,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts4115(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS4115,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts4116(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS4116,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts4117(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS4117,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts4119(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS4119,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts4121(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS4121,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts4122(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS4122,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts4123(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS4123,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts4127(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS4127, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts4128(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS4128, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7023(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS7023,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7029(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS7029, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7030(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS7030, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2534(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2534, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7031(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS7031,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7034(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS7034,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7051(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS7051,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7052(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS7052,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7053(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS7053,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7054(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS7054,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7055(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS7055,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7025(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS7025,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7056(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS7056, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7057(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS7057, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7058(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS7058,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7059(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS7059, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7060(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS7060, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7061(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS7061, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1117(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1117, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1155(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1155,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2378(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2378, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2390(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2390, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2391(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2391, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2528(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2528, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2739(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2739,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2740(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        arg3: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2740,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
                DiagnosticArg::from(arg3.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7008(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS7008,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7013(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS7013, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7020(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS7020, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7010(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS7010,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2377(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2377, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2392(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2392, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2695(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2695, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1015(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1015, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1016(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1016, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1039(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1039, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1049(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1049, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1095(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1095, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1119(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1119, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1192(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1192,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1244(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1244, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1253(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1253, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2369(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2369, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2371(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2371, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts17009(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS17009, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts17011(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS17011, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18004(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS18004,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2676(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2676, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2678(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2678,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2515(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2515,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2653(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2653,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2654(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2654,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2655(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        arg3: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2655,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
                DiagnosticArg::from(arg3.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2511(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2511, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2420(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2420,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn surge_parser_error(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &SURGE_PARSER_ERROR,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn surge_duplicate_type_parameter(
        arg0: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &SURGE_DUPLICATE_TYPE_PARAMETER,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn surge_unsupported_module_syntax(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &SURGE_UNSUPPORTED_MODULE_SYNTAX,
            Vec::<DiagnosticArg>::new(),
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn surge_unsupported_declaration(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &SURGE_UNSUPPORTED_DECLARATION,
            Vec::<DiagnosticArg>::new(),
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn surge_type_alias_cycle(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &SURGE_TYPE_ALIAS_CYCLE,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn surge_type_declaration_cycle(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &SURGE_TYPE_DECLARATION_CYCLE,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2703(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2703, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2704(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2704, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2791(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2791, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2790(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2790, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18011(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS18011, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1102(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1102, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2358(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2358, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2474(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2474, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2488(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2488,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2504(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2504,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2763(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2763,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2764(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2764,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2765(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2765,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2766(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2766,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2698(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2698, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2416(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2416,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2423(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2423,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2425(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2425,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2426(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2426,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2312(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2312, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2636(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2636,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2590(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2590, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2430(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2430,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2558(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2558,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2564(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2564,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2565(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2565,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2403(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2403,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2449(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2449,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2650(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2650,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2651(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2651, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2721(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2721, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2722(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2722, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2723(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2723, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7009(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS7009, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2350(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2350, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2407(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2407,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2724(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2724,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2729(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2729,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2683(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2683, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1019(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1019, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1021(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1021, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1024(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1024, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1028(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1028, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1047(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1047, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1051(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1051, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1061(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1061, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1064(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1064,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1066(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1066, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1092(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1092, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1093(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1093, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1096(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1096, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1098(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1098, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1099(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1099, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1108(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1108, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1141(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1141, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1164(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1164, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1172(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1172, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1173(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1173, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1174(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1174, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1175(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1175, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1176(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1176, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1183(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1183, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1184(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1184, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1187(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1187, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1249(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1249, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1257(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1257, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1263(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1263, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1264(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1264, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1265(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1265, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1266(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1266, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1276(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1276, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1318(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1318, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1354(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1354, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1363(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1363, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1477(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1477, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1490(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1490, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1502(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1502, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1545(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1545, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1546(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1546, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2206(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2206, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2207(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2207, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2452(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2452, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2499(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2499, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2566(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2566, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2681(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2681, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2730(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2730, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2784(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2784, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts5085(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS5085, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts5086(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS5086, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts5087(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS5087, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts8002(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS8002, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts8005(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS8005, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts8012(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS8012, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts8016(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS8016, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts8037(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS8037, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts17000(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS17000, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18010(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS18010, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18055(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS18055,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18058(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS18058, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18059(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS18059, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2311(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2311,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2503(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2503,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2581(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2581,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2582(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2582,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2583(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2583,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2584(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2584,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2585(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2585,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2592(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2592,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2593(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2593,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2661(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2661,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2662(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2662,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2663(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2663,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2301(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2301,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2844(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2844,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2708(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2708,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2713(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2713,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2865(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2865,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2867(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2867,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2868(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2868,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2709(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2709,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2702(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2702,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2833(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2833,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1194(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1194, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2694(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2694,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2689(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2689,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1312(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1312, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1308(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1308, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7026(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS7026,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7017(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS7017,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1107(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1107, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1115(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1115, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1116(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1116, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1105(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1105, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1104(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1104, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1111(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1111,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1114(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1114,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1344(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1344, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1101(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1101, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2410(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2410, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1100(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1100,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1210(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1210,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1215(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1215,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1212(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1212,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1213(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1213,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1214(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1214,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts17012(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS17012,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18061(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS18061,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2795(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2795, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2838(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2838,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1017(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1017, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1018(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1018, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1020(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1020, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1022(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1022, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1491(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1491,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1495(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1495,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1097(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1097,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1171(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1171, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2500(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2500, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18024(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS18024, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1484(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1484,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1485(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1485,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1205(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1205,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1280(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1280,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1282(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1282,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1283(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1283,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1284(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1284,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1285(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1285,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1292(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1292,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1448(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1448,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1453(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1453, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1269(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1269,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2855(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2855,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1048(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1048, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1186(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1186, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1493(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1493, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1494(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1494, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1355(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1355, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts17013(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS17013,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2526(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2526, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1338(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1338, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2427(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2427,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2457(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2457,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2431(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2431,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2368(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2368,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2414(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2414,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2492(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2492,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2480(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2480, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1163(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1163, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2523(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2523, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2524(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2524, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18037(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS18037, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1103(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1103, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2335(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2335, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2337(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2337, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2660(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2660, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2466(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2466, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2465(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2465, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2331(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2331, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2332(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2332, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1036(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1036, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1038(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1038, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1337(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1337, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1268(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1268, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2374(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2374,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2567(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2567, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1182(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1182, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1070(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1070,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1071(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1071,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1090(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1090,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1273(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1273,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1031(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1031,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1248(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1248,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2628(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2628,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2629(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2629,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2630(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2630,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2631(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2631,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1345(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1345, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2783(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2783,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2464(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2464, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2359(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2359, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2383(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2383, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2384(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2384, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2385(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2385, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2516(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2516, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2386(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2386, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1341(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1341, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2387(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2387, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2388(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2388, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2389(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2389,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1245(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1245,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2512(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2512, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2308(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2308,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2309(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2309, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2395(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2395,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2428(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2428,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2370(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2370, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1030(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1030,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2450(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2450,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1118(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1118, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1255(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1255, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1156(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1156,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2842(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2842,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2372(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2372,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2373(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2373,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2804(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2804,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2502(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2502,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2313(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2313,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1330(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1330, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1331(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1331, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1332(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1332, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1335(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1335, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1206(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1206, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1147(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1147, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2880(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2880, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2348(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2348,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2539(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2539,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1089(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1089,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1202(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1202, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1203(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1203, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2699(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2699,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1274(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1274,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2637(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2637, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1333(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1333, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1334(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1334, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1091(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1091, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1188(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1188, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1189(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1189, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1190(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1190, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2404(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2404, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2483(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2483, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2364(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2364, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2357(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2357, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2779(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2779, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2777(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2777, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1014(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1014, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2462(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2462, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1166(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1166, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1169(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1169, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1170(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1170, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2834(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2834, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2835(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2835,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1044(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1044,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1040(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1040,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1042(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1042,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1243(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1243,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1277(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1277,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1191(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1191, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1231(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1231, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1232(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1232, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1233(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1233, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1234(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1234, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1235(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1235, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1258(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1258, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2439(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2439, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2435(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2435, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1540(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1540, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2669(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2669, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2670(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2670, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2666(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2666, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2667(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2667, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2668(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2668, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1046(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1046, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1346(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1346, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1347(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1347, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1162(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1162, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2848(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2848, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18016(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS18016, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18019(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS18019,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1034(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1034, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1275(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1275, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1228(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1228, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2815(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2815, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2737(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2737, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1433(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1433, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1196(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1196, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2463(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2463, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2491(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2491, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2406(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2406, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2780(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2780, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2781(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2781, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2778(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2778, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2408(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2408, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2409(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2409, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18041(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS18041, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts17005(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS17005, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1053(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1053, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1052(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1052, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1054(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1054, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1267(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1267,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1242(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1242, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18006(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS18006, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2680(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2680,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1358(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1358, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1317(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1317, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2467(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2467, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1225(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1225,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1230(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1230,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2415(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2415,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2417(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2417,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2507(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2507,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2863(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2863,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2864(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2864,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2446(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2446,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18013(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS18013,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18014(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS18014,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2673(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2673,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2674(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2674,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2675(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2675,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts4105(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS4105,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2376(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2376, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2401(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2401, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2803(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2803,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2806(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2806, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2725(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2725,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2397(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2397,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1216(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1216, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2438(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2438,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2441(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2441,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2818(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2818,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2578(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2578, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2664(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2664,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2671(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2671,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2436(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2436, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2432(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2432, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2477(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2477, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2478(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2478, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2476(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2476, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2475(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2475, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2748(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2748,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts17004(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS17004, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7027(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS7027, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2687(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2687,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2814(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2814, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2813(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2813,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2434(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2434, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2433(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2433, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2652(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2652,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2481(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2481,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2744(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2744, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2700(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2700, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2574(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2574, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2495(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2495,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2505(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2505, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts6234(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS6234, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2690(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2690,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2560(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2560,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7032(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS7032,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7041(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS7041, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7028(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS7028, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2677(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2677, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2405(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2405, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1011(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1011, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1221(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1221, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1222(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1222, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1368(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1368, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18038(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS18038, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2487(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2487, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1005(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1005,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2846(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2846,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts6263(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS6263,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1254(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1254, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts17019(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS17019,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts17020(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS17020,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1110(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1110, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts6053(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS6053,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts6054(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS6054,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts6231(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS6231,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts6504(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS6504,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1006(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1006, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1109(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1109, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1002(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1002, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1003(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1003, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1010(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1010, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1012(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1012, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1068(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1068, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1124(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1124, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1125(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1125, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1126(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1126, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1127(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1127, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1128(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1128, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1129(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1129, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1130(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1130, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1131(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1131, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1132(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1132, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1134(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1134, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1135(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1135, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1136(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1136, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1137(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1137, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1138(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1138, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1139(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1139, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1140(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1140, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1142(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1142, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1144(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1144, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1145(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1145, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1146(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1146, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1160(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1160, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1161(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1161, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1177(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1177, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1178(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1178, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1179(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1179, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1180(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1180, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1181(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1181, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1185(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1185, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1198(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1198, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1199(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1199, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1209(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1209,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1260(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1260, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1351(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1351, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1352(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1352, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1353(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1353, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1357(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1357, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1359(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1359,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1381(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1381, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1382(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1382, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1385(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1385, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1386(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1386, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1387(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1387, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1388(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1388, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1389(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1389,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1390(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1390,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1434(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1434, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1435(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1435,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1436(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1436, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1437(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1437, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1438(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1438, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1439(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1439, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1440(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1440, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1441(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1441, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1442(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1442, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1443(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1443, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1472(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1472, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1473(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1473, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1474(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1474, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1478(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1478, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1487(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1487,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1488(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1488,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2656(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2656,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2657(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2657, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2809(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2809, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2819(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2819,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts6188(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS6188, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts6189(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS6189, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts8003(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS8003, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts8004(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS8004, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts8006(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS8006,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts8008(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS8008, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts8009(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS8009,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts8010(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS8010, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts8011(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS8011, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts8013(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS8013, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts8017(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS8017, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts8038(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS8038, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts17002(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS17002,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts17006(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS17006,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts17007(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS17007, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts17008(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS17008,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts17014(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS17014, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts17015(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS17015, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts17021(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS17021, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18007(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS18007, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18009(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS18009, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18026(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS18026, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18029(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS18029, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18030(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS18030, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1262(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1262,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1314(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1314, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1315(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1315, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1316(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1316, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts5061(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS5061,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18012(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS18012, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1499(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1499, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1500(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1500, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1501(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1501,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1503(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1503, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1504(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1504, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1505(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1505, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1506(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1506, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1507(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1507, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1508(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1508,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1509(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1509, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1510(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1510, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1511(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1511, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1512(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1512, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1513(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1513, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1514(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1514, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1515(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1515, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1516(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1516, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1517(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1517, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1518(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1518, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1519(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1519, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1520(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1520, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1521(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1521, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1522(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1522, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1523(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1523, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1524(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1524, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1525(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1525, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1526(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1526, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1527(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1527, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1528(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1528, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1529(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1529, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1530(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1530, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1531(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1531,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1532(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1532,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1533(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1533,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1534(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1534, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1535(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1535, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1536(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1536,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1537(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1537, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1538(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1538, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1497(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1497, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1270(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1270,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1271(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1271,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1329(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1329,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1238(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1238, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1239(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1239, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1240(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1240, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1241(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1241, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1200(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1200, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1294(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1294, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts6807(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS6807,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2823(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2823, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1323(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1323, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2513(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2513,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts6138(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS6138,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts6205(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS6205, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2506(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2506,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2310(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2310,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1123(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1123, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1009(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1009, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2320(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2320,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts9007(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS9007, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts9008(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS9008, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts9009(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS9009, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts9010(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS9010, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts9011(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS9011, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts9012(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS9012, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts9013(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS9013, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts9015(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS9015, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts9016(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS9016, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts9017(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS9017, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts9018(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS9018, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts9019(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS9019, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts9020(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS9020, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts9021(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS9021, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts9022(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS9022, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts9023(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS9023, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts9025(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS9025, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts9037(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS9037, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts9038(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS9038, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2743(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2743,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2874(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2874,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2879(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2879,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1471(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1471,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1479(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1479,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1541(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1541, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2720(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2720,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1309(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1309, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1470(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1470, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts8026(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS8026,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts8027(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS8027,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1340(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1340,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts8023(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS8023,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts8024(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS8024,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts8030(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS8030, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts8032(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS8032,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts8033(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS8033, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts8039(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS8039, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1069(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1069, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2559(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2559,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1286(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1286, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1287(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1287, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1295(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1295, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1324(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1324, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1325(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1325, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1326(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1326, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2712(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2712, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2858(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2858, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts7036(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS7036,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2604(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2604,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2607(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2607,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2609(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2609, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2786(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2786,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts17016(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS17016, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts17017(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS17017, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2639(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2639, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts17001(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS17001, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2816(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2816, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2498(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2498,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2375(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2375,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2379(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2379,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2412(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2412,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2635(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2635,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18033(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS18033,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2354(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2354,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2343(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2343,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2807(
        arg0: impl ToString,
        arg1: impl ToString,
        arg2: impl ToString,
        file_name: impl Into<String>,
    ) -> Self {
        Self::from_descriptor(
            &TS2807,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
                DiagnosticArg::from(arg2.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1492(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS1492,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1547(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1547, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1548(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1548, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18054(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS18054, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2852(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2852, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1375(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1375, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1378(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1378, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1431(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1431, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1432(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1432, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2853(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2853, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2854(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS2854, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts1013(file_name: impl Into<String>) -> Self {
        Self::from_descriptor(&TS1013, Vec::<DiagnosticArg>::new(), file_name)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts2684(arg0: impl ToString, arg1: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS2684,
            vec![
                DiagnosticArg::from(arg0.to_string()),
                DiagnosticArg::from(arg1.to_string()),
            ],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts5059(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS5059,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts5067(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS5067,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn ts18035(arg0: impl ToString, file_name: impl Into<String>) -> Self {
        Self::from_descriptor(
            &TS18035,
            vec![DiagnosticArg::from(arg0.to_string())],
            file_name,
        )
    }
}
