// Triangular cycle: X -> Y -> Z -> X
import { TriangleX } from "./triangleX";

export class TriangleY {
  getValue(): number {
    return 2;
  }

  getXValue(): number {
    const x = new TriangleX();
    return x.getValue();
  }
}
