type Conflicting = {
    [key: `a${string}`]: 'a';
    [key: `${string}a`]: 'b';
    [key: `a${string}a`]: 'c';
}
interface IC {
    [key: `a${string}`]: 'a';
    [key: `a${string}a`]: 'c';
}
