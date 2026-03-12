export function verifyToken(token: string): boolean {
  return token.startsWith("token:");
}
