declare var x: any;
declare function isFunction(x: unknown): x is Function;
declare function isObject(x: unknown): x is Object;
declare function isAnything(x: unknown): x is {};
declare function isError(x: unknown): x is Error;
if (isFunction(x)) {
    x();
    x.prop;
}
if (isObject(x)) {
    x.method();
}
if (isAnything(x)) {
    x.method();
}
if (isError(x)) {
    x.message;
    x.mesage;
}
