import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { CircleHelp, FolderOpen, Play, RefreshCw, ShieldCheck, Smartphone, WalletCards } from "lucide-react";
import { useT } from "@/i18n";

type NameMode = "same" | "file" | "random";

type Provider = { kind: string; label: string; requires_endpoint: boolean };
type Country = { id: string; name: string };
type Price = { country: Country; price: number; available: number | null; operator: string | null };

export function RegistrarPage() {
  const t = useT();
  const [providers, setProviders] = useState<Provider[]>([]);
  const [provider, setProvider] = useState("");
  const [countries, setCountries] = useState<Country[]>([]);
  const [prices, setPrices] = useState<Price[]>([]);
  const [balance, setBalance] = useState<number | null>(null);
  const [pending, setPending] = useState(false);
  const [requestError, setRequestError] = useState("");
  const requestGeneration = useRef(0);
  const [apiKey, setApiKey] = useState("");
  const [customEndpoint, setCustomEndpoint] = useState("");
  const [selectedCountries, setSelectedCountries] = useState<string[]>([]);
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

  useEffect(() => {
    let disposed = false;
    invoke<Provider[]>("registrar_sms_providers").then((items) => {
      if (disposed) return;
      setProviders(items);
      setProvider(items[0]?.kind ?? "");
    }).catch(() => {
      if (!disposed) setRequestError("registrar.providersError");
    });
    return () => { disposed = true; requestGeneration.current++; };
  }, []);

  useEffect(() => {
    requestGeneration.current++;
    setCountries([]);
    setPrices([]);
    setBalance(null);
    setSelectedCountries([]);
    setRequestError("");
    setPending(false);
  }, [provider, apiKey, customEndpoint]);

  const requestProviderData = async (catalog: boolean) => {
    const generation = ++requestGeneration.current;
    setPending(true);
    setRequestError("");
    const config = { kind: provider, api_key: apiKey, endpoint: customEndpoint || null };
    try {
      if (catalog) {
        const [loadedCountries, loadedPrices] = await Promise.all([
          invoke<Country[]>("registrar_sms_countries", { config }),
          invoke<Price[]>("registrar_sms_prices", { config, service: "telegram", country: null }),
        ]);
        if (generation !== requestGeneration.current) return;
        const telegramCountries = new Set(loadedPrices.map((price) => price.country.id));
        setCountries(loadedCountries.filter((country) => telegramCountries.has(country.id)));
        setPrices(loadedPrices);
        setSelectedCountries([]);
      } else {
        const value = await invoke<number>("registrar_sms_balance", { config });
        if (generation === requestGeneration.current) setBalance(value);
      }
    } catch {
      if (generation === requestGeneration.current) setRequestError(catalog ? "registrar.catalogError" : "registrar.balanceError");
    } finally {
      if (generation === requestGeneration.current) setPending(false);
    }
  };

  const selectedCountryNames = useMemo(
    () => countries.filter((country) => selectedCountries.includes(country.id)).map((country) => country.name),
    [selectedCountries, countries],
  );

  const pricesByCountry = useMemo(() => {
    const grouped = new Map<string, Price[]>();
    for (const price of prices) {
      const group = grouped.get(price.country.id) ?? [];
      group.push(price);
      grouped.set(price.country.id, group);
    }
    return grouped;
  }, [prices]);

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
            {t("registrar.description")}
          </p>
        </div>
        <div className="rounded-md border border-border bg-card px-3 py-2 text-xs text-muted-foreground">
          {t("registrar.status")}
        </div>
      </div>

      <Section icon={WalletCards} title={t("registrar.smsSection")}>
        <div className="grid gap-5 lg:grid-cols-2">
          <Field label={t("registrar.provider")}>
            <select value={provider} onChange={(event) => setProvider(event.target.value)} className={selectClass}>
              {providers.map((item) => <option key={item.kind} value={item.kind}>{item.label}</option>)}
            </select>
          </Field>
          <Field label={t("registrar.apiKey")} hint={t("registrar.keyHint")}>
            <input value={apiKey} onChange={(event) => setApiKey(event.target.value)} type="password" autoComplete="off" className={inputClass} placeholder="••••••••••••" />
          </Field>
        </div>
        {providers.find((item) => item.kind === provider)?.requires_endpoint && (
          <div className="mt-5 max-w-xl">
            <Field label={t("registrar.endpoint")} hint={t("registrar.endpointHint")}>
              <input value={customEndpoint} onChange={(event) => setCustomEndpoint(event.target.value)} className={inputClass} placeholder="https://example.com/stubs/handler_api.php" />
            </Field>
          </div>
        )}
        <div className="mt-4 flex flex-wrap gap-2">
          <ProviderButton disabled={pending || !apiKey.trim() || !provider} onClick={() => void requestProviderData(false)} label={t("registrar.balance")} />
          <ProviderButton disabled={pending || !apiKey.trim() || !provider} onClick={() => void requestProviderData(true)} label={t("registrar.loadCountries")} />
        </div>
        {balance !== null && <p className="mt-2 text-sm">{t("registrar.balanceValue")}: {balance}</p>}
        {pending && <p role="status" className="mt-2 text-xs text-muted-foreground">{t("registrar.loading")}</p>}
        {requestError && <p role="alert" className="mt-2 text-sm text-destructive">{t(requestError)}</p>}
        <p className="mt-2 text-xs text-muted-foreground">
          {t("registrar.catalogHint")}
        </p>

        <div className="mt-5 border-t border-border pt-5">
          <label className="text-sm font-medium">{t("registrar.countries")}</label>
          <p className="mb-3 text-xs text-muted-foreground">{t("registrar.countriesHint")}</p>
          <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-3">
            {countries.map((country) => (
              <div key={country.id}>
                <CheckRow checked={selectedCountries.includes(country.id)} onChange={() => toggleCountry(country.id)} label={country.name} />
                {(pricesByCountry.get(country.id) ?? []).map((price, index) => (
                  <p key={index} className="mt-1 text-xs text-muted-foreground">
                    {price.operator ? `${price.operator}: ` : ""}{price.price} · {t("registrar.available")}: {price.available ?? t("registrar.unknown")}
                  </p>
                ))}
              </div>
            ))}
          </div>
          <div className="mt-3 text-xs text-muted-foreground">
            {t("registrar.selected")}{selectedCountryNames.length ? selectedCountryNames.join(", ") : t("registrar.none")}
          </div>
        </div>

        <div className="mt-5 grid gap-5 border-t border-border pt-5 lg:grid-cols-2">
          <Field label={t("registrar.quantity")}>
            <div className="flex gap-2">
              <select value={countMode} onChange={(event) => setCountMode(event.target.value as "count" | "budget")} className="h-9 w-44 rounded-md border border-input bg-background px-3 text-sm outline-none focus:border-primary/50">
                <option value="count">{t("registrar.exactCount")}</option>
                <option value="budget">{t("registrar.fullBalance")}</option>
              </select>
              {countMode === "count" && <input value={count} onChange={(event) => setCount(event.target.value)} min="1" type="number" className={inputClass} />}
            </div>
          </Field>
          <Field label={t("registrar.smsTimeout")}>
            <input value={smsTimeout} onChange={(event) => setSmsTimeout(event.target.value)} min="30" type="number" className={inputClass} />
          </Field>
        </div>
      </Section>

      <div className="grid gap-6 xl:grid-cols-2">
        <Section icon={ShieldCheck} title={t("registrar.resultSection")}>
          <div className="space-y-4">
            <ToggleRow checked={setTwoFa} onChange={setSetTwoFa} label={t("registrar.setTwoFa")} />
            {setTwoFa && <Field label={t("registrar.twoFaPassword")}><input value={twoFaPassword} onChange={(event) => setTwoFaPassword(event.target.value)} type="password" autoComplete="new-password" className={inputClass} /></Field>}
            <ToggleRow checked={sendPush} onChange={setSendPush} label={t("registrar.sendPush")} hint={t("registrar.pushHint")} />
            <ToggleRow checked={addToPanel} onChange={setAddToPanel} label={t("registrar.addToPanel")} />
            <ToggleRow checked={openResults} onChange={setOpenResults} label={t("registrar.openResults")} />
            <ToggleRow checked={rejectAppCode} onChange={setRejectAppCode} label={t("registrar.rejectAppCode")} />
          </div>
        </Section>

        <Section icon={Smartphone} title={t("registrar.antiSafety")}>
          <ToggleRow checked={useAntiSafety} onChange={setUseAntiSafety} label={t("registrar.useAntiSafety")} hint={t("registrar.antiSafetyHint")} />
          {useAntiSafety && (
            <div className="mt-4 grid gap-4">
              <Field label={t("registrar.antiSafetyKey")}><input value={antiSafetyKey} onChange={(event) => setAntiSafetyKey(event.target.value)} type="password" autoComplete="off" className={inputClass} /></Field>
              <Field label="AID"><input value={antiSafetyAid} onChange={(event) => setAntiSafetyAid(event.target.value)} className={inputClass} placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" /></Field>
              <p className="rounded-md border border-border bg-background/40 px-3 py-2 text-xs text-muted-foreground">
                {t("registrar.challengeHint")}
              </p>
            </div>
          )}
        </Section>
      </div>

      <Section icon={Smartphone} title={t("registrar.profileSection")}>
        <div className="grid gap-6 xl:grid-cols-2">
          <Field label={t("registrar.apiPairs")} hint={t("registrar.apiPairsHint")}>
            <textarea value={apiPairs} onChange={(event) => setApiPairs(event.target.value)} rows={6} className={textareaClass} placeholder={"12345:0123456789abcdef0123456789abcdef"} />
          </Field>
          <div>
            <label className="text-sm font-medium">{t("registrar.names")}</label>
            <div className="mt-2 flex flex-wrap gap-2">
              <Choice active={nameMode === "same"} onClick={() => setNameMode("same")} label={t("registrar.sameNames")}/>
              <Choice active={nameMode === "file"} onClick={() => setNameMode("file")} label={t("registrar.fileNames")}/>
              <Choice active={nameMode === "random"} onClick={() => setNameMode("random")} label={t("registrar.randomNames")}/>
            </div>
            {nameMode === "same" && <div className="mt-3 grid grid-cols-2 gap-3"><input value={firstName} onChange={(event) => setFirstName(event.target.value)} className={inputClass} placeholder={t("registrar.firstName")} /><input value={lastName} onChange={(event) => setLastName(event.target.value)} className={inputClass} placeholder={t("registrar.lastName")} /></div>}
            {nameMode === "file" && <div className="mt-3 flex items-center gap-2"><button type="button" onClick={selectNamesFile} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50"><FolderOpen className="h-4 w-4" />{t("registrar.chooseFile")}</button><span className="truncate text-xs text-muted-foreground">{namesFile || t("registrar.noFile")}</span></div>}
            {nameMode === "random" && <p className="mt-3 text-xs text-muted-foreground">{t("registrar.randomNamesHint")}</p>}
          </div>
        </div>
      </Section>

      <div className="grid gap-6 xl:grid-cols-2">
        <Section icon={CircleHelp} title={t("registrar.priceSection")}>
          <ToggleRow checked={stopOnPriceChange} onChange={setStopOnPriceChange} label={t("registrar.stopOnPriceChange")} />
          <div className="mt-4 grid grid-cols-2 gap-4">
            <Field label={t("registrar.minPrice")}><input value={minPrice} onChange={(event) => setMinPrice(event.target.value)} type="number" min="0" step="0.01" className={inputClass} placeholder="0.00" /></Field>
            <Field label={t("registrar.maxPrice")}><input value={maxPrice} onChange={(event) => setMaxPrice(event.target.value)} type="number" min="0" step="0.01" className={inputClass} placeholder="0.00" /></Field>
          </div>
        </Section>

        <Section icon={Smartphone} title={t("registrar.proxySection")}>
          <ToggleRow checked={useSmsProxy} onChange={setUseSmsProxy} label={t("registrar.useSmsProxy")} />
          {useSmsProxy && <Field label={t("registrar.proxy")} hint="socks5://user:password@host:port"><textarea value={smsProxy} onChange={(event) => setSmsProxy(event.target.value)} rows={3} className={`${textareaClass} mt-3`} placeholder="socks5://127.0.0.1:1080" /></Field>}
        </Section>
      </div>

      <div className="flex flex-wrap items-center justify-between gap-4 rounded-xl border border-border bg-card px-5 py-4">
        <div className="text-sm">
          <div className="font-medium">{t("registrar.taskStatus")}</div>
          <div className="mt-0.5 text-xs text-muted-foreground">{t("registrar.taskHint")}</div>
        </div>
        <button disabled className="flex items-center gap-2 rounded-md border border-primary/50 bg-primary/10 px-4 py-2 text-sm font-medium text-primary opacity-50" title={t("registrar.startHint")}>
          <Play className="h-4 w-4" />{t("registrar.start")}
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

function ProviderButton({ label, disabled, onClick }: { label: string; disabled: boolean; onClick: () => void }) {
  return <button type="button" disabled={disabled} onClick={onClick} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium text-muted-foreground hover:border-primary/50 hover:text-foreground disabled:opacity-50"><RefreshCw className="h-4 w-4" />{label}</button>;
}
