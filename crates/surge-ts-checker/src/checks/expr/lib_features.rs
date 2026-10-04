//! tsc's lib feature map: which lib first declares a global or one of its
//! members, for the "change your target library" diagnostics.

use surge_ts_types::Type;

/// tsc's `getFeatureMap` (checker/utilities.go): for each global interface,
/// the lib that first declares each of its members, in declaration order.
pub(crate) const FEATURE_MAP: &[(&str, &[(&str, &[&str])])] = &[
    (
        "Array",
        &[
            ("es2015", &["find", "findIndex", "fill", "copyWithin", "entries", "keys", "values"]),
            ("es2016", &["includes"]),
            ("es2019", &["flat", "flatMap"]),
            ("es2022", &["at"]),
            ("es2023", &["findLastIndex", "findLast", "toReversed", "toSorted", "toSpliced", "with"]),
        ],
    ),
    (
        "Iterator",
        &[
            ("es2015", &[]),
        ],
    ),
    (
        "AsyncIterator",
        &[
            ("es2015", &[]),
        ],
    ),
    (
        "ArrayBuffer",
        &[
            ("es2024", &["maxByteLength", "resizable", "resize", "detached", "transfer", "transferToFixedLength"]),
        ],
    ),
    (
        "Atomics",
        &[
            ("es2017", &["add", "and", "compareExchange", "exchange", "isLockFree", "load", "or", "store", "sub", "wait", "notify", "xor"]),
            ("es2024", &["waitAsync"]),
        ],
    ),
    (
        "SharedArrayBuffer",
        &[
            ("es2017", &["byteLength", "slice"]),
            ("es2024", &["growable", "maxByteLength", "grow"]),
        ],
    ),
    (
        "AsyncIterable",
        &[
            ("es2018", &[]),
        ],
    ),
    (
        "AsyncIterableIterator",
        &[
            ("es2018", &[]),
        ],
    ),
    (
        "AsyncGenerator",
        &[
            ("es2018", &[]),
        ],
    ),
    (
        "AsyncGeneratorFunction",
        &[
            ("es2018", &[]),
        ],
    ),
    (
        "RegExp",
        &[
            ("es2015", &["flags", "sticky", "unicode"]),
            ("es2018", &["dotAll"]),
            ("es2024", &["unicodeSets"]),
        ],
    ),
    (
        "RegExpConstructor",
        &[
            ("es2025", &["escape"]),
        ],
    ),
    (
        "Reflect",
        &[
            ("es2015", &["apply", "construct", "defineProperty", "deleteProperty", "get", "getOwnPropertyDescriptor", "getPrototypeOf", "has", "isExtensible", "ownKeys", "preventExtensions", "set", "setPrototypeOf"]),
        ],
    ),
    (
        "ArrayConstructor",
        &[
            ("es2015", &["from", "of"]),
            ("esnext", &["fromAsync"]),
        ],
    ),
    (
        "ObjectConstructor",
        &[
            ("es2015", &["assign", "getOwnPropertySymbols", "keys", "is", "setPrototypeOf"]),
            ("es2017", &["values", "entries", "getOwnPropertyDescriptors"]),
            ("es2019", &["fromEntries"]),
            ("es2022", &["hasOwn"]),
            ("es2024", &["groupBy"]),
        ],
    ),
    (
        "NumberConstructor",
        &[
            ("es2015", &["isFinite", "isInteger", "isNaN", "isSafeInteger", "parseFloat", "parseInt"]),
        ],
    ),
    (
        "Math",
        &[
            ("es2015", &["clz32", "imul", "sign", "log10", "log2", "log1p", "expm1", "cosh", "sinh", "tanh", "acosh", "asinh", "atanh", "hypot", "trunc", "fround", "cbrt"]),
            ("es2025", &["f16round"]),
        ],
    ),
    (
        "Map",
        &[
            ("es2015", &["entries", "keys", "values"]),
            ("esnext", &["getOrInsert", "getOrInsertComputed"]),
        ],
    ),
    (
        "MapConstructor",
        &[
            ("es2024", &["groupBy"]),
        ],
    ),
    (
        "Set",
        &[
            ("es2015", &["entries", "keys", "values"]),
            ("es2025", &["union", "intersection", "difference", "symmetricDifference", "isSubsetOf", "isSupersetOf", "isDisjointFrom"]),
        ],
    ),
    (
        "PromiseConstructor",
        &[
            ("es2015", &["all", "race", "reject", "resolve"]),
            ("es2020", &["allSettled"]),
            ("es2021", &["any"]),
            ("es2024", &["withResolvers"]),
            ("es2025", &["try"]),
        ],
    ),
    (
        "Symbol",
        &[
            ("es2015", &["for", "keyFor"]),
            ("es2019", &["description"]),
        ],
    ),
    (
        "WeakMap",
        &[
            ("es2015", &[]),
            ("esnext", &["getOrInsert", "getOrInsertComputed"]),
        ],
    ),
    (
        "WeakSet",
        &[
            ("es2015", &[]),
        ],
    ),
    (
        "String",
        &[
            ("es2015", &["codePointAt", "includes", "endsWith", "normalize", "repeat", "startsWith", "anchor", "big", "blink", "bold", "fixed", "fontcolor", "fontsize", "italics", "link", "small", "strike", "sub", "sup"]),
            ("es2017", &["padStart", "padEnd"]),
            ("es2019", &["trimStart", "trimEnd", "trimLeft", "trimRight"]),
            ("es2020", &["matchAll"]),
            ("es2021", &["replaceAll"]),
            ("es2022", &["at"]),
            ("es2024", &["isWellFormed", "toWellFormed"]),
        ],
    ),
    (
        "StringConstructor",
        &[
            ("es2015", &["fromCodePoint", "raw"]),
        ],
    ),
    (
        "DateTimeFormat",
        &[
            ("es2017", &["formatToParts"]),
        ],
    ),
    (
        "Promise",
        &[
            ("es2015", &[]),
            ("es2018", &["finally"]),
        ],
    ),
    (
        "RegExpMatchArray",
        &[
            ("es2018", &["groups"]),
        ],
    ),
    (
        "RegExpExecArray",
        &[
            ("es2018", &["groups"]),
        ],
    ),
    (
        "Intl",
        &[
            ("es2018", &["PluralRules"]),
            ("es2020", &["RelativeTimeFormat", "Locale", "DisplayNames"]),
            ("es2021", &["ListFormat", "DateTimeFormat"]),
            ("es2022", &["Segmenter"]),
            ("es2025", &["DurationFormat"]),
        ],
    ),
    (
        "NumberFormat",
        &[
            ("es2018", &["formatToParts"]),
        ],
    ),
    (
        "SymbolConstructor",
        &[
            ("es2020", &["matchAll"]),
            ("esnext", &["metadata", "dispose", "asyncDispose"]),
        ],
    ),
    (
        "DataView",
        &[
            ("es2020", &["setBigInt64", "setBigUint64", "getBigInt64", "getBigUint64"]),
            ("es2025", &["setFloat16", "getFloat16"]),
        ],
    ),
    (
        "BigInt",
        &[
            ("es2020", &[]),
        ],
    ),
    (
        "RelativeTimeFormat",
        &[
            ("es2020", &["format", "formatToParts", "resolvedOptions"]),
        ],
    ),
    (
        "Int8Array",
        &[
            ("es2022", &["at"]),
            ("es2023", &["findLastIndex", "findLast", "toReversed", "toSorted", "toSpliced", "with"]),
        ],
    ),
    (
        "Uint8Array",
        &[
            ("es2022", &["at"]),
            ("es2023", &["findLastIndex", "findLast", "toReversed", "toSorted", "toSpliced", "with"]),
        ],
    ),
    (
        "Uint8ClampedArray",
        &[
            ("es2022", &["at"]),
            ("es2023", &["findLastIndex", "findLast", "toReversed", "toSorted", "toSpliced", "with"]),
        ],
    ),
    (
        "Int16Array",
        &[
            ("es2022", &["at"]),
            ("es2023", &["findLastIndex", "findLast", "toReversed", "toSorted", "toSpliced", "with"]),
        ],
    ),
    (
        "Uint16Array",
        &[
            ("es2022", &["at"]),
            ("es2023", &["findLastIndex", "findLast", "toReversed", "toSorted", "toSpliced", "with"]),
        ],
    ),
    (
        "Int32Array",
        &[
            ("es2022", &["at"]),
            ("es2023", &["findLastIndex", "findLast", "toReversed", "toSorted", "toSpliced", "with"]),
        ],
    ),
    (
        "Uint32Array",
        &[
            ("es2022", &["at"]),
            ("es2023", &["findLastIndex", "findLast", "toReversed", "toSorted", "toSpliced", "with"]),
        ],
    ),
    (
        "Float16Array",
        &[
            ("es2025", &[]),
        ],
    ),
    (
        "Float32Array",
        &[
            ("es2022", &["at"]),
            ("es2023", &["findLastIndex", "findLast", "toReversed", "toSorted", "toSpliced", "with"]),
        ],
    ),
    (
        "Float64Array",
        &[
            ("es2022", &["at"]),
            ("es2023", &["findLastIndex", "findLast", "toReversed", "toSorted", "toSpliced", "with"]),
        ],
    ),
    (
        "BigInt64Array",
        &[
            ("es2020", &[]),
            ("es2022", &["at"]),
            ("es2023", &["findLastIndex", "findLast", "toReversed", "toSorted", "toSpliced", "with"]),
        ],
    ),
    (
        "BigUint64Array",
        &[
            ("es2020", &[]),
            ("es2022", &["at"]),
            ("es2023", &["findLastIndex", "findLast", "toReversed", "toSorted", "toSpliced", "with"]),
        ],
    ),
    (
        "Error",
        &[
            ("es2022", &["cause"]),
        ],
    ),
    (
        "ErrorConstructor",
        &[
            ("esnext", &["isError"]),
        ],
    ),
    (
        "Uint8ArrayConstructor",
        &[
            ("esnext", &["fromBase64", "fromHex"]),
        ],
    ),
    (
        "DisposableStack",
        &[
            ("esnext", &[]),
        ],
    ),
    (
        "AsyncDisposableStack",
        &[
            ("esnext", &[]),
        ],
    ),
    (
        "Date",
        &[
            ("esnext", &["toTemporalInstant"]),
        ],
    ),
];

