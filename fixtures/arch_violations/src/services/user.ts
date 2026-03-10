import { renderRoute } from "../routes/http";
import { formatName } from "../utils/format";

export function userLabel(name: string): string {
  return renderRoute(formatName(name));
}
