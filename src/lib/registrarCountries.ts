/// <reference lib="es2021.intl" />

import { getCountries, getCountryCallingCode } from "libphonenumber-js/min";
import { COUNTRY_NAMES, COUNTRY_NAMES_EN } from "./countryData.ts";

export interface RegistrarCountry {
  id: string;
  name: string;
}

export interface RegistrarPrice {
  country: RegistrarCountry;
  price: number;
  available: number | null;
  operator: string | null;
}

function normalizeCountryName(name: string): string {
  return name.normalize("NFKD").replace(/\p{M}/gu, "").toLowerCase().replace(/[^\p{L}\p{N}]/gu, "");
}

const callingCodes = new Map<string, string>();
const englishNames = new Intl.DisplayNames(["en"], { type: "region" });
const russianNames = new Intl.DisplayNames(["ru"], { type: "region" });

for (const country of getCountries()) {
  const callingCode = `+${getCountryCallingCode(country)}`;
  for (const name of [
    country, englishNames.of(country), russianNames.of(country),
    COUNTRY_NAMES[country], COUNTRY_NAMES_EN[country],
  ]) {
    if (name) callingCodes.set(normalizeCountryName(name), callingCode);
  }
}

const aliases: Record<string, string> = {
  USA: "US", "United States of America": "US", UK: "GB", England: "GB",
  "Great Britain": "GB", "Russian Federation": "RU", "Czech Republic": "CZ",
  "South Korea": "KR", "North Korea": "KP", "Republic of Korea": "KR",
  "Ivory Coast": "CI", "Hong Kong": "HK", "Taiwan": "TW", "Macao": "MO",
  "Cape Verde": "CV", "Swaziland": "SZ", "East Timor": "TL",
  "DR Congo": "CD", "Congo Republic": "CG", "Congo (Dem. Rep.)": "CD",
  "США (виртуальные)": "US", "Дем. Конго": "CD", "Макао": "MO",
  "Сауд. Аравия": "SA", "Тимор-Лесте": "TL", "Свазиленд": "SZ",
  "ЦАР": "CF", "Сент-Винсент и Гренадин": "VC", "Острова Кайман": "KY",
  "Южная Корея": "KR", "Северная Корея": "KP",
  "Соломоновы острова": "SB", "Бермуды": "BM",
};

for (const [name, country] of Object.entries(aliases)) {
  const code = callingCodes.get(normalizeCountryName(country));
  if (code) callingCodes.set(normalizeCountryName(name), code);
}

export function countryCallingCode(country: RegistrarCountry): string | undefined {
  return callingCodes.get(normalizeCountryName(country.name))
    ?? callingCodes.get(normalizeCountryName(country.id));
}

export function sortRegistrarCountries(
  countries: RegistrarCountry[],
  prices: RegistrarPrice[],
): RegistrarCountry[] {
  const lowestPrices = new Map<string, number>();
  for (const price of prices) {
    if (!Number.isFinite(price.price) || price.price < 0 || price.available === 0) continue;
    lowestPrices.set(price.country.id, Math.min(lowestPrices.get(price.country.id) ?? Infinity, price.price));
  }
  return [...countries].sort((left, right) => {
    const leftPrice = lowestPrices.get(left.id) ?? Infinity;
    const rightPrice = lowestPrices.get(right.id) ?? Infinity;
    if (leftPrice !== rightPrice) return leftPrice < rightPrice ? -1 : 1;
    return left.name.localeCompare(right.name) || left.id.localeCompare(right.id);
  });
}
