// An expression-bodied arrow whose body does not fit the contextual return type
// is reported at the body, whatever the size of the expected interface.
declare function produceElement(producer: () => Element): void;
produceElement(() => undefined);
declare function produceWindow(producer: () => Window): void;
produceWindow(() => 1);
function generic<T extends Window>(producer: () => T) { }
generic<Window>(() => undefined);
generic<Window>(() => null);
