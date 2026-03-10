import { userLabel } from "../services/user";

export function accountName(name: string): string {
  return userLabel(name);
}
