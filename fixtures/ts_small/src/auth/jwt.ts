export function sign(payload: string): string {
  return `signed:${payload}`;
}

export function verify(token: string): boolean {
  return token.startsWith("signed:");
}
