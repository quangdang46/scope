import { callApi } from "../shared/api"

export async function runJob() {
  return callApi("https://example.com/worker")
}
