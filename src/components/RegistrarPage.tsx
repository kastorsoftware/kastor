import { useMemo, useState } from "react";
import { CircleHelp, FolderOpen, Play, RefreshCw, ShieldCheck, Smartphone, WalletCards } from "lucide-react";
import { useI18n } from "@/i18n";

type NameMode = "same" | "file" | "random";

const providers = [
  "Grizzly SMS", "SMS Bower", "SMS-Man", "365SMS", "24SMS7", "Greedy SMS",
  "BRINGSMS", "5SIM", "Vak-SMS", "OnlineSIM", "Custom SMS-Activate",
];

const countries = [
  { id: "russia", ru: "Россия", en: "Russia" },
  { id: "ukraine", ru: "Украина", en: "Ukraine" },
  { id: "kazakhstan", ru: "Казахстан", en: "Kazakhstan" },
  { id: "indonesia", ru: "Индонезия", en: "Indonesia" },
  { id: "philippines", ru: "Филиппины", en: "Philippines" },
  { id: "brazil", ru: "Бразилия", en: "Brazil" },
];

export function RegistrarPage() {
  const { locale } = useI18n();
  const ru = locale === "ru";
  const text = (russian: string, english: string) => (ru ? russian : english);
  const [provider, setProvider] = useState(providers[0]);
  const [apiKey, setApiKey] = useState("");
  const [customEndpoint, setCustomEndpoint] = useState("");
  const [selectedCountries, setSelectedCountries] = useState<string[]>(["russia"]);
  const [countMode, setCountMode] = useState<"count" | "budget">("count");
  const [count, setCount] = useState("10");
  const [setTwoFa, setSetTwoFa] = useState(false);
  const [twoFaPassword, setTwoFaPassword] = useState("");
  const [sendPush, setSendPush] = useState(false);
  const [useAntiSafety, setUseAntiSafety] = useState(false);
  const [antiSafetyKey, setAntiSafetyKey] = useState("");
  const [antiSafetyAid, setAntiSafetyAid] = useState("");
  const [addToPanel, setAddToPanel] = useState(true);
  const [openResults, setOpenResults] = useState(false);
  const [apiPairs, setApiPairs] = useState("");
  const [smsTimeout, setSmsTimeout] = useState("300");
  const [stopOnPriceChange, setStopOnPriceChange] = useState(true);
  const [minPrice, setMinPrice] = useState("");
  const [maxPrice, setMaxPrice] = useState("");
  const [nameMode, setNameMode] = useState<NameMode>("random");
  const [firstName, setFirstName] = useState("");
  const [lastName, setLastName] = useState("");
  const [namesFile, setNamesFile] = useState("");
  const [useSmsProxy, setUseSmsProxy] = useState(false);
  const [smsProxy, setSmsProxy] = useState("");
  const [rejectAppCode, setRejectAppCode] = useState(true);

  const selectedCountryNames = useMemo(
    () => countries.filter((country) => selectedCountries.includes(country.id)).map((country) => ru ? country.ru : country.en),
    [selectedCountries, ru],
  );

  const toggleCountry = (id: string) => {
    setSelectedCountries((current) => current.includes(id)
      ? current.filter((country) => country !== id)
      : [...current, id]);
  };

  const selectNamesFile = async () => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open({ multiple: false, filters: [{ name: "Text", extensions: ["txt"] }] });
    if (typeof path === "string") setNamesFile(path);
  };

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div>
          <p className="text-sm text-muted-foreground">
            {text("Настройки будущей задачи регистрации. Запросы к бэкенду и запуск будут подключены следующим этапом.", "Configure a future registration task. Backend requests and task launch will be connected in the next step.")}
          </p>
        </div>
        <div className="rounded-md border border-border bg-card px-3 py-2 text-xs text-muted-foreground">
          {text("Локальная настройка", "Local configuration")}
        </div>
      </div>

      <Section icon={WalletCards} title={text("SMS-сервис и номера", "SMS provider and numbers")}>
        <div className="grid gap-5 lg:grid-cols-2">
          <Field label={text("SMS-сервис", "SMS provider")}>
            <select value={provider} onChange={(event) => setProvider(event.target.value)} className={selectClass}>
              {providers.map((item) => <option key={item}>{item}</option>)}
            </select>
          </Field>
          <Field label={text("API-ключ", "API key")} hint={text("Хранится только в настройках задачи, пока она открыта.", "Kept only in the task settings while this page is open.")}>
            <input value={apiKey} onChange={(event) => setApiKey(event.target.value)} type="password" autoComplete="off" className={inputClass} placeholder="••••••••••••" />
          </Field>
        </div>
        {provider === "Custom SMS-Activate" && (
          <div className="mt-5 max-w-xl">
            <Field label={text("Адрес handler_api.php", "handler_api.php endpoint")} hint={text("Для сервиса с совместимым протоколом SMS-Activate.", "For a service compatible with the SMS-Activate protocol.")}>
              <input value={customEndpoint} onChange={(event) => setCustomEndpoint(event.target.value)} className={inputClass} placeholder="https://example.com/stubs/handler_api.php" />
            </Field>
          </div>
        )}
        <div className="mt-4 flex flex-wrap gap-2">
          <PendingButton icon={RefreshCw} label={text("Запросить баланс", "Request balance")} />
          <PendingButton icon={RefreshCw} label={text("Загрузить страны и цены Telegram", "Load Telegram countries and prices")} />
        </div>
        <p className="mt-2 text-xs text-muted-foreground">
          {text("Данные будут загружаться с выбранного сервиса после подключения UI к бэкенду.", "Data will be loaded from the selected provider once the UI is connected to the backend.")}
        </p>

        <div className="mt-5 border-t border-border pt-5">
          <label className="text-sm font-medium">{text("Страны", "Countries")}</label>
          <p className="mb-3 text-xs text-muted-foreground">{text("Выберите страны для покупки номеров Telegram.", "Choose countries for Telegram number purchases.")}</p>
          <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-3">
            {countries.map((country) => (
              <CheckRow key={country.id} checked={selectedCountries.includes(country.id)} onChange={() => toggleCountry(country.id)} label={ru ? country.ru : country.en} />
            ))}
          </div>
          <div className="mt-3 text-xs text-muted-foreground">
            {text("Выбрано: ", "Selected: ")}{selectedCountryNames.length ? selectedCountryNames.join(", ") : text("ничего", "none")}
          </div>
        </div>

        <div className="mt-5 grid gap-5 border-t border-border pt-5 lg:grid-cols-2">
          <Field label={text("Количество аккаунтов", "Account quantity")}>
            <div className="flex gap-2">
              <select value={countMode} onChange={(event) => setCountMode(event.target.value as "count" | "budget")} className="h-9 w-44 rounded-md border border-input bg-background px-3 text-sm outline-none focus:border-primary/50">
                <option value="count">{text("Точное число", "Exact count")}</option>
                <option value="budget">{text("На весь баланс", "Use full balance")}</option>
              </select>
              {countMode === "count" && <input value={count} onChange={(event) => setCount(event.target.value)} min="1" type="number" className={inputClass} />}
            </div>
          </Field>
          <Field label={text("Таймаут ожидания SMS, сек.", "SMS wait timeout, sec.")}>
            <input value={smsTimeout} onChange={(event) => setSmsTimeout(event.target.value)} min="30" type="number" className={inputClass} />
          </Field>
        </div>
      </Section>

      <div className="grid gap-6 xl:grid-cols-2">
        <Section icon={ShieldCheck} title={text("Регистрация и результат", "Registration and result")}>
          <div className="space-y-4">
            <ToggleRow checked={setTwoFa} onChange={setSetTwoFa} label={text("Установить 2FA после регистрации", "Set 2FA after registration")} />
            {setTwoFa && <Field label={text("Пароль 2FA", "2FA password")}><input value={twoFaPassword} onChange={(event) => setTwoFaPassword(event.target.value)} type="password" autoComplete="new-password" className={inputClass} /></Field>}
            <ToggleRow checked={sendPush} onChange={setSendPush} label={text("Зарегистрировать Push-токен после регистрации", "Register Push token after registration")} hint={text("Токен нужен для будущих уведомлений аккаунта, не для получения кода.", "The token enables future account notifications, not code delivery.")} />
            <ToggleRow checked={addToPanel} onChange={setAddToPanel} label={text("Добавлять аккаунты в панель", "Add accounts to the panel")} />
            <ToggleRow checked={openResults} onChange={setOpenResults} label={text("Открыть папку с итоговыми TData после завершения", "Open the results TData folder when complete")} />
            <ToggleRow checked={rejectAppCode} onChange={setRejectAppCode} label={text("Отказаться от номера, если код ушёл в приложение", "Reject number if the code is sent to the app")} />
          </div>
        </Section>

        <Section icon={Smartphone} title={text("AntiSafety", "AntiSafety")}>
          <ToggleRow checked={useAntiSafety} onChange={setUseAntiSafety} label={text("Использовать AntiSafety", "Use AntiSafety")} hint={text("Опционально: передача reCAPTCHA-токена при соответствующей ошибке Telegram и Push после регистрации.", "Optional: pass a reCAPTCHA token after the matching Telegram error and register Push after registration.")} />
          {useAntiSafety && (
            <div className="mt-4 grid gap-4">
              <Field label={text("API-ключ AntiSafety", "AntiSafety API key")}><input value={antiSafetyKey} onChange={(event) => setAntiSafetyKey(event.target.value)} type="password" autoComplete="off" className={inputClass} /></Field>
              <Field label="AID"><input value={antiSafetyAid} onChange={(event) => setAntiSafetyAid(event.target.value)} className={inputClass} placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" /></Field>
              <p className="rounded-md border border-border bg-background/40 px-3 py-2 text-xs text-muted-foreground">
                {text("reCAPTCHA запрашивается только после ответа Telegram RECAPTCHA_CHECK. Push будет зарегистрирован только при включённой опции выше.", "reCAPTCHA is requested only after Telegram returns RECAPTCHA_CHECK. Push is registered only when the option above is enabled.")}
              </p>
            </div>
          )}
        </Section>
      </div>

      <Section icon={Smartphone} title={text("Профили приложения и имена", "Application profiles and names")}>
        <div className="grid gap-6 xl:grid-cols-2">
          <Field label={text("Пары API ID : API hash", "API ID : API hash pairs")} hint={text("Одна пара на строку. Для каждой регистрации будет выбран профиль из списка.", "One pair per line. A profile will be selected from this list for each registration.")}>
            <textarea value={apiPairs} onChange={(event) => setApiPairs(event.target.value)} rows={6} className={textareaClass} placeholder={"12345:0123456789abcdef0123456789abcdef"} />
          </Field>
          <div>
            <label className="text-sm font-medium">{text("Имена", "Names")}</label>
            <div className="mt-2 flex flex-wrap gap-2">
              <Choice active={nameMode === "same"} onClick={() => setNameMode("same")} label={text("Одинаковые", "Same")}/>
              <Choice active={nameMode === "file"} onClick={() => setNameMode("file")} label={text("Из файла", "From file")}/>
              <Choice active={nameMode === "random"} onClick={() => setNameMode("random")} label={text("Случайные символы", "Random characters")}/>
            </div>
            {nameMode === "same" && <div className="mt-3 grid grid-cols-2 gap-3"><input value={firstName} onChange={(event) => setFirstName(event.target.value)} className={inputClass} placeholder={text("Имя", "First name")} /><input value={lastName} onChange={(event) => setLastName(event.target.value)} className={inputClass} placeholder={text("Фамилия", "Last name")} /></div>}
            {nameMode === "file" && <div className="mt-3 flex items-center gap-2"><button type="button" onClick={selectNamesFile} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50"><FolderOpen className="h-4 w-4" />{text("Выбрать файл", "Choose file")}</button><span className="truncate text-xs text-muted-foreground">{namesFile || text("Файл не выбран", "No file selected")}</span></div>}
            {nameMode === "random" && <p className="mt-3 text-xs text-muted-foreground">{text("Имя и фамилия будут сгенерированы из случайных символов.", "First and last names will be generated from random characters.")}</p>}
          </div>
        </div>
      </Section>

      <div className="grid gap-6 xl:grid-cols-2">
        <Section icon={CircleHelp} title={text("Цена и остановка", "Pricing and stop conditions")}>
          <ToggleRow checked={stopOnPriceChange} onChange={setStopOnPriceChange} label={text("Остановиться при изменении цены", "Stop when the price changes")} />
          <div className="mt-4 grid grid-cols-2 gap-4">
            <Field label={text("Минимальная цена", "Minimum price")}><input value={minPrice} onChange={(event) => setMinPrice(event.target.value)} type="number" min="0" step="0.01" className={inputClass} placeholder="0.00" /></Field>
            <Field label={text("Максимальная цена", "Maximum price")}><input value={maxPrice} onChange={(event) => setMaxPrice(event.target.value)} type="number" min="0" step="0.01" className={inputClass} placeholder="0.00" /></Field>
          </div>
        </Section>

        <Section icon={Smartphone} title={text("Прокси для SMS-сервиса", "Proxy for SMS provider")}>
          <ToggleRow checked={useSmsProxy} onChange={setUseSmsProxy} label={text("Проксировать запросы к SMS-сервису", "Proxy requests to the SMS provider")} />
          {useSmsProxy && <Field label={text("Прокси", "Proxy")} hint="socks5://user:password@host:port"><textarea value={smsProxy} onChange={(event) => setSmsProxy(event.target.value)} rows={3} className={`${textareaClass} mt-3`} placeholder="socks5://127.0.0.1:1080" /></Field>}
        </Section>
      </div>

      <div className="flex flex-wrap items-center justify-between gap-4 rounded-xl border border-border bg-card px-5 py-4">
        <div className="text-sm">
          <div className="font-medium">{text("Задача ещё не подключена к бэкенду", "Task is not connected to the backend yet")}</div>
          <div className="mt-0.5 text-xs text-muted-foreground">{text("Все параметры формы уже готовы для передачи в команду регистрации.", "All form parameters are ready to be passed to the registration command.")}</div>
        </div>
        <button disabled className="flex items-center gap-2 rounded-md border border-primary/50 bg-primary/10 px-4 py-2 text-sm font-medium text-primary opacity-50" title={text("Будет включено при подключении к бэкенду", "Will be enabled when connected to the backend")}>
          <Play className="h-4 w-4" />{text("Запустить регистрацию", "Start registration")}
        </button>
      </div>
    </div>
  );
}

