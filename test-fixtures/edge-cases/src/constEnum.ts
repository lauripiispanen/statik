export const enum Color {
  Red = 0,
  Green = 1,
  Blue = 2,
}

export enum Direction {
  Up = "UP",
  Down = "DOWN",
  Left = "LEFT",
  Right = "RIGHT",
}

export const enum Priority {
  Low = 0,
  Medium = 1,
  High = 2,
  Critical = 3,
}

export function colorToString(color: Color): string {
  switch (color) {
    case Color.Red: return "red";
    case Color.Green: return "green";
    case Color.Blue: return "blue";
  }
}
