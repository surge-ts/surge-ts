// `x op= v` with an operand of the wrong type reports the operand and leaves
// the assignment itself unchecked (`checkAssignmentOperator` needs both
// operands valid); `x = x op v` still checks the write.
declare var flag: boolean;
declare var text: string;
flag *= true;
flag -= 1;
text /= 2;
text **= "";
flag = flag * 2;
