export function parseApiPairs(value: string): { api_id: number; api_hash: string }[] {
  const pairs = value.split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
  if (!pairs.length || pairs.length > 1000) throw new Error("registrar.invalidApiPairs");
  return pairs.map((line) => {
    const match = /^(\d+)\s*:\s*([a-fA-F0-9]{32})$/.exec(line);
    if (!match) throw new Error("registrar.invalidApiPairs");
    const apiId = Number(match[1]);
    if (!Number.isInteger(apiId) || apiId <= 0 || apiId > 2147483647) throw new Error("registrar.invalidApiPairs");
    return { api_id: apiId, api_hash: match[2] };
  });
}

export function optionalPrice(value: string): number | null {
  if (!value.trim()) return null;
  const price = Number(value);
  if (!Number.isFinite(price) || price < 0) throw new Error("registrar.invalidPrice");
  return price;
}
