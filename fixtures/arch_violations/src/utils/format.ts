import { accountName } from "../models/account";

export function formatName(name: string): string {
  return accountName(name).trim();
}
