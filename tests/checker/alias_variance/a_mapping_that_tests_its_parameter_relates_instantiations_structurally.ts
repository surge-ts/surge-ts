interface F<P> {
    (props: P & { children?: boolean }): void;
    propTypes: { [K in keyof P]: null extends P ? K : K };
}
declare function g(C: F<unknown>): string;
export function wu<CP extends { o: object }>(CC: F<CP>) {
    g(CC);
}
