export function formatResult(operation: string, result: number): string {
  return `${operation} = ${result}`;
}

export function formatError(error: Error): string {
  return `ERROR: ${error.message}`;
}
