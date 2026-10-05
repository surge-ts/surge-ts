function fn4<T extends string, U extends number>(n: T, m: U): void;
function fn4<T extends number, U extends string>(n: T, m: U): void;
function fn4() { }
fn4<boolean, Date>(null!, null!);
fn4<string, number>("", 1);