const inputClass = "h-9 w-full rounded-md border border-input bg-background px-3 text-sm outline-none transition focus:border-primary/50";
const selectClass = "h-9 w-full rounded-md border border-input bg-background px-3 text-sm outline-none transition focus:border-primary/50";
const textareaClass = "w-full rounded-md border border-input bg-background px-3 py-2 text-sm outline-none transition focus:border-primary/50 resize-y";

function Section({ icon: Icon, title, children }: { icon: typeof WalletCards; title: string; children: React.ReactNode }) {
  return <section className="rounded-xl border border-border bg-card p-6"><div className="mb-5 flex items-center gap-2"><Icon className="h-4 w-4 text-primary" /><h2 className="text-base font-semibold">{title}</h2></div>{children}</section>;
}

function Field({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return <div><label className="text-sm font-medium">{label}</label>{hint && <p className="mb-1.5 mt-0.5 text-xs text-muted-foreground">{hint}</p>}<div className={hint ? "" : "mt-1.5"}>{children}</div></div>;
}

function ToggleRow({ checked, onChange, label, hint }: { checked: boolean; onChange: (checked: boolean) => void; label: string; hint?: string }) {
  return <label className="flex cursor-pointer items-start gap-3"><input type="checkbox" checked={checked} onChange={(event) => onChange(event.target.checked)} className="mt-0.5 h-4 w-4 rounded border-border accent-primary" /><span><span className="text-sm font-medium">{label}</span>{hint && <span className="mt-0.5 block text-xs text-muted-foreground">{hint}</span>}</span></label>;
}

function CheckRow({ checked, onChange, label }: { checked: boolean; onChange: () => void; label: string }) {
  return <label className="flex cursor-pointer items-center gap-2 rounded-md border border-border bg-background/40 px-3 py-2 text-sm hover:border-primary/40"><input type="checkbox" checked={checked} onChange={onChange} className="h-4 w-4 rounded border-border accent-primary" />{label}</label>;
}

function Choice({ active, onClick, label }: { active: boolean; onClick: () => void; label: string }) {
  return <button type="button" onClick={onClick} className={`rounded-md border px-3 py-1.5 text-sm transition ${active ? "border-primary/50 bg-primary/10 text-primary" : "border-border bg-background text-muted-foreground hover:border-primary/40"}`}>{label}</button>;
}

function PendingButton({ icon: Icon, label }: { icon: typeof RefreshCw; label: string }) {
  return <button type="button" className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium text-muted-foreground hover:border-primary/50 hover:text-foreground"><Icon className="h-4 w-4" />{label}</button>;
}
