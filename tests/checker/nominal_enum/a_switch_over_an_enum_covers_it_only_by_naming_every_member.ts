enum G { a = 1, b = 2 }
function partial(g: G): number {
    switch (g) {
        case G.a: return 1;
    }
}
function complete(g: G): number {
    switch (g) {
        case G.a: return 1;
        case G.b: return 2;
    }
}

enum Merged { x = 1 }
enum Merged { y = 2 }
function mergedPartial(m: Merged): number {
    switch (m) {
        case Merged.x: return 1;
    }
}
function mergedComplete(m: Merged): number {
    switch (m) {
        case Merged.x: return 1;
        case Merged.y: return 2;
    }
}

enum Label { one = "one", two = "two" }
function label(t: Label): string {
    switch (t) {
        case Label.one: return "1";
    }
}
