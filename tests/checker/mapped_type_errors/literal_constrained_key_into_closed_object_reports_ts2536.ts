type AB = {
    a: 'a'
    b: 'a'
};
type R = AB[keyof AB];
type T3 = { [key in R]: true };
type T4<K extends 'a' | 'b'> = T3[K];
type T5<K extends 'a'> = T3[K];
type T6<K extends 'a' | 'b'> = AB[K];
type T7<K extends 'a' | 'c'> = { [k: string]: number }[K];
