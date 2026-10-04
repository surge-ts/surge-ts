type Recurse = {
    [K in keyof Recurse]: Recurse[K]
}

type R1 = {
    [K in keyof R2]: R1[K]
}
type R2 = {
    [K in keyof R1]: R2[K]
}

interface Foo { hello: string }

function f1<T extends T["hello"]>() {}
function f3<T extends Foo | T["hello"]>() {}
function f6<T extends U["a"], U extends { a: T }>() {}
function f8<T extends Foo | U, U extends T>() {}

function fine1<T extends keyof T>() {}
function fine2<T extends Array<T>>() {}
function fine3<T extends (Foo | T)[]>() {}
