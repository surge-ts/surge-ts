// @noImplicitReturns: true
function throughTry(x: number): number {
    test:
    try {
        do {
            do {
                break test;
            } while (true);
            x++;
        } while (true);
    }
    catch (e) {
        return 1;
    }
}
function outOfTry(x: number): number {
    test:
    try {
        break test;
    }
    catch (e) {
        return 1;
    }
}
function outOfBlock(x: number): number {
    test: {
        break test;
    }
}
function outOfItsOwnLoop(x: number): number {
    test:
    do {
        break test;
    } while (true);
}
function outOfAnOuterLoop(x: number): number {
    test:
    do {
        do { break test; } while (true);
    } while (true);
}
function guardedBlock(x: number): number {
    outer: {
        if (x) break outer;
        return 1;
    }
}
function endlessOuterLoop(x: number): number {
    outer: while (true) {
        while (true) {
            if (x) continue outer;
        }
    }
}
function everyPathReturns(x: number): number {
    outer: {
        if (x) break outer;
        return 1;
    }
    return 2;
}
