export function formatDate(date: Date): string {
  return date.toISOString().split("T")[0];
}

export function formatDateTime(date: Date): string {
  return date.toISOString().replace("T", " ").split(".")[0];
}

export function formatCurrency(amount: number, currency: string = "USD"): string {
  return new Intl.NumberFormat("en-US", { style: "currency", currency }).format(amount);
}

export function formatLogMessage(level: string, module: string, message: string): string {
  return `[${formatDateTime(new Date())}] [${level.toUpperCase()}] [${module}] ${message}`;
}

export function truncate(str: string, maxLength: number): string {
  if (str.length <= maxLength) return str;
  return str.substring(0, maxLength - 3) + "...";
}

export function capitalize(str: string): string {
  return str.charAt(0).toUpperCase() + str.slice(1);
}
