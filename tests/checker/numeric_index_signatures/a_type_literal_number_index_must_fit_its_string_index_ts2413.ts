var a: { [x: string]: string; [x: number]: number; };
function other<T>(arg: T) {
    var b: {
        [x: string]: Object;
        [x: number]: T;
    };
}
