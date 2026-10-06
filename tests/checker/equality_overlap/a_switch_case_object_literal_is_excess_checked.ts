class C { id: number = 0; }
class D extends C { name: string = ""; }

declare const c: C;
switch (c) {
  case new D():
  case { id: 12, name: "" }:
  case { id: 12 }:
  case new C():
    break;
}
