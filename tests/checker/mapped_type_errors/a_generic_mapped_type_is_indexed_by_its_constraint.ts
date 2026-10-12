type AB = {
    a: 'a'
    b: 'a'
};

type T1<K extends keyof AB> = { [key in AB[K]]: true };
type T2<K extends 'a'|'b'> = T1<K>[K];

type R = AB[keyof AB];
type T3 = { [key in R]: true };
type T4<K extends 'a'|'b'> = T3[K];

type T5<S extends 'a'|'b'|'extra'> = {[key in AB[S]]: true}[S];

type T6<S extends 'a'|'b', L extends 'a'|'b'> = {[key in AB[S]]: true}[L];

type T7<S extends 'a'|'b', L extends 'a'> = {[key in AB[S]]: true}[L];

type Id = { a: 'a'; b: 'b' };
type T8<K extends 'a'|'b'> = { [P in Id[K]]: 1 }[K];
