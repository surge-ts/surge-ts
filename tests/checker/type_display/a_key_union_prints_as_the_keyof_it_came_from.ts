// @surge-compare: messages
interface Shape { name: string; width: number; height: number }
function getProperty(key: keyof Shape) {}
getProperty("size");
const k: keyof Shape = "depth";
