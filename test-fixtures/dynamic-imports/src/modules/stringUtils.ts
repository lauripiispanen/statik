export function formatName(first: string, last: string): string {
  return `${capitalize(first)} ${capitalize(last)}`;
}

function capitalize(str: string): string {
  return str.charAt(0).toUpperCase() + str.slice(1);
}

export function padLeft(str: string, length: number, char: string = " "): string {
  return str.padStart(length, char);
}

export function reverseString(str: string): string {
  return str.split("").reverse().join("");
}
