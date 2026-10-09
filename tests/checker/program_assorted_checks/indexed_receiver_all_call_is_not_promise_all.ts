// @strict: true
declare const query: { [key: string]: unknown; all(placeholder?: Record<string, unknown>): string[] };
export const rows: string[] = query.all();
export const count: number = query.all().length;
