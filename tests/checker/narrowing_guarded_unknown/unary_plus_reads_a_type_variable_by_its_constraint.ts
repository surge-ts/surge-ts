// @strict: true
// @target: es2020
function getKey<S extends number | bigint>(key: S) {
    +key;
}
function big<S extends bigint>(key: S) {
    +key;
}
function num<S extends number>(key: S) {
    +key;
    -key;
}
