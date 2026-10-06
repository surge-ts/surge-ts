declare let x: { [x: `${string}xxx${string}` & `${string}yyy${string}`]: string };
const a1 = x['axxxbbbyc'];
const a2 = x['axxxbbbyyyc'];
const a3: 7 = a2;
