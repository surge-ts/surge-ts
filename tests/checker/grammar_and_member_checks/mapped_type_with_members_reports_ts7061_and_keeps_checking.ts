type After = {
    [placeType in "a" | "b"]: void;
    model: 'hour' | 'day'
}

type AfterImplicit = {
    [placeType in "a"]
    model: 'hour' | 'day';
}

const E = class {
    [P in 'a' | 'b']: any
}
