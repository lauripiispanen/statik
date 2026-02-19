export function formatUser(name: string, email: string): string {
  return `${name} <${email}>`;
}

export function formatList(items: string[]): string {
  return items.join(", ");
}
