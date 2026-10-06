type Thing = { foo: string | number; bar(): number; baz: object };

function f40(o: Thing | undefined) {
  switch (o?.foo) {
    case "abc":
      o.foo;
      break;
    case 42:
      o.foo;
      break;
    case undefined:
      o.foo;
      break;
    default:
      o.foo;
      break;
  }
}

function f41(o: Thing | undefined) {
  switch (typeof o?.foo) {
    case "string":
      o.foo;
      break;
    case "number":
      o.foo;
      break;
    case "undefined":
      o.foo;
      break;
    default:
      o.foo;
      break;
  }
}

function f42(o: Thing | undefined) {
  switch (o?.foo) {
    case "abc":
    case 42:
      return;
  }
  o.foo;
}

export { f40, f41, f42 };
