import { verify } from "./jwt";

export function verifyToken(token: string): boolean {
  return verify(token);
}
