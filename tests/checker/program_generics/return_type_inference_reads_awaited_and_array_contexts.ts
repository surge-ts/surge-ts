// @strict: true
interface Stmt {
  raw<T = unknown[]>(options?: { columnNames?: boolean }): Promise<T[]>;
  sync<T = unknown[]>(): T[];
}
declare const stmt: Stmt;

export async function awaitedReturn<T extends any[] = unknown[]>(): Promise<T[]> {
  return stmt.raw();
}
export async function awaitedOperand<T extends any[] = unknown[]>(): Promise<T[]> {
  return await stmt.raw();
}
export function promiseReturn<T extends any[] = unknown[]>(): Promise<T[]> {
  return stmt.raw();
}
export function arrayReturn<T extends any[] = unknown[]>(): T[] {
  return stmt.sync();
}
export async function concrete(): Promise<string[][]> {
  const rows: string[][] = await stmt.raw();
  return rows;
}
export function notAwaited(): string[][] {
  return stmt.raw();
}
interface Client {
  transaction<T>(action: (tx: string) => Promise<T>): Promise<T>;
}
declare const client: Client;
export async function nestedTransaction<T>(transaction: (tx: number) => Promise<T>): Promise<T> {
  return await client.transaction(async (clientTx) => {
    return await transaction(clientTx.length);
  });
}
