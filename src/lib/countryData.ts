export const CIS_COUNTRIES = ["RU", "UA", "BY", "KZ", "UZ", "KG", "TJ", "TM", "AZ", "AM", "MD", "GE"];
export const ASIA_COUNTRIES = ["VN", "TH", "ID", "PH", "MY", "SG", "JP", "KR", "CN", "IN", "BD", "PK", "MM", "KH", "LA", "HK", "TW", "MN", "NP", "LK", "AF", "BN", "TL", "MV", "BT", "IR", "IQ"];
export const EUROPE_COUNTRIES = ["DE", "GB", "FR", "IT", "ES", "PL", "NL", "BE", "SE", "NO", "FI", "DK", "AT", "CH", "CZ", "PT", "IE", "RO", "HU", "BG", "HR", "SK", "LT", "LV", "EE", "RS", "BA", "AL", "MK", "ME", "XK", "CY", "MT", "IS", "LU", "AD", "MC", "SM", "LI", "FO", "GI"];
export const AMERICAS_COUNTRIES = ["US", "CA", "BR", "MX", "AR", "CO", "CL", "PE", "VE", "EC", "BO", "PY", "UY", "DO", "GT", "HN", "SV", "CR", "PA", "CU", "JM", "HT", "TT", "BB", "BS", "BZ", "SR", "GY", "NI", "PR", "AW", "CW"];
export const AFRICA_COUNTRIES = ["DZ", "MA", "TN", "LY", "EG", "SD", "ET", "KE", "GH", "TZ", "UG", "CM", "CI", "SN", "MG", "NG", "ZA", "MZ", "AO", "ZW", "ZM", "BW", "NA", "MW", "RW", "BJ", "TG", "NE", "ML", "BF", "GA", "CG", "CD", "TD", "MR", "SO", "DJ", "ER", "SS", "SL", "LR", "GM", "GN", "SC", "MU", "CV"];
export const MIDDLE_EAST_COUNTRIES = ["TR", "IL", "AE", "SA", "JO", "LB", "KW", "QA", "BH", "OM", "YE", "SY", "PS"];

export const COUNTRY_NAMES: Record<string, string> = {
  ANON: "Анонимный",
  RU: "Россия", UA: "Украина", BY: "Беларусь", KZ: "Казахстан",
  UZ: "Узбекистан", KG: "Кыргызстан", TJ: "Таджикистан", TM: "Туркменистан",
  AZ: "Азербайджан", AM: "Армения", MD: "Молдова", GE: "Грузия",
  VN: "Вьетнам", TH: "Таиланд", ID: "Индонезия", PH: "Филиппины",
  MY: "Малайзия", SG: "Сингапур", JP: "Япония", KR: "Корея",
  CN: "Китай", IN: "Индия", BD: "Бангладеш", PK: "Пакистан",
  MM: "Мьянма", KH: "Камбоджа", LA: "Лаос",
  DE: "Германия", GB: "Великобритания", FR: "Франция", IT: "Италия",
  ES: "Испания", PL: "Польша", NL: "Нидерланды", BE: "Бельгия",
  SE: "Швеция", NO: "Норвегия", FI: "Финляндия", DK: "Дания",
  AT: "Австрия", CH: "Швейцария", CZ: "Чехия", PT: "Португалия",
  IE: "Ирландия", RO: "Румыния", HU: "Венгрия", BG: "Болгария",
  HR: "Хорватия", SK: "Словакия", LT: "Литва", LV: "Латвия", EE: "Эстония",
  US: "США", CA: "Канада", BR: "Бразилия", MX: "Мексика",
  AR: "Аргентина", CO: "Колумбия", CL: "Чили", PE: "Перу", VE: "Венесуэла",
  TR: "Турция", IL: "Израиль", AE: "ОАЭ", SA: "Саудовская Аравия",
  EG: "Египет", NG: "Нигерия", ZA: "ЮАР", AU: "Австралия", NZ: "Новая Зеландия",
  DZ: "Алжир", MA: "Марокко", TN: "Тунис", LY: "Ливия", SD: "Судан",
  ET: "Эфиопия", KE: "Кения", GH: "Гана", TZ: "Танзания", UG: "Уганда",
  CM: "Камерун", CI: "Кот-д'Ивуар", SN: "Сенегал", MG: "Мадагаскар",
  IQ: "Ирак", AF: "Афганистан", NP: "Непал", LK: "Шри-Ланка",
  HK: "Гонконг", TW: "Тайвань", MN: "Монголия",
  EC: "Эквадор", BO: "Боливия", PY: "Парагвай", UY: "Уругвай",
  DO: "Доминикана", GT: "Гватемала", HN: "Гондурас", SV: "Сальвадор",
  CR: "Коста-Рика", PA: "Панама", CU: "Куба", JM: "Ямайка",
  RS: "Сербия", BA: "Босния", AL: "Албания", MK: "Северная Македония",
  ME: "Черногория", XK: "Косово", CY: "Кипр", MT: "Мальта",
  IS: "Исландия", LU: "Люксембург",
  MZ: "Мозамбик", AO: "Ангола", ZW: "Зимбабве", ZM: "Замбия",
  BW: "Ботсвана", NA: "Намибия", MW: "Малави", RW: "Руанда",
  BJ: "Бенин", TG: "Того", NE: "Нигер", ML: "Мали", BF: "Буркина-Фасо",
  GA: "Габон", CG: "Конго", CD: "ДР Конго", TD: "Чад", MR: "Мавритания",
  SO: "Сомали", DJ: "Джибути", ER: "Эритрея", SS: "Южный Судан",
  SL: "Сьерра-Леоне", LR: "Либерия", GM: "Гамбия", GN: "Гвинея",
  SC: "Сейшелы", MU: "Маврикий", CV: "Кабо-Верде",
  JO: "Иордания", LB: "Ливан", KW: "Кувейт", QA: "Катар",
  BH: "Бахрейн", OM: "Оман", YE: "Йемен", SY: "Сирия", PS: "Палестина",
  IR: "Иран",
  BN: "Бруней", TL: "Восточный Тимор", MV: "Мальдивы", BT: "Бутан",
  FJ: "Фиджи", PG: "Папуа-Новая Гвинея", WS: "Самоа", TO: "Тонга",
  HT: "Гаити", TT: "Тринидад и Тобаго", BB: "Барбадос", BS: "Багамы",
  BZ: "Белиз", SR: "Суринам", GY: "Гайана", NI: "Никарагуа",
  PR: "Пуэрто-Рико", AW: "Аруба", CW: "Кюрасао",
  AD: "Андорра", MC: "Монако", SM: "Сан-Марино", LI: "Лихтенштейн",
  FO: "Фарерские острова", GI: "Гибралтар", GL: "Гренландия",
};

