// @strict: false
declare var chars: string[];
function f1() {
    for (var i in chars) { }
    var i: number;
}
function f3() {
    for (var j in chars) { var j: number; }
}
function f4() {
    for (var k in chars) { chars[k]; }
    var k: string;
}
