export function almostValid(): string {
  return "this function is fine";
}

export function brokenFunction( {
  // missing closing paren and body
  const x = 42
  if (x > 10 {
    console.log("unclosed brace"
  }

export class PartialClass {
  method(): void {
    // this class is syntactically incomplete
