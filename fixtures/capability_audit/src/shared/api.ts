import { fetch } from "../http/client"

export async function callApi(url: string) {
  return fetch(url)
}
