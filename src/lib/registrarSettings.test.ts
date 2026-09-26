import assert from "node:assert/strict";
import test from "node:test";
import { optionalPrice, parseApiPairs } from "./registrarSettings.ts";

test("API pairs accept multiple profiles and reject malformed credentials", () => {
  const hash = "0123456789abcdef0123456789abcdef";
  assert.deepEqual(parseApiPairs(`1 : ${hash}\r\n\n2:${hash}`), [
    { api_id: 1, api_hash: hash }, { api_id: 2, api_hash: hash },
  ]);
  for (const value of ["", `0:${hash}`, `2147483648:${hash}`, "1:abc", `1:${hash}:extra`]) {
    assert.throws(() => parseApiPairs(value), /invalidApiPairs/);
  }
});

test("optional prices preserve zero and reject nonfinite or negative values", () => {
  assert.equal(optionalPrice(" "), null);
  assert.equal(optionalPrice("0"), 0);
  assert.equal(optionalPrice("1.25"), 1.25);
  for (const value of ["-1", "NaN", "Infinity", "1,25"]) {
    assert.throws(() => optionalPrice(value), /invalidPrice/);
  }
});
