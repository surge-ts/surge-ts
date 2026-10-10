// An object whose `then` is callable but takes no callback cannot be adopted
// as a promise: awaiting it is TS1320, returning it from an async function
// TS1058 (at the function when its return type is inferred).
declare class Thenable {
    then(): void;
}
declare let thenable: Thenable;
declare let notThenable: { then: string };
declare let promise: Promise<number>;
async function awaitsStatement() {
    await thenable;
}
async function awaitsInitializer() {
    const value = await thenable;
}
async function returnsAwaited() {
    return await thenable;
}
async function returnsThenable() {
    return thenable;
}
async function awaitsValid() {
    await notThenable;
    await promise;
    return notThenable;
}
