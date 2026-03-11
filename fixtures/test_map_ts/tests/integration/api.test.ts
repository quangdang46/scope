import { verifyToken } from "../../src/routes/api";

export function apiSpec(): boolean {
  return verifyToken("signed:value");
}
