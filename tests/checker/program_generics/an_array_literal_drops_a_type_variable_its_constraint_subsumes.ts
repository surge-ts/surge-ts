// @strict: true
// An array literal's element union is subtype-reduced: a type variable whose
// constraint another element's type subsumes goes.
class Plain {
  str(): string { return ""; }
}
function h<T extends Plain>(t: T, plain: Plain) {
  var both: Plain[];
  var both = [t, plain];
  return both;
}
