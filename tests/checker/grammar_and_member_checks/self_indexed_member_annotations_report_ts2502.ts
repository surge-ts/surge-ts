type T1 = {
    x: T1["x"];
};

interface T3<T extends T3<T>> {
    x: T["x"];
}

interface T4<T extends T4<T>> {
    x: T4<T>["x"];
}

class C1 {
    x!: C1["x"];
}

type Fine = {
    a: string;
    b: Fine["a"];
};
