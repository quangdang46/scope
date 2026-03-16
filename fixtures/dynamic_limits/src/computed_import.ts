export async function loadFeature(specifier: string) {
  return import(specifier);
}
