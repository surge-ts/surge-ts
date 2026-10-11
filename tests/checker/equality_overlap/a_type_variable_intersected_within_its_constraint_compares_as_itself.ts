// @surge-compare: messages
function exact<T extends {}>(value: T & {}) {
    if (value === 42) { }
}
function withNull<T extends {}>(value: T & ({} | null)) {
    if (value === 42) { }
}
function covered<T extends {} | null>(value: T & ({} | null)) {
    if (value === 42) { }
}
function unconstrained<T>(value: T & ({} | null)) {
    if (value === 42) { }
}
function literals<T extends "a" | "b">(value: T & string) {
    if (value === 1) { }
}
