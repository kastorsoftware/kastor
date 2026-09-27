import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { CircleHelp, FolderOpen, Play, Square, RefreshCw, ShieldCheck, Smartphone, WalletCards } from "lucide-react";
import { useT } from "@/i18n";
import { countryCallingCode, sortRegistrarCountries } from "@/lib/registrarCountries";
import { optionalPrice } from "@/lib/registrarSettings";

type NameMode = "same" | "file" | "random";

type Provider = { kind: string; label: string; requires_endpoint: boolean };
type Country = { id: string; name: string };
type Price = { country: Country; price: number; available: number | null; operator: string | null };
type Progress = { task_id: string; total: number | null; processed: number; registered: number; failed: number; finished: boolean; reason: string; results_directory: string };
type Identity = { first_name: string; last_name: string };

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
  const [count, setCount] = useState("1");
  const [setTwoFa, setSetTwoFa] = useState(false);
  const [twoFaPassword, setTwoFaPassword] = useState("");
  const [antiSafetyKey, setAntiSafetyKey] = useState("");
  const [antiSafetyAid, setAntiSafetyAid] = useState("");
  const [addToPanel, setAddToPanel] = useState(true);
  const [openResults, setOpenResults] = useState(false);
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
  const [telegramProxies, setTelegramProxies] = useState("");
  const [deviceModel, setDeviceModel] = useState("Desktop");
  const [systemVersion, setSystemVersion] = useState("Windows");
  const [appVersion, setAppVersion] = useState("");
  const [concurrency, setConcurrency] = useState("1");
  const [pollInterval, setPollInterval] = useState("5");
  const [progress, setProgress] = useState<Progress | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [launching, setLaunching] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [listenersReady, setListenersReady] = useState(false);
  const taskId = useRef<string | null>(sessionStorage.getItem("registrar_task_id"));
  const lastEvent = useRef<Progress | null>(null);
  const running = progress !== null && !progress.finished;

  useEffect(() => {
    let disposed = false;
    const cleanups: UnlistenFn[] = [];
    const accept = (value: Progress) => {
      if (disposed) return;
      lastEvent.current = value;
      if (value.task_id === taskId.current) {
        setProgress((current) => current?.task_id === value.task_id
          && (current.processed > value.processed || current.finished)
          ? current : { ...value, total: value.total ?? current?.total ?? null });
        if (value.finished) setStopping(false);
      }
    };
    const setup = async () => {
      for (const subscribe of [
        () => listen<Progress>("registrar-progress", ({ payload }) => accept(payload)),
        () => listen<string>("registrar-log", ({ payload }) => {
          if (!disposed && taskId.current && payload.startsWith(`[${taskId.current}]`)) {
            setLogs((current) => [...current.slice(-499), payload]);
          }
        }),
      ]) {
        const unlisten = await subscribe();
        if (disposed) unlisten(); else cleanups.push(unlisten);
      }
      if (taskId.current) {
        try { accept(await invoke<Progress>("registrar_status", { taskId: taskId.current })); }
        catch {
          if (!disposed) {
            taskId.current = null;
            sessionStorage.removeItem("registrar_task_id");
          }
        }
      }
      if (!disposed) setListenersReady(true);
    };
    void setup().catch(() => { if (!disposed) setRequestError("registrar.eventsError"); });
    void invoke<string>("get_version").then((version) => { if (!disposed) setAppVersion(version); })
      .catch(() => { if (!disposed) setRequestError("registrar.versionUnavailable"); });
    return () => { disposed = true; cleanups.forEach((cleanup) => cleanup()); };
  }, []);

  useEffect(() => {
    if (!running || !progress) return;
    let disposed = false;
    const id = progress.task_id;
    let pending = false;
    const timer = window.setInterval(() => {
      if (pending) return;
      pending = true;
      void invoke<Progress>("registrar_status", { taskId: id }).then((value) => {
        if (!disposed && taskId.current === id) {
          setProgress((current) => current && (current.processed > value.processed || current.finished)
            ? current : { ...value, total: value.total ?? current?.total ?? null });
        }
      }).catch(() => { if (!disposed) setRequestError("registrar.statusFailed"); })
        .finally(() => { pending = false; });
    }, 3000);
    return () => { disposed = true; window.clearInterval(timer); };
  }, [running, progress?.task_id]);

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
  }, [provider, apiKey, customEndpoint, useSmsProxy, smsProxy]);

  const requestProviderData = async (catalog: boolean) => {
    const generation = ++requestGeneration.current;
    setPending(true);
    setRequestError("");
    const config = { kind: provider, api_key: apiKey, endpoint: customEndpoint || null, http_proxy: useSmsProxy ? smsProxy.trim() : null };
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

  const sortedCountries = useMemo(() => sortRegistrarCountries(countries, prices), [countries, prices]);

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

  const startTask = async () => {
    setLaunching(true);
    setRequestError("");
    try {
      const pairs = [{ api_id: 4, api_hash: "014b35b6184100b085b0d0572f9b5103" }];
      const minimum = optionalPrice(minPrice);
      const maximum = optionalPrice(maxPrice);
      if (minimum !== null && maximum !== null && minimum > maximum) throw new Error("registrar.invalidPrice");
      const quantity = Number(count);
      if (countMode === "count" && (!Number.isInteger(quantity) || quantity < 1 || quantity > 100000)) throw new Error("registrar.invalidCount");
      if (!selectedCountries.length) throw new Error("registrar.selectCountries");
      if (setTwoFa && !twoFaPassword) throw new Error("registrar.passwordRequired");
      if (!antiSafetyKey.trim() || !antiSafetyAid.trim()) throw new Error("registrar.pushCredentialsRequired");
      if (useSmsProxy && !smsProxy.trim()) throw new Error("registrar.proxyRequired");
      const identities: Identity[] = nameMode === "file"
        ? await invoke<Identity[]>("registrar_read_names", { path: namesFile })
        : [{ first_name: nameMode === "random" ? crypto.randomUUID().replace(/-/g, "").slice(0, 10) : firstName.trim(), last_name: lastName.trim() }];
      if (!identities.length || identities.some((identity) => !identity.first_name)) throw new Error("registrar.namesRequired");
      const proxies = await invoke<unknown[]>("registrar_parse_proxies", { proxies: telegramProxies });
      const attempts = Array.from({ length: Math.max(pairs.length, identities.length, proxies.length, 1) }, (_, index) => ({
        profile: {
          ...pairs[index % pairs.length], device_model: deviceModel, system_version: systemVersion,
          app_version: appVersion, system_lang_code: navigator.language, lang_code: navigator.language.split("-")[0],
          timezone_offset_seconds: -new Date().getTimezoneOffset() * 60,
        },
        identity: identities[index % identities.length], proxy: proxies.length ? proxies[index % proxies.length] : null,
      }));
      const expectedPrices: Record<string, number> = {};
      for (const price of prices) {
        if (!selectedCountries.includes(price.country.id) || price.available === 0 || !Number.isFinite(price.price)
          || price.price < (minimum ?? 0) || price.price > (maximum ?? Infinity)) continue;
        expectedPrices[price.country.id] = Math.min(expectedPrices[price.country.id] ?? Infinity, price.price);
      }
      if (stopOnPriceChange && selectedCountries.some((country) => expectedPrices[country] === undefined)) {
        throw new Error("registrar.noEligiblePrices");
      }
      setLogs([]);
      const id = await invoke<string>("registrar_start", { config: {
        sms_provider: { kind: provider, api_key: apiKey, endpoint: customEndpoint || null, http_proxy: useSmsProxy ? smsProxy.trim() : null },
        number_request: { country: selectedCountries[0], service: "telegram", operator: null, max_price: maximum },
        attempts, attempt_count: countMode === "count" ? quantity : null, random_names: nameMode === "random",
        countries: selectedCountries, expected_prices: expectedPrices, use_full_balance: countMode === "budget",
        min_price: minimum, stop_on_price_change: stopOnPriceChange, reject_app_code: rejectAppCode,
        two_fa_password: setTwoFa ? twoFaPassword : null, send_push: true,
        antisafety: { api_key: antiSafetyKey.trim(), aid: antiSafetyAid.trim() },
        add_to_panel: addToPanel, open_results: openResults, concurrency: Number(concurrency),
        poll_interval_secs: Number(pollInterval), code_timeout_secs: Number(smsTimeout),
      } });
      taskId.current = id;
      sessionStorage.setItem("registrar_task_id", id);
      setProgress(lastEvent.current?.task_id === id ? lastEvent.current : {
        task_id: id, total: countMode === "count" ? quantity : null, processed: 0, registered: 0,
        failed: 0, finished: false, reason: "running", results_directory: "",
      });
    } catch (error) {
      const key = error instanceof Error && error.message.startsWith("registrar.") ? error.message : "registrar.startFailed";
      setRequestError(key);
    } finally { setLaunching(false); }
  };

  const stopTask = async () => {
    if (!taskId.current) return;
    setStopping(true);
    try { await invoke("registrar_stop", { taskId: taskId.current }); }
    catch { setStopping(false); setRequestError("registrar.stopFailed"); }
  };

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div>
          <p className="text-sm text-muted-foreground">
            {t("registrar.description")}
          </p>
        </div>
      </div>

      <fieldset disabled={running || launching} className="space-y-6 disabled:opacity-70">
      <Section icon={WalletCards} title={t("registrar.smsSection")}>
        <div className="grid gap-5 lg:grid-cols-2">
          <Field label={t("registrar.provider")}>
            <select value={provider} onChange={(event) => setProvider(event.target.value)} className={selectClass}>
              {providers.map((item) => <option key={item.kind} value={item.kind}>{item.label}</option>)}
            </select>
          </Field>
          <Field label={t("registrar.apiKey")}>
            <input value={apiKey} onChange={(event) => setApiKey(event.target.value)} type="password" autoComplete="off" className={inputClass} placeholder="••••••••••••" />
          </Field>
        </div>
        <p className="mt-2 text-xs text-muted-foreground">{t("registrar.keyHint")}</p>
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
            {sortedCountries.map((country) => (
              <div key={country.id}>
                <CheckRow checked={selectedCountries.includes(country.id)} onChange={() => toggleCountry(country.id)} label={`${country.name} (${countryCallingCode(country) ?? t("registrar.unknownCallingCode")})`} />
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
            {countMode === "budget" && <p className="mt-2 text-xs text-muted-foreground">{t("registrar.budgetHint")}</p>}
            {countMode === "count" && <p className="mt-2 text-xs text-muted-foreground">{t("registrar.countHint")}</p>}
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
            <ToggleRow checked={addToPanel} onChange={setAddToPanel} label={t("registrar.addToPanel")} />
            <ToggleRow checked={openResults} onChange={setOpenResults} label={t("registrar.openResults")} />
            <ToggleRow checked={rejectAppCode} onChange={setRejectAppCode} label={t("registrar.rejectAppCode")} />
          </div>
        </Section>

        <Section icon={Smartphone} title={t("registrar.antiSafety")}>
          <p className="text-xs text-muted-foreground">{t("registrar.antiSafetyHint")}</p>
          <div className="mt-4 grid gap-4">
            <Field label={t("registrar.antiSafetyKey")}><input value={antiSafetyKey} onChange={(event) => setAntiSafetyKey(event.target.value)} type="password" autoComplete="off" className={inputClass} /></Field>
            <Field label="AID"><input value={antiSafetyAid} onChange={(event) => setAntiSafetyAid(event.target.value)} className={inputClass} placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" /></Field>
            <p className="rounded-md border border-border bg-background/40 px-3 py-2 text-xs text-muted-foreground">
              {t("registrar.challengeHint")}
            </p>
          </div>
        </Section>
      </div>

      <Section icon={Smartphone} title={t("registrar.profileSection")}>
        <div className="grid gap-6 xl:grid-cols-2">
          <div>
            <label className="text-sm font-medium">{t("registrar.names")}</label>
            <div className="mt-2 flex flex-wrap gap-2">
              <Choice active={nameMode === "same"} onClick={() => setNameMode("same")} label={t("registrar.sameNames")}/>
              <Choice active={nameMode === "file"} onClick={() => setNameMode("file")} label={t("registrar.fileNames")}/>
              <Choice active={nameMode === "random"} onClick={() => setNameMode("random")} label={t("registrar.randomNames")}/>
            </div>
            {nameMode === "same" && <div className="mt-3 grid grid-cols-2 gap-3"><input value={firstName} onChange={(event) => setFirstName(event.target.value)} className={inputClass} placeholder={t("registrar.firstName")} /><input value={lastName} onChange={(event) => setLastName(event.target.value)} className={inputClass} placeholder={t("registrar.lastName")} /></div>}
            {nameMode === "file" && <><div className="mt-3 flex items-center gap-2"><button type="button" onClick={selectNamesFile} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50"><FolderOpen className="h-4 w-4" />{t("registrar.chooseFile")}</button><span className="truncate text-xs text-muted-foreground">{namesFile || t("registrar.noFile")}</span></div><p className="mt-2 text-xs text-muted-foreground">{t("registrar.namesFileHint")}</p></>}
            {nameMode === "random" && <p className="mt-3 text-xs text-muted-foreground">{t("registrar.randomNamesHint")}</p>}
          </div>
        </div>
        <div className="mt-5 grid gap-4 md:grid-cols-3">
          <Field label={t("registrar.deviceModel")}><input value={deviceModel} onChange={(event) => setDeviceModel(event.target.value)} className={inputClass}/></Field>
          <Field label={t("registrar.systemVersion")}><input value={systemVersion} onChange={(event) => setSystemVersion(event.target.value)} className={inputClass}/></Field>
          <Field label={t("registrar.appVersion")}><input value={appVersion} onChange={(event) => setAppVersion(event.target.value)} className={inputClass}/></Field>
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
          <div className="mt-4"><Field label={t("registrar.telegramProxies")} hint={t("registrar.telegramProxiesHint")}><textarea value={telegramProxies} onChange={(event) => setTelegramProxies(event.target.value)} rows={3} className={textareaClass}/></Field></div>
        </Section>
      </div>
      <Section icon={CircleHelp} title={t("registrar.execution")}>
        <div className="grid gap-4 md:grid-cols-2">
          <Field label={t("common.threads")} hint={t("registrar.concurrencyHint")}><input type="number" min="1" max="20" value={concurrency} onChange={(event) => setConcurrency(event.target.value)} className={inputClass}/></Field>
          <Field label={t("registrar.pollInterval")}><input type="number" min="2" max="60" value={pollInterval} onChange={(event) => setPollInterval(event.target.value)} className={inputClass}/></Field>
        </div>
      </Section>
      </fieldset>

      <div className="flex flex-wrap items-center justify-between gap-4 rounded-xl border border-border bg-card px-5 py-4">
        <div className="text-sm">
          <div className="font-medium">{t(progress ? `registrar.reasons.${progress.reason}` : "registrar.ready")}</div>
          {progress && <div className="mt-1 text-xs text-muted-foreground">{t("registrar.processed")}: {progress.processed} · {t("registrar.registered")}: {progress.registered}{progress.total !== null ? ` / ${progress.total}` : ""} · {t("registrar.failed")}: {progress.failed}</div>}
          {progress?.finished && <p className="mt-1 break-all text-xs text-muted-foreground">{progress.results_directory}</p>}
          {requestError && <p role="alert" className="mt-2 text-sm text-destructive">{t(requestError)}</p>}
        </div>
        {running ? <button type="button" disabled={stopping} onClick={() => void stopTask()} className="flex items-center gap-2 rounded-md border border-destructive/50 px-4 py-2 text-sm disabled:opacity-50"><Square className="h-4 w-4"/>{t(stopping ? "registrar.stopping" : "common.stop")}</button>
          : <button type="button" disabled={launching || pending || !listenersReady || !provider || !apiKey.trim()} onClick={() => void startTask()} className="flex items-center gap-2 rounded-md border border-primary/50 bg-primary/10 px-4 py-2 text-sm font-medium text-primary disabled:opacity-50"><Play className="h-4 w-4" />{t(launching ? "registrar.launching" : "registrar.start")}</button>}
      </div>
      {logs.length > 0 && <pre aria-live="polite" className="max-h-72 overflow-auto whitespace-pre-wrap rounded-xl border border-border bg-card p-4 text-xs">{logs.map((line) => {
        const split = line.indexOf("] ");
        const message = line.slice(split + 2);
        const key = `registrar.stages.${message}`;
        const translated = t(key);
        return split >= 0 && translated !== key ? line.slice(0, split + 2) + translated : line;
      }).join("\n")}</pre>}
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
  return <label className="grid cursor-pointer grid-cols-[1rem_minmax(0,1fr)] items-center gap-x-3"><input type="checkbox" checked={checked} onChange={(event) => onChange(event.target.checked)} className="m-0 h-4 w-4 rounded border-border accent-primary" /><span className="text-sm font-medium leading-5">{label}</span>{hint && <span className="col-start-2 mt-0.5 text-xs text-muted-foreground">{hint}</span>}</label>;
}

function CheckRow({ checked, onChange, label }: { checked: boolean; onChange: () => void; label: string }) {
  return <label className="flex cursor-pointer items-center gap-2 rounded-md border border-border bg-background/40 px-3 py-2 text-sm hover:border-primary/40"><input type="checkbox" checked={checked} onChange={onChange} className="m-0 h-4 w-4 shrink-0 rounded border-border accent-primary" /><span className="min-w-0 leading-5">{label}</span></label>;
}

function Choice({ active, onClick, label }: { active: boolean; onClick: () => void; label: string }) {
  return <button type="button" onClick={onClick} className={`rounded-md border px-3 py-1.5 text-sm transition ${active ? "border-primary/50 bg-primary/10 text-primary" : "border-border bg-background text-muted-foreground hover:border-primary/40"}`}>{label}</button>;
}

function ProviderButton({ label, disabled, onClick }: { label: string; disabled: boolean; onClick: () => void }) {
  return <button type="button" disabled={disabled} onClick={onClick} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium text-muted-foreground hover:border-primary/50 hover:text-foreground disabled:opacity-50"><RefreshCw className="h-4 w-4" />{label}</button>;
}
