import { callApi } from "../shared/api"

export async function main() {
  return callApi("https://example.com/cli")
}
