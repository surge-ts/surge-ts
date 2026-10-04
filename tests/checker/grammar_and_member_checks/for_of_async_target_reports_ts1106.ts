var async: any;
for (async of [1, 2]) {}
for ((async) of [1, 2]) {}
async function f() {
    for (async of [1]) {}
}
