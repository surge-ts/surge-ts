// @surge-compare: messages
function computedKey({ [C]: q }: any, C: string) {}
function nestedComputedKey({ x: { [C]: y } }: any, C: string) {}
function keyAndDefault({ [C]: b = C }: any, C: string) {}
function patternTargetDefault({ x: { y, z } = C }: any, C: any) {}
function patternTargetReadsItsOwnBinding({ a: { b } = b }: any) {}
function earlierSiblingInThePattern({ C, [C]: q }: any) {}
function keyWithAnEmptyValue({ [C]: {} }: any, C: string) {}
function emptyPatternInitializer({} = C, C?: any) {}
function multiLinePattern({
  [C]: q
}: any, C: string) {}
function identifierDefault({ a = C }: any, C: string) {}
function identifierSelfReference(a = a) {}
