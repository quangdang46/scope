import { loadFeature } from "./computed_import";
import { loadPlugin } from "./dynamic_require";

export async function boot(name) {
  await loadFeature(`./plugins/${name}.js`);
  return loadPlugin(name);
}
