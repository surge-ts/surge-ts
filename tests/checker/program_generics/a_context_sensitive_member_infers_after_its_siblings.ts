// @strict: true
declare const load: () => Promise<string[]>;
declare function query<A, D = A>(options: { queryFn?: () => A | Promise<A>; select?: (data: A) => D }): [A, D];
const selected: [] = query({ queryFn: load, select: (data) => data.length });
const defaulted: [] = query({ queryFn: load });

interface Options<A = unknown, D = A, K extends readonly unknown[] = readonly unknown[]> {
    queryFn?: (context: K) => A;
    initialData?: D;
}
type Mapped<A, D, K extends readonly unknown[]> = { [P in keyof Options<A, D, K>]: Options<A, D, K>[P] };
declare function useQuery<A = unknown, D = A, K extends readonly unknown[] = readonly unknown[]>(
    options: Mapped<A, D, K> & { initialData?: undefined },
): { data: D; key: K };
const result: [] = useQuery({ queryFn: () => 1 });
