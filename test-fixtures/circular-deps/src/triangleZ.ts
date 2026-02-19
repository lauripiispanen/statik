// Triangular cycle: X -> Y -> Z -> X
import { TriangleY } from "./triangleY";

export class TriangleZ {
  getValue(): number {
    return 3;
  }

  getYValue(): number {
    const y = new TriangleY();
    return y.getValue();
  }
}
