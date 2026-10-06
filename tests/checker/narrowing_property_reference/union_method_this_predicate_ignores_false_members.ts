class Entry {
    c = 1;
    isInit(x: any): this is Entry { return true }
}
class Group {
    d = 'no';
    isInit(x: any): false { return false }
}
class Other {
    e = 1;
    isInit(x: any): boolean { return false }
}
function f(chunk: Entry | Group) {
    return chunk.isInit(chunk) ? chunk.c : chunk.d;
}
function g(chunk: Entry | Other) {
    return chunk.isInit(chunk) ? chunk.c : 0;
}
