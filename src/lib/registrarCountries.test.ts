import assert from "node:assert/strict";
import test from "node:test";
import { countryCallingCode, sortRegistrarCountries } from "./registrarCountries.ts";

test("country calling codes use names or ISO codes, not provider numeric IDs", () => {
  assert.equal(countryCallingCode({ id: "0", name: "Россия" }), "+7");
  assert.equal(countryCallingCode({ id: "2", name: "Kazakhstan" }), "+7");
  assert.equal(countryCallingCode({ id: "99", name: "United States of America" }), "+1");
  assert.equal(countryCallingCode({ id: "GB", name: "unknown" }), "+44");
  assert.equal(countryCallingCode({ id: "7", name: "unknown" }), undefined);
  assert.equal(countryCallingCode({ id: "x", name: "🇺🇸 США (виртуальные)" }), "+1");
  assert.equal(countryCallingCode({ id: "y", name: "🇨🇩 Дем. Конго" }), "+243");
  assert.equal(countryCallingCode({ id: "z", name: "🇻🇨 Сент-Винсент и Гренадин" }), "+1");
});

test("country order follows the cheapest available price without mutating inputs", () => {
  const countries = [{ id: "a", name: "A" }, { id: "b", name: "B" }, { id: "c", name: "C" }];
  const prices = [
    { country: countries[0], price: 0.1, available: 0, operator: null },
    { country: countries[0], price: 5, available: 2, operator: null },
    { country: countries[1], price: 2, available: null, operator: null },
    { country: countries[2], price: NaN, available: 5, operator: null },
  ];
  assert.deepEqual(sortRegistrarCountries(countries, prices).map((country) => country.id), ["b", "a", "c"]);
  assert.deepEqual(countries.map((country) => country.id), ["a", "b", "c"]);
});

test("equal prices have stable alphabetical ordering", () => {
  assert.deepEqual(sortRegistrarCountries([
    { id: "b", name: "B" }, { id: "a", name: "A" },
  ], []).map((country) => country.id), ["a", "b"]);
});