export const COUNTRY_NAMES_EN: Record<string, string> = {
  ANON: "Anonymous",
  RU: "Russia", UA: "Ukraine", BY: "Belarus", KZ: "Kazakhstan",
  UZ: "Uzbekistan", KG: "Kyrgyzstan", TJ: "Tajikistan", TM: "Turkmenistan",
  AZ: "Azerbaijan", AM: "Armenia", MD: "Moldova", GE: "Georgia",
  VN: "Vietnam", TH: "Thailand", ID: "Indonesia", PH: "Philippines",
  MY: "Malaysia", SG: "Singapore", JP: "Japan", KR: "Korea",
  CN: "China", IN: "India", BD: "Bangladesh", PK: "Pakistan",
  MM: "Myanmar", KH: "Cambodia", LA: "Laos",
  DE: "Germany", GB: "United Kingdom", FR: "France", IT: "Italy",
  ES: "Spain", PL: "Poland", NL: "Netherlands", BE: "Belgium",
  SE: "Sweden", NO: "Norway", FI: "Finland", DK: "Denmark",
  AT: "Austria", CH: "Switzerland", CZ: "Czechia", PT: "Portugal",
  IE: "Ireland", RO: "Romania", HU: "Hungary", BG: "Bulgaria",
  HR: "Croatia", SK: "Slovakia", LT: "Lithuania", LV: "Latvia", EE: "Estonia",
  US: "USA", CA: "Canada", BR: "Brazil", MX: "Mexico",
  AR: "Argentina", CO: "Colombia", CL: "Chile", PE: "Peru", VE: "Venezuela",
  TR: "Turkey", IL: "Israel", AE: "UAE", SA: "Saudi Arabia",
  EG: "Egypt", NG: "Nigeria", ZA: "South Africa", AU: "Australia", NZ: "New Zealand",
  DZ: "Algeria", MA: "Morocco", TN: "Tunisia", LY: "Libya", SD: "Sudan",
  ET: "Ethiopia", KE: "Kenya", GH: "Ghana", TZ: "Tanzania", UG: "Uganda",
  CM: "Cameroon", CI: "Ivory Coast", SN: "Senegal", MG: "Madagascar",
  IQ: "Iraq", AF: "Afghanistan", NP: "Nepal", LK: "Sri Lanka",
  HK: "Hong Kong", TW: "Taiwan", MN: "Mongolia",
  EC: "Ecuador", BO: "Bolivia", PY: "Paraguay", UY: "Uruguay",
  DO: "Dominican Republic", GT: "Guatemala", HN: "Honduras", SV: "El Salvador",
  CR: "Costa Rica", PA: "Panama", CU: "Cuba", JM: "Jamaica",
  RS: "Serbia", BA: "Bosnia", AL: "Albania", MK: "North Macedonia",
  ME: "Montenegro", XK: "Kosovo", CY: "Cyprus", MT: "Malta",
  IS: "Iceland", LU: "Luxembourg",
  MZ: "Mozambique", AO: "Angola", ZW: "Zimbabwe", ZM: "Zambia",
  BW: "Botswana", NA: "Namibia", MW: "Malawi", RW: "Rwanda",
  BJ: "Benin", TG: "Togo", NE: "Niger", ML: "Mali", BF: "Burkina Faso",
  GA: "Gabon", CG: "Congo", CD: "DR Congo", TD: "Chad", MR: "Mauritania",
  SO: "Somalia", DJ: "Djibouti", ER: "Eritrea", SS: "South Sudan",
  SL: "Sierra Leone", LR: "Liberia", GM: "Gambia", GN: "Guinea",
  SC: "Seychelles", MU: "Mauritius", CV: "Cape Verde",
  JO: "Jordan", LB: "Lebanon", KW: "Kuwait", QA: "Qatar",
  BH: "Bahrain", OM: "Oman", YE: "Yemen", SY: "Syria", PS: "Palestine",
  IR: "Iran",
  BN: "Brunei", TL: "East Timor", MV: "Maldives", BT: "Bhutan",
  FJ: "Fiji", PG: "Papua New Guinea", WS: "Samoa", TO: "Tonga",
  HT: "Haiti", TT: "Trinidad and Tobago", BB: "Barbados", BS: "Bahamas",
  BZ: "Belize", SR: "Suriname", GY: "Guyana", NI: "Nicaragua",
  PR: "Puerto Rico", AW: "Aruba", CW: "Curaçao",
  AD: "Andorra", MC: "Monaco", SM: "San Marino", LI: "Liechtenstein",
  FO: "Faroe Islands", GI: "Gibraltar", GL: "Greenland",
};

