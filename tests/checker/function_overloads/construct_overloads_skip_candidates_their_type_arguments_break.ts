interface C4 {
    new<T extends string, U extends number>(n: T, m: U): {};
    new<T extends number, U extends string>(n: T, m: U): {};
}
declare var c4: C4;
new c4<string, number>('', 3);
new c4<string, number>(3, '');
new c4<number, string>('', 3);
new c4<boolean, Date>(null!, null!);
