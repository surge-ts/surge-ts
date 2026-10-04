function fn4<T extends string, U extends number>(n: T, m: U): void;
function fn4<T extends number, U extends string>(n: T, m: U): void;
function fn4() { }

fn4<string, number>('', 3);
fn4<string, number>(3, '');
fn4<number, string>('', 3);
fn4<number, string>(3, '');