export function getCountryName(code: string): string {
  const locale = (typeof localStorage !== "undefined" && localStorage.getItem("app_locale")) || "ru";
  if (locale === "en") return COUNTRY_NAMES_EN[code] || COUNTRY_NAMES[code] || code;
  return COUNTRY_NAMES[code] || code;
}

export interface GeoOption {
  value: string;
  label: string;
  searchAliases?: string[];
  separator?: boolean;
}

const COUNTRY_SEARCH_ALIASES: Record<string, string[]> = {
  US: ["usa", "u.s.a", "united states", "united states of america", "america", "сша", "соединенные штаты", "соединённые штаты", "соединенные штаты америки", "соединённые штаты америки", "америка"],
  GB: ["uk", "u.k", "great britain", "britain", "united kingdom", "england", "великобритания", "британия", "соединенное королевство", "соединённое королевство", "англия"],
};

const geoLabels: Record<string, { ru: string; en: string }> = {
  all: { ru: "Все", en: "All" },
  cis: { ru: "СНГ", en: "CIS" },
  "non-cis": { ru: "Не СНГ", en: "Non-CIS" },
  asia: { ru: "Азия", en: "Asia" },
  europe: { ru: "Европа", en: "Europe" },
  americas: { ru: "Америка", en: "Americas" },
  africa: { ru: "Африка", en: "Africa" },
  "middle-east": { ru: "Ближний Восток", en: "Middle East" },
};

function gl(key: string): string {
  const locale = (typeof localStorage !== "undefined" && localStorage.getItem("app_locale")) || "ru";
  const entry = geoLabels[key];
  if (entry) return locale === "en" ? entry.en : entry.ru;
  return key;
}

export function buildGeoOptions(): GeoOption[] {
  const opts: GeoOption[] = [
    { value: "all", label: gl("all") },
    { value: "cis", label: gl("cis") },
    { value: "non-cis", label: gl("non-cis") },
    { value: "asia", label: gl("asia") },
    { value: "europe", label: gl("europe") },
    { value: "americas", label: gl("americas") },
    { value: "africa", label: gl("africa") },
    { value: "middle-east", label: gl("middle-east") },
    { value: "_sep", label: "", separator: true },
  ];
  const allCountries = [...new Set([
    ...CIS_COUNTRIES, ...ASIA_COUNTRIES, ...EUROPE_COUNTRIES, ...AMERICAS_COUNTRIES,
    ...AFRICA_COUNTRIES, ...MIDDLE_EAST_COUNTRIES, "AU", "NZ", "FJ", "PG",
  ])];
  allCountries.sort((a, b) => getCountryName(a).localeCompare(getCountryName(b)));
  for (const code of allCountries) {
    opts.push({ value: code, label: getCountryName(code), searchAliases: COUNTRY_SEARCH_ALIASES[code] });
  }
  return opts;
}

export const GEO_OPTIONS = buildGeoOptions();

export function getGeoLabel(filter: string): string {
  const entry = geoLabels[filter];
  if (entry) return gl(filter);
  return getCountryName(filter);
}

export function getGeoSearchText(option: GeoOption): string {
  return [option.label, option.value, ...(option.searchAliases || [])].join(" ").toLowerCase();
}
