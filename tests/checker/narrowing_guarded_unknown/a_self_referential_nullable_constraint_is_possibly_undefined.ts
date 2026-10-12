function fx3<T extends Record<keyof T, string> | undefined, K extends keyof T>(obj: T, key: K) {
    const x1 = obj[key];
    const x2 = obj && obj[key];
}
class TableBaseEnum<
    PublicSpec extends Record<keyof InternalSpec, any>,
    InternalSpec extends Record<keyof PublicSpec, any> | undefined = undefined> {
    m() {
        let iSpec = null! as InternalSpec;
        iSpec[null! as keyof InternalSpec];
        iSpec[null! as keyof PublicSpec];
        if (iSpec === undefined) {
            return;
        }
        iSpec[null! as keyof InternalSpec];
    }
}
