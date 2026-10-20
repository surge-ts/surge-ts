// @strict: true
// A getter's type is the union of what its body returns, of which only a
// lone literal widens (`getReturnTypeFromBody`).
class Connection {
    get label() {
        if (Math.random() > 0.5) {
            return "open";
        }
        return "closed";
    }
    get single() {
        return "only";
    }
}
declare const connection: Connection;
export const label: "open" | "closed" = connection.label;
export const single: "only" = connection.single;
