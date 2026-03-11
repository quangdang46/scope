import { verifyToken } from "../../src/auth/middleware";

export function middlewareSpec(): boolean {
  return verifyToken("signed:value");
}
