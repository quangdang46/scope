import { log } from "./logger";

export function format(value: string): string {
  log(value);
  return value.trim();
}