/// tsc's `getSuggestedLibForNonExistentName`: the first lib of the name's entry.
pub(crate) fn suggested_lib_for_nonexistent_name(name: &str) -> Option<&'static str> {
    FEATURE_MAP
        .iter()
        .find(|(interface, _)| *interface == name)
        .and_then(|(_, libs)| libs.first())
        .map(|(lib, _)| *lib)
}

/// tsc's `getSuggestedLibForNonExistentProperty`: the lib that declares
/// `member` on the interface the receiver's apparent type is an instance of.
pub(crate) fn lib_feature_of_missing_member(receiver: &Type, member: &str) -> Option<&'static str> {
    let container = apparent_symbol_name(receiver)?;
    FEATURE_MAP
        .iter()
        .find(|(interface, _)| *interface == container)?
        .1
        .iter()
        .find(|(_, members)| members.contains(&member))
        .map(|(lib, _)| *lib)
}

/// The name of `getApparentType(receiver).symbol`: the global interface a
/// primitive or array reads its members from, or the declaration a reference
/// names.
fn apparent_symbol_name(receiver: &Type) -> Option<String> {
    Some(match receiver {
        Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) => "Array".to_string(),
        Type::String | Type::StringLiteral(_) => "String".to_string(),
        Type::Number | Type::NumberLiteral(_) => "Number".to_string(),
        Type::BigInt => "BigInt".to_string(),
        Type::Symbol => "Symbol".to_string(),
        Type::Reference(reference) if !reference.is_readonly_array() => {
            let name = reference.id.rsplit('\0').next()?;
            name.rsplit('.').next()?.to_string()
        }
        Type::Object(object) => match object.alias_name.as_deref().and_then(|name| name.strip_prefix("typeof ")) {
            Some(namespace) => namespace.to_string(),
            None => receiver.name(),
        },
        _ => return None,
    })
}
