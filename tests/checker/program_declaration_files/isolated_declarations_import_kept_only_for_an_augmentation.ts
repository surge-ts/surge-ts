// @isolatedDeclarations: true
// @declaration: true
// @strict: true
// @filename: augmenter.ts
import { Host } from './host';
declare module './host' {
    interface Host {
        added: number;
    }
}
export function touch(host: Host): void {
    host.added = 1;
}
export const marker: number = 1;
// @filename: host.ts
import { touch } from './augmenter';
export class Host implements Host {}
touch(Host.prototype);
// @filename: user.ts
import { marker } from './augmenter';
export const copy: typeof marker = marker;
// @filename: other.ts
import { touch } from './augmenter';
touch(null!);
