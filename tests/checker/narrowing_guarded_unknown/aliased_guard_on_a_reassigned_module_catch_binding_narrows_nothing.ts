// @useUnknownInCatchVariables: true
try {}
catch (e) {
    const isString = typeof e === "string";
    if (isString) {
        e.toUpperCase();
    }
}
try {}
catch (e) {
    const isString = typeof e === "string";
    e = 1;
    if (isString) {
        e.toUpperCase();
    }
}
