const secretValue = 42;

export { secretValue as renamed };

export { secretValue as alsoRenamed };

export function getSecret(): number {
  return secretValue;
}
