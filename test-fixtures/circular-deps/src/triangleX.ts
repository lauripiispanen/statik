// Triangular cycle: X -> Y -> Z -> X
import { TriangleZ } from "./triangleZ";

export class TriangleX {
  getValue(): number {
    return 1;
  }

  getZValue(): number {
    const z = new TriangleZ();
    return z.getValue();
  }
}
