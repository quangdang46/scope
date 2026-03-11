import { verifyToken } from "../../src/app";

export function appSpec(): boolean {
  return verifyToken("signed:value");
}
