import { useEffect, useId, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { Check, Copy, CircleHelp, FileText, FolderOpen, Play, Square, RefreshCw, ShieldCheck, Smartphone, WalletCards } from "lucide-react";
import { useT } from "@/i18n";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { countryCallingCode, sortRegistrarCountries } from "@/lib/registrarCountries";
import { optionalPrice, parseSmsTimeout } from "@/lib/registrarSettings";
import { AntiSafetyGuide } from "@/components/AntiSafetyGuide";

type NameMode = "same" | "file" | "random";
type AvatarMode = "emoji" | "folder" | "single" | "generated";
type UsernameMode = "random" | "from_list" | "prefix_random";
type EmailSource = "anti_safety" | "firstmail" | "gmx";
type RecoveryEmailSource = "firstmail" | "gmx" | "mail_tm";

type Provider = { kind: string; label: string; requires_endpoint: boolean };
type Country = { id: string; name: string };
type Price = { country: Country; price: number; available: number | null; operator: string | null };
type Progress = { task_id: string; total: number | null; processed: number; registered: number; failed: number; finished: boolean; reason: string; results_directory: string };
type Identity = { first_name: string; last_name: string };
type StoredSettings = {
  provider: string; apiKey: string; customEndpoint: string; selectedCountries: string[]; countMode: "count" | "budget"; count: string;
  setTwoFa: boolean; twoFaPassword: string; reRegisterExisting: boolean; mobileDevice: boolean; antiSafetyKey: string; antiSafetyAid: string; emailSource: EmailSource; emailFilePath: string; emailPremium: boolean; allowEmailFallback: boolean;
  setRecoveryEmail: boolean; recoveryEmailSource: RecoveryEmailSource; recoveryEmailFilePath: string;
  addToPanel: boolean; openResults: boolean; hidePhoneNumber: boolean; hideOnlineStatus: boolean; setAvatar: boolean; setUsername: boolean; smsTimeout: string; stopOnPriceChange: boolean; minPrice: string; maxPrice: string;
  nameMode: NameMode; firstName: string; lastName: string; namesFile: string; rejectAppCode: boolean; concurrency: string; pollInterval: string;
  failureStop: boolean; failureLimit: string;
  avatarMode: AvatarMode; avatarPath: string; usernameMode: UsernameMode; usernamePrefix: string; usernameListPath: string;
  manualLocale: boolean; registrationLanguage: "ru" | "en"; timezoneOffset: string;
};

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
  const [reRegisterExisting, setReRegisterExisting] = useState(true);
  const [mobileDevice, setMobileDevice] = useState(true);
  const [antiSafetyKey, setAntiSafetyKey] = useState("");
  const [antiSafetyAid, setAntiSafetyAid] = useState("");
  const [emailSource, setEmailSource] = useState<EmailSource>("firstmail");
  const [emailFilePath, setEmailFilePath] = useState("");
  const [setRecoveryEmail, setSetRecoveryEmail] = useState(false);
  const [recoveryEmailSource, setRecoveryEmailSource] = useState<RecoveryEmailSource>("firstmail");
  const [recoveryEmailFilePath, setRecoveryEmailFilePath] = useState("");
  const [emailPremium, setEmailPremium] = useState(false);
  const [allowEmailFallback, setAllowEmailFallback] = useState(false);
  const [addToPanel, setAddToPanel] = useState(true);
  const [openResults, setOpenResults] = useState(false);
  const [hidePhoneNumber, setHidePhoneNumber] = useState(false);
  const [hideOnlineStatus, setHideOnlineStatus] = useState(false);
  const [setAvatar, setSetAvatar] = useState(false);
  const [setUsername, setSetUsername] = useState(false);
  const [avatarMode, setAvatarMode] = useState<AvatarMode>("emoji");
  const [avatarPath, setAvatarPath] = useState("");
  const [usernameMode, setUsernameMode] = useState<UsernameMode>("random");
  const [usernamePrefix, setUsernamePrefix] = useState("");
  const [usernameListPath, setUsernameListPath] = useState("");
  const [manualLocale, setManualLocale] = useState(false);
  const [registrationLanguage, setRegistrationLanguage] = useState<"ru" | "en">("en");
  const [timezoneOffset, setTimezoneOffset] = useState("0");
  const [smsTimeout, setSmsTimeout] = useState("300");
  const [stopOnPriceChange, setStopOnPriceChange] = useState(true);
  const [minPrice, setMinPrice] = useState("");
  const [maxPrice, setMaxPrice] = useState("");
  const [nameMode, setNameMode] = useState<NameMode>("random");
  const [firstName, setFirstName] = useState("");
  const [lastName, setLastName] = useState("");
  const [namesFile, setNamesFile] = useState("");
  const [rejectAppCode, setRejectAppCode] = useState(true);
  const [concurrency, setConcurrency] = useState("1");
  const [pollInterval, setPollInterval] = useState("3");
  const [failureStop, setFailureStop] = useState(false);
  const [failureLimit, setFailureLimit] = useState("");
  const [settingsLock, setSettingsLock] = useState<"loading" | "setup" | "locked" | "unlocked">("loading");
  const [registrarPassword, setRegistrarPassword] = useState("");
  const [passwordConfirmation, setPasswordConfirmation] = useState("");
  const [settingsPasswordError, setSettingsPasswordError] = useState("");
  const [settingsPending, setSettingsPending] = useState(false);
  const [resetConfirmation, setResetConfirmation] = useState(false);
  const [resettingSettings, setResettingSettings] = useState(false);
  const [antiSafetyHelpOpen, setAntiSafetyHelpOpen] = useState(false);
  const [launchNoticeOpen, setLaunchNoticeOpen] = useState(false);
  const [launchNoticeRead, setLaunchNoticeRead] = useState(false);
  const [launchNoticeDismissed, setLaunchNoticeDismissed] = useState(false);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [logsCopied, setLogsCopied] = useState(false);
  const [logsCopyError, setLogsCopyError] = useState(false);
  const [launching, setLaunching] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [listenersReady, setListenersReady] = useState(false);
  const taskId = useRef<string | null>(sessionStorage.getItem("registrar_task_id"));
  const lastEvent = useRef<Progress | null>(null);
  const running = progress !== null && !progress.finished;
  const readableLogs = useMemo(() => logs.map((line) => {
    const split = line.indexOf("] ");
    const message = line.slice(split + 2);
    const floodWait = /^telegram_flood_wait:(\d+)$/.exec(message);
    if (split >= 0 && floodWait) return line.slice(0, split + 2) + t("registrar.stages.telegram_flood_wait", { seconds: floodWait[1] });
    const premiumDays = /^telegram_premium_required_days:(\d+)$/.exec(message);
    if (split >= 0 && premiumDays) return line.slice(0, split + 2) + t("registrar.stages.telegram_premium_required_days", { days: premiumDays[1] });
    const unknownRpc = /^telegram_unknown_rpc:([A-Z][A-Z0-9]*(?:_[A-Z0-9]+)+)$/.exec(message);
    if (split >= 0 && unknownRpc) return line.slice(0, split + 2) + t("registrar.stages.telegram_unknown_rpc", { code: unknownRpc[1] });
    const key = `registrar.stages.${message}`;
    const translated = t(key);
    return split >= 0 && translated !== key ? line.slice(0, split + 2) + translated : line;
  }).join("\n"), [logs, t]);

  const copyLogs = async () => {
    try {
      await writeText(readableLogs);
      setLogsCopyError(false);
      setLogsCopied(true);
      window.setTimeout(() => setLogsCopied(false), 1500);
    } catch {
      setLogsCopyError(true);
    }
  };

  const storedSettings = (): StoredSettings => ({
    provider, apiKey, customEndpoint, selectedCountries, countMode, count, setTwoFa, twoFaPassword, reRegisterExisting, mobileDevice, antiSafetyKey, antiSafetyAid,
    emailSource, emailFilePath, emailPremium, allowEmailFallback, addToPanel, openResults, hidePhoneNumber, hideOnlineStatus, setAvatar, setUsername, smsTimeout, stopOnPriceChange, minPrice, maxPrice, nameMode,
    setRecoveryEmail, recoveryEmailSource, recoveryEmailFilePath,
    firstName, lastName, namesFile, rejectAppCode, concurrency, pollInterval, failureStop, failureLimit,
    avatarMode, avatarPath, usernameMode, usernamePrefix, usernameListPath,
    manualLocale, registrationLanguage, timezoneOffset,
  });

  const applyStoredSettings = (settings: Partial<StoredSettings>) => {
    if (typeof settings.provider === "string") setProvider(settings.provider);
    if (typeof settings.apiKey === "string") setApiKey(settings.apiKey);
    if (typeof settings.customEndpoint === "string") setCustomEndpoint(settings.customEndpoint);
    if (Array.isArray(settings.selectedCountries)) setSelectedCountries(settings.selectedCountries.filter((value): value is string => typeof value === "string"));
    if (settings.countMode === "count" || settings.countMode === "budget") setCountMode(settings.countMode);
    if (typeof settings.count === "string") setCount(settings.count);
    if (typeof settings.setTwoFa === "boolean") setSetTwoFa(settings.setTwoFa);
    if (typeof settings.twoFaPassword === "string") setTwoFaPassword(settings.twoFaPassword);
    if (typeof settings.setRecoveryEmail === "boolean") setSetRecoveryEmail(settings.setRecoveryEmail);
    if (settings.recoveryEmailSource && ["firstmail", "gmx", "mail_tm"].includes(settings.recoveryEmailSource)) setRecoveryEmailSource(settings.recoveryEmailSource);
    if (typeof settings.recoveryEmailFilePath === "string") setRecoveryEmailFilePath(settings.recoveryEmailFilePath);
    if (typeof settings.reRegisterExisting === "boolean") setReRegisterExisting(settings.reRegisterExisting);
    if (typeof settings.mobileDevice === "boolean") setMobileDevice(settings.mobileDevice);
    if (typeof settings.antiSafetyKey === "string") setAntiSafetyKey(settings.antiSafetyKey);
    if (typeof settings.antiSafetyAid === "string") setAntiSafetyAid(settings.antiSafetyAid);
    if (settings.emailSource && ["anti_safety", "firstmail", "gmx"].includes(settings.emailSource)) setEmailSource(settings.emailSource);
    if (typeof settings.emailFilePath === "string") setEmailFilePath(settings.emailFilePath);
    if (typeof settings.emailPremium === "boolean") setEmailPremium(settings.emailPremium);
    if (typeof settings.allowEmailFallback === "boolean") setAllowEmailFallback(settings.allowEmailFallback);
    if (typeof settings.addToPanel === "boolean") setAddToPanel(settings.addToPanel);
    if (typeof settings.openResults === "boolean") setOpenResults(settings.openResults);
    if (typeof settings.hidePhoneNumber === "boolean") setHidePhoneNumber(settings.hidePhoneNumber);
    if (typeof settings.hideOnlineStatus === "boolean") setHideOnlineStatus(settings.hideOnlineStatus);
    if (typeof settings.setAvatar === "boolean") setSetAvatar(settings.setAvatar);
    if (typeof settings.setUsername === "boolean") setSetUsername(settings.setUsername);
    if (settings.avatarMode && ["emoji", "folder", "single", "generated"].includes(settings.avatarMode)) setAvatarMode(settings.avatarMode);
    if (typeof settings.avatarPath === "string") setAvatarPath(settings.avatarPath);
    if (settings.usernameMode && ["random", "from_list", "prefix_random"].includes(settings.usernameMode)) setUsernameMode(settings.usernameMode);
    if (typeof settings.usernamePrefix === "string") setUsernamePrefix(settings.usernamePrefix);
    if (typeof settings.usernameListPath === "string") setUsernameListPath(settings.usernameListPath);
    if (typeof settings.manualLocale === "boolean") setManualLocale(settings.manualLocale);
    if (settings.registrationLanguage === "ru" || settings.registrationLanguage === "en") setRegistrationLanguage(settings.registrationLanguage);
    if (typeof settings.timezoneOffset === "string") setTimezoneOffset(settings.timezoneOffset);
    if (typeof settings.smsTimeout === "string") setSmsTimeout(settings.smsTimeout);
    if (typeof settings.stopOnPriceChange === "boolean") setStopOnPriceChange(settings.stopOnPriceChange);
    if (typeof settings.minPrice === "string") setMinPrice(settings.minPrice);
    if (typeof settings.maxPrice === "string") setMaxPrice(settings.maxPrice);
    if (["same", "file", "random"].includes(settings.nameMode ?? "")) setNameMode(settings.nameMode as NameMode);
    if (typeof settings.firstName === "string") setFirstName(settings.firstName);
    if (typeof settings.lastName === "string") setLastName(settings.lastName);
    if (typeof settings.namesFile === "string") setNamesFile(settings.namesFile);
    if (typeof settings.rejectAppCode === "boolean") setRejectAppCode(settings.rejectAppCode);
    if (typeof settings.concurrency === "string") setConcurrency(settings.concurrency);
    if (typeof settings.pollInterval === "string") setPollInterval(settings.pollInterval);
    if (typeof settings.failureStop === "boolean") setFailureStop(settings.failureStop);
    if (typeof settings.failureLimit === "string") setFailureLimit(settings.failureLimit);
  };

  useEffect(() => {
    void invoke<{ configured: boolean }>("registrar_settings_state")
      .then((state) => setSettingsLock(state.configured ? "locked" : "setup"))
      .catch(() => setSettingsLock("setup"));
  }, []);

  const unlockSettings = async () => {
    if (settingsPending) return;
    setSettingsPasswordError("");
    if (settingsLock === "setup" && registrarPassword !== passwordConfirmation) { setSettingsPasswordError(t("registrar.passwordMismatch")); return; }
    setSettingsPending(true);
    try {
      if (settingsLock === "setup") await invoke("registrar_settings_save", { password: registrarPassword, settings: storedSettings() });
      else applyStoredSettings(await invoke<Partial<StoredSettings>>("registrar_settings_unlock", { password: registrarPassword }));
      setSettingsLock("unlocked");
      setPasswordConfirmation("");
    } catch (error) { setSettingsPasswordError(error instanceof Error ? error.message : t("registrar.settingsUnlockFailed")); }
    finally { setSettingsPending(false); }
  };

  const resetSettings = async () => {
    setResettingSettings(true);
    setSettingsPasswordError("");
    try {
      await invoke("registrar_settings_reset");
      setRegistrarPassword("");
      setPasswordConfirmation("");
      setResetConfirmation(false);
      setSettingsLock("setup");
    } catch (error) {
      setSettingsPasswordError(error instanceof Error ? error.message : String(error));
    } finally {
      setResettingSettings(false);
    }
  };

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
      setProvider((current) => current || items[0]?.kind || "");
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
    const config = { kind: provider, api_key: apiKey, endpoint: customEndpoint || null, http_proxy: null };
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

  const selectEmailFile = async (recovery = false) => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open({ multiple: false, filters: [{ name: "Text", extensions: ["txt"] }] });
    if (typeof path === "string") {
      if (recovery) setRecoveryEmailFilePath(path);
      else setEmailFilePath(path);
    }
  };

  const startTask = async () => {
    setLaunching(true);
    setRequestError("");
    try {
      const pairs = [{ api_id: 6, api_hash: "eb06d4abfb49dc3eeb1aeb98ae0f581e" }];
      const minimum = optionalPrice(minPrice);
      const maximum = optionalPrice(maxPrice);
      if (minimum !== null && maximum !== null && minimum > maximum) throw new Error("registrar.invalidPrice");
      const quantity = Number(count);
      const codeTimeout = parseSmsTimeout(smsTimeout);
      if (countMode === "count" && (!Number.isInteger(quantity) || quantity < 1 || quantity > 100000)) throw new Error("registrar.invalidCount");
      if (!selectedCountries.length) throw new Error("registrar.selectCountries");
      if (setTwoFa && !twoFaPassword) throw new Error("registrar.passwordRequired");
      if (setTwoFa && setRecoveryEmail && recoveryEmailSource !== "mail_tm" && !recoveryEmailFilePath) throw new Error("registrar.recoveryEmailFileRequired");
      if ((mobileDevice || emailSource === "anti_safety") && (!antiSafetyKey.trim() || !antiSafetyAid.trim())) throw new Error("registrar.pushCredentialsRequired");
      if (emailSource !== "anti_safety" && !emailFilePath) throw new Error("registrar.emailFileRequired");
      if (setAvatar && (avatarMode === "folder" || avatarMode === "single") && !avatarPath) throw new Error("registrar.avatarRequired");
      if (setUsername && usernameMode === "from_list" && !usernameListPath) throw new Error("registrar.usernameFileRequired");
      if (manualLocale && (!Number.isInteger(Number(timezoneOffset)) || Number(timezoneOffset) < -12 || Number(timezoneOffset) > 14)) throw new Error("registrar.invalidTimezone");
      if (failureStop && (!Number.isInteger(Number(failureLimit)) || Number(failureLimit) < 1)) throw new Error("registrar.invalidFailureLimit");
      await invoke("registrar_settings_save", { password: registrarPassword, settings: storedSettings() });
      const identities: Identity[] = nameMode === "file"
        ? await invoke<Identity[]>("registrar_read_names", { path: namesFile })
        : [{ first_name: nameMode === "random" ? crypto.randomUUID().replace(/-/g, "").slice(0, 10) : firstName.trim(), last_name: lastName.trim() }];
      if (!identities.length || identities.some((identity) => !identity.first_name)) throw new Error("registrar.namesRequired");
      const attempts = Array.from({ length: Math.max(pairs.length, identities.length, 1) }, (_, index) => ({
        profile: {
          ...pairs[index % pairs.length], system_lang_code: navigator.language, lang_code: navigator.language.split("-")[0],
          timezone_offset_seconds: -new Date().getTimezoneOffset() * 60,
        },
        identity: identities[index % identities.length], proxy: null,
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
        sms_provider: { kind: provider, api_key: apiKey, endpoint: customEndpoint || null, http_proxy: null },
        number_request: { country: selectedCountries[0], service: "telegram", operator: null, max_price: maximum },
        attempts, attempt_count: countMode === "count" ? quantity : null, random_names: nameMode === "random",
        countries: selectedCountries, expected_prices: expectedPrices, use_full_balance: countMode === "budget",
        min_price: minimum, stop_on_price_change: stopOnPriceChange, reject_app_code: rejectAppCode,
        two_fa_password: setTwoFa ? twoFaPassword : null, re_register_existing: reRegisterExisting, send_push: mobileDevice,
        set_recovery_email: setTwoFa && setRecoveryEmail, recovery_email_source: recoveryEmailSource, recovery_email_file_path: recoveryEmailFilePath || null,
        antisafety: mobileDevice || emailSource === "anti_safety" ? { api_key: antiSafetyKey.trim(), aid: antiSafetyAid.trim() } : null,
        email_source: emailSource, email_file_path: emailSource === "anti_safety" ? null : emailFilePath,
        email_premium: emailPremium,
        allow_email_fallback: allowEmailFallback,
        max_consecutive_failures: failureStop ? Number(failureLimit) : null,
        add_to_panel: addToPanel, open_results: openResults, hide_phone_number: hidePhoneNumber, hide_online_status: hideOnlineStatus, set_avatar: setAvatar, set_username: setUsername, concurrency: Number(concurrency),
        avatar_mode: avatarMode, avatar_path: avatarPath, username_mode: usernameMode, username_prefix: usernamePrefix, username_list_path: usernameListPath,
        manual_locale: manualLocale, registration_language: registrationLanguage, timezone_offset_seconds: Number(timezoneOffset) * 3600,
        poll_interval_secs: Number(pollInterval), code_timeout_secs: codeTimeout,
      } });
      taskId.current = id;
      sessionStorage.setItem("registrar_task_id", id);
      setProgress(lastEvent.current?.task_id === id ? lastEvent.current : {
        task_id: id, total: countMode === "count" ? quantity : null, processed: 0, registered: 0,
        failed: 0, finished: false, reason: "running", results_directory: "",
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : typeof error === "string" ? error : "";
      const key = message.startsWith("registrar.") ? message : "registrar.startFailed";
      setRequestError(key);
    } finally { setLaunching(false); }
  };

  const stopTask = async () => {
    if (!taskId.current) return;
    setStopping(true);
    try { await invoke("registrar_stop", { taskId: taskId.current }); }
    catch { setStopping(false); setRequestError("registrar.stopFailed"); }
  };

  const requestStart = () => {
    if (launchNoticeDismissed) void startTask();
    else setLaunchNoticeOpen(true);
  };

  const selectProfileResource = async (kind: "avatar" | "username") => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open(kind === "avatar" && avatarMode === "folder"
      ? { directory: true, multiple: false }
      : { multiple: false, filters: [{ name: kind === "avatar" ? "Images" : "Text", extensions: kind === "avatar" ? ["jpg", "jpeg", "png", "webp"] : ["txt"] }] });
    if (typeof path === "string") {
      if (kind === "avatar") setAvatarPath(path);
      else setUsernameListPath(path);
    }
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

      <fieldset disabled={running || launching} className="space-y-4 disabled:opacity-70">
      <Section icon={WalletCards} title={t("registrar.smsSection")}>
        <div className="grid gap-5 lg:grid-cols-2">
          <Field label={t("registrar.provider")}>
            <RegistrarSelect label={t("registrar.provider")} value={provider} onChange={setProvider} options={providers.map((item) => ({ value: item.kind, label: item.label }))} />
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
              <RegistrarSelect label={t("registrar.quantity")} value={countMode} onChange={(value) => setCountMode(value as "count" | "budget")} className="w-44" options={[{ value: "count", label: t("registrar.exactCount") }, { value: "budget", label: t("registrar.fullBalance") }]} />
              {countMode === "count" && <input value={count} onChange={(event) => setCount(event.target.value)} min="1" type="number" className={inputClass} />}
            </div>
            {countMode === "budget" && <p className="mt-2 text-xs text-muted-foreground">{t("registrar.budgetHint")}</p>}
            {countMode === "count" && <p className="mt-2 text-xs text-muted-foreground">{t("registrar.countHint")}</p>}
          </Field>
          <Field label={t("registrar.smsTimeout")}>
            <input value={smsTimeout} onChange={(event) => setSmsTimeout(event.target.value)} onBlur={() => { if (Number(smsTimeout) < 30) setSmsTimeout("30"); }} min="30" max="900" step="1" type="number" className={inputClass} />
          </Field>
        </div>
      </Section>

      <div className="grid gap-4 xl:grid-cols-2">
        <Section icon={ShieldCheck} title={t("registrar.resultSection")}>
          <div className="space-y-4">
            <ToggleRow checked={setTwoFa} onChange={setSetTwoFa} label={t("registrar.setTwoFa")} />
            {setTwoFa && <Field label={t("registrar.twoFaPassword")}><input value={twoFaPassword} onChange={(event) => setTwoFaPassword(event.target.value)} type="password" autoComplete="new-password" className={inputClass} /></Field>}
            {setTwoFa && <ToggleRow checked={setRecoveryEmail} onChange={setSetRecoveryEmail} label={t("registrar.setRecoveryEmail")} />}
            <ToggleRow checked={reRegisterExisting} onChange={setReRegisterExisting} label={t("registrar.reRegisterExisting")} help={t("registrar.reRegisterExistingHint")} />
            <ToggleRow checked={mobileDevice} onChange={setMobileDevice} label={t("registrar.mobileDevice")} help={t("registrar.mobileDeviceHint")} />
            <ToggleRow checked={addToPanel} onChange={setAddToPanel} label={t("registrar.addToPanel")} />
            <ToggleRow checked={openResults} onChange={setOpenResults} label={t("registrar.openResults")} />
            <ToggleRow checked={hidePhoneNumber} onChange={setHidePhoneNumber} label={t("registrar.hidePhoneNumber")} />
            <ToggleRow checked={hideOnlineStatus} onChange={setHideOnlineStatus} label={t("registrar.hideOnlineStatus")} />
            <ToggleRow checked={setAvatar} onChange={setSetAvatar} label={t("registrar.setAvatar")} />
            {setAvatar && <div className="space-y-2 pl-7">
              <div className="flex flex-wrap items-center gap-2">
                {([["folder", "registrar.avatarFolder"], ["single", "registrar.avatarSingle"], ["generated", "registrar.avatarGenerated"], ["emoji", "registrar.avatarEmoji"]] as const).map(([mode, label]) => <ModeButton key={mode} active={avatarMode === mode} onClick={() => { setAvatarMode(mode); setAvatarPath(""); }}>{t(label)}</ModeButton>)}
              </div>
              {(avatarMode === "folder" || avatarMode === "single") && <PickerButton onClick={() => void selectProfileResource("avatar")} icon={avatarMode === "folder" ? "folder" : "file"} label={t(avatarMode === "folder" ? "registrar.chooseFolder" : "registrar.chooseFile")} value={avatarPath} />}
              {avatarMode === "generated" && <p className="text-xs text-muted-foreground">{t("registrar.avatarGeneratedHint")}</p>}
            </div>}
            <ToggleRow checked={setUsername} onChange={setSetUsername} label={t("registrar.setUsername")} />
            {setUsername && <div className="space-y-2 pl-7">
              <div className="flex flex-wrap items-center gap-2">
                <ModeButton active={usernameMode === "random"} onClick={() => setUsernameMode("random")}>{t("accountActions.usernameRandom")}</ModeButton>
                <ModeButton active={usernameMode === "from_list"} onClick={() => setUsernameMode("from_list")}>{t("accountActions.usernameFromList")}</ModeButton>
                <ModeButton active={usernameMode === "prefix_random"} onClick={() => setUsernameMode("prefix_random")}>{t("accountActions.usernamePrefixRandom")}</ModeButton>
              </div>
              {usernameMode === "prefix_random" && <input value={usernamePrefix} onChange={(event) => setUsernamePrefix(event.target.value)} className={`${inputClass} max-w-xs`} placeholder={t("accountActions.prefixPlaceholder")} />}
              {usernameMode === "from_list" && <><PickerButton onClick={() => void selectProfileResource("username")} icon="file" label={t("accountActions.usernameListLabel")} value={usernameListPath} /><p className="text-xs text-muted-foreground">{t("registrar.usernameFileHint")}</p></>}
            </div>}
            <ToggleRow checked={rejectAppCode} onChange={setRejectAppCode} label={t("registrar.rejectAppCode")} />
          </div>
        </Section>

        <Section icon={Smartphone} title={t("registrar.emailSection")}>
          <div className="space-y-4">
            <Field label={t("registrar.emailSource")}><RegistrarSelect label={t("registrar.emailSource")} value={emailSource} onChange={(value) => setEmailSource(value as EmailSource)} options={[{ value: "anti_safety", label: "AntiSafety" }, { value: "firstmail", label: "Firstmail" }, { value: "gmx", label: "GMX.de / GMX.com" }]} /></Field>
            {emailSource === "anti_safety" ? <div className="space-y-3">
              <div className="flex items-center gap-2.5 text-xs font-medium"><span className={!emailPremium ? "text-primary" : "text-muted-foreground"}>Basic</span><button type="button" role="switch" aria-label={t("registrar.emailPremium")} aria-checked={emailPremium} onClick={() => setEmailPremium((current) => !current)} className={`relative h-6 w-11 rounded-full border transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/50 ${emailPremium ? "border-primary bg-primary" : "border-border bg-muted"}`}><span className={`absolute top-0.5 h-[18px] w-[18px] rounded-full bg-background shadow-sm transition-transform ${emailPremium ? "left-[22px]" : "left-0.5"}`} /></button><span className={emailPremium ? "text-primary" : "text-muted-foreground"}>Premium</span></div>
              <ToggleRow checked={allowEmailFallback} onChange={setAllowEmailFallback} label={t("registrar.emailFallback")} />
            </div> : <div className="space-y-2">
              <PickerButton onClick={() => void selectEmailFile()} icon="file" label={t("registrar.chooseFile")} value={emailFilePath} />
              <p className="text-xs text-muted-foreground">{t("registrar.emailFileHint")}</p>
              {emailSource === "firstmail" && <p className="text-xs text-muted-foreground">{t("registrar.firstmailHint")}</p>}
              {emailSource === "gmx" && <p className="text-xs text-muted-foreground">{t("registrar.gmxHint")}</p>}
            </div>}
          </div>
          {setTwoFa && setRecoveryEmail && <div className="mt-4 space-y-3 border-t border-border pt-4">
            <Field label={t("registrar.recoveryEmailSource")}><RegistrarSelect label={t("registrar.recoveryEmailSource")} value={recoveryEmailSource} onChange={(value) => setRecoveryEmailSource(value as RecoveryEmailSource)} options={[{ value: "firstmail", label: "Firstmail" }, { value: "gmx", label: "GMX.de / GMX.com" }, { value: "mail_tm", label: "mail.tm" }]} /></Field>
            {recoveryEmailSource === "mail_tm" ? <p className="text-xs text-muted-foreground">{t("registrar.mailTmHint")} <a href="https://mail.tm" target="_blank" rel="noopener noreferrer" className="text-primary underline">mail.tm</a></p> : <div className="space-y-2">
              <PickerButton onClick={() => void selectEmailFile(true)} icon="file" label={t("registrar.chooseFile")} value={recoveryEmailFilePath} />
              <p className="text-xs text-muted-foreground">{t("registrar.emailFileHint")}</p>
              <p className="text-xs text-muted-foreground">{t(recoveryEmailSource === "firstmail" ? "registrar.firstmailHint" : "registrar.gmxHint")}</p>
            </div>}
          </div>}
        </Section>
      </div>

      {(mobileDevice || emailSource === "anti_safety") && <Section icon={Smartphone} title={t("registrar.antiSafety")} headingClassName="mb-3">
        <p className="text-xs text-muted-foreground">{t("registrar.antiSafetyHint")}</p>
        <div className="mt-3 grid gap-4 md:grid-cols-2">
          <Field label={t("registrar.antiSafetyKey")}><input value={antiSafetyKey} onChange={(event) => setAntiSafetyKey(event.target.value)} type="password" autoComplete="off" className={inputClass} /></Field>
          <Field label="AID"><input value={antiSafetyAid} onChange={(event) => setAntiSafetyAid(event.target.value)} className={inputClass} placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" /></Field>
        </div>
        <button type="button" onClick={() => setAntiSafetyHelpOpen(true)} className="mt-3 w-fit rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50">{t("registrar.whatIsThis")}</button>
      </Section>}

      <Section icon={Smartphone} title={t("registrar.namesSection")} headingClassName="mb-3">
        <div className="grid gap-4 xl:grid-cols-2">
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
      </Section>

      <Section icon={CircleHelp} title={t("registrar.priceSection")}>
        <ToggleRow checked={stopOnPriceChange} onChange={setStopOnPriceChange} label={t("registrar.stopOnPriceChange")} />
        {stopOnPriceChange && <div className="mt-4 grid grid-cols-2 gap-4">
          <Field label={t("registrar.minPrice")}><input value={minPrice} onChange={(event) => setMinPrice(event.target.value)} type="number" min="0" step="0.01" className={inputClass} placeholder="0.00" /></Field>
          <Field label={t("registrar.maxPrice")}><input value={maxPrice} onChange={(event) => setMaxPrice(event.target.value)} type="number" min="0" step="0.01" className={inputClass} placeholder="0.00" /></Field>
        </div>}
      </Section>
      <Section icon={CircleHelp} title={t("registrar.execution")}>
        <div className="grid gap-4 md:grid-cols-3">
          <Field className="flex h-full flex-col" label={t("common.threads")} hint={t("registrar.concurrencyHint")}><input type="number" min="1" max="100" value={concurrency} onChange={(event) => setConcurrency(event.target.value)} className={inputClass}/></Field>
          <Field className="flex h-full flex-col" label={t("registrar.pollInterval")} hint={t("registrar.pollIntervalHint")}><input type="number" min="2" max="60" value={pollInterval} onChange={(event) => setPollInterval(event.target.value)} className={inputClass}/></Field>
          <Field className="flex h-full flex-col" label={t("registrar.failureLimit")} hint={t("registrar.failureLimitHint")}>
            <RegistrarSelect label={t("registrar.failureLimit")} value={failureStop ? "limit" : "never"} onChange={(value) => setFailureStop(value === "limit")} options={[{ value: "never", label: t("registrar.neverStop") }, { value: "limit", label: t("registrar.stopAtFailures") }]} />
            {failureStop && <input type="number" min="1" value={failureLimit} onChange={(event) => setFailureLimit(event.target.value)} className={`${inputClass} mt-2`} placeholder="1" />}
          </Field>
        </div>
      </Section>
      <Section icon={Smartphone} title={t("registrar.registrationLocale")}>
        <p className="text-xs text-muted-foreground">{t("registrar.autoLocaleHint")}</p>
        <div className="mt-4"><ToggleRow checked={manualLocale} onChange={setManualLocale} label={t("registrar.manualLocale")} /></div>
        {manualLocale && <div className="mt-4 grid gap-4 md:grid-cols-2"><Field label={t("registrar.registrationLanguage")}><RegistrarSelect label={t("registrar.registrationLanguage")} value={registrationLanguage} onChange={(value) => setRegistrationLanguage(value as "ru" | "en")} options={[{ value: "en", label: "English" }, { value: "ru", label: "Русский" }]} /></Field><Field label={t("registrar.timezone")}><RegistrarSelect label={t("registrar.timezone")} value={timezoneOffset} onChange={setTimezoneOffset} options={Array.from({ length: 27 }, (_, index) => index - 12).map((offset) => ({ value: String(offset), label: `UTC${offset >= 0 ? "+" : ""}${offset}` }))} /></Field></div>}
      </Section>
      </fieldset>

      <div className="flex flex-wrap items-center justify-between gap-4 rounded-xl border border-border bg-card px-5 py-4">
        <div className="text-sm">
          <div className="font-medium">{t(progress ? `registrar.reasons.${progress.reason}` : "registrar.ready")}</div>
          {progress && <div className="mt-1 text-xs text-muted-foreground">{t("registrar.processed")}: {progress.processed} · {t("registrar.registered")}: {progress.registered}{progress.total !== null ? ` / ${progress.total}` : ""} · {t("registrar.failed")}: {progress.failed}</div>}
          {progress?.finished && <p className="mt-1 break-all text-xs text-muted-foreground">{progress.results_directory}</p>}
          {requestError && <p role="alert" className="mt-2 text-sm text-destructive">{t(requestError)}</p>}
        </div>
        {running ? <button type="button" disabled={stopping} onClick={() => void stopTask()} className="flex items-center gap-2 rounded-md border border-destructive/50 px-4 py-2 text-sm font-medium disabled:opacity-50"><Square className="h-4 w-4"/>{t(stopping ? "registrar.stopping" : "common.stop")}</button>
          : <button type="button" disabled={launching || pending || !listenersReady || !provider || !apiKey.trim()} onClick={requestStart} className="flex items-center gap-2 rounded-md border border-primary/50 bg-primary/10 px-4 py-2 text-sm font-medium text-primary disabled:opacity-50"><Play className="h-4 w-4" />{t(launching ? "registrar.launching" : "registrar.start")}</button>}
      </div>
      {logs.length > 0 && <div className="rounded-xl border border-border bg-card">
        <div className="flex items-center justify-end border-b border-border px-3 py-2">
          {logsCopyError && <span role="alert" className="mr-auto text-xs text-destructive">{t("common.copyLogsFailed")}</span>}
          <button type="button" onClick={() => void copyLogs()} className="flex items-center gap-1.5 rounded-md px-2 py-1 text-xs text-muted-foreground hover:bg-muted hover:text-foreground">{logsCopied ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}{t(logsCopied ? "common.logsCopied" : "common.copyLogs")}</button>
        </div>
        <pre aria-live="polite" className="max-h-72 cursor-text select-text overflow-auto whitespace-pre-wrap p-4 text-xs">{readableLogs}</pre>
      </div>}
      {antiSafetyHelpOpen && <RegistrarModal title={t("registrar.antiSafetyGuide.title")} onClose={() => setAntiSafetyHelpOpen(false)}><AntiSafetyGuide onClose={() => setAntiSafetyHelpOpen(false)} /></RegistrarModal>}
      {settingsLock !== "unlocked" && settingsLock !== "loading" && <RegistrarModal title={t(settingsLock === "setup" ? "registrar.settingsPasswordTitle" : "registrar.settingsUnlockTitle")} onClose={() => {}}>
        <p>{t(settingsLock === "setup" ? "registrar.settingsPasswordWarning" : "registrar.settingsUnlockHint")}</p>
        <div className="mt-4 space-y-3">
          <input value={registrarPassword} onChange={(event) => setRegistrarPassword(event.target.value)} disabled={settingsPending} type="password" autoComplete={settingsLock === "setup" ? "new-password" : "current-password"} className={inputClass} placeholder={t("registrar.settingsPassword")} />
          {settingsLock === "setup" && <input value={passwordConfirmation} onChange={(event) => setPasswordConfirmation(event.target.value)} disabled={settingsPending} type="password" autoComplete="new-password" className={inputClass} placeholder={t("registrar.settingsPasswordConfirm")} />}
          {settingsPasswordError && <p role="alert" className="text-sm text-destructive">{settingsPasswordError}</p>}
          {resetConfirmation && <p className="rounded-md border border-destructive/40 bg-destructive/5 p-3 text-sm text-destructive">{t("registrar.settingsResetWarning")}</p>}
          <div className="flex flex-wrap items-center gap-2">
            <button type="button" disabled={resettingSettings || settingsPending} onClick={() => void unlockSettings()} className="rounded-md border border-primary/50 bg-primary/10 px-4 py-2 text-sm font-medium text-primary disabled:opacity-50">{settingsPending ? t("registrar.settingsPasswordWorking") : settingsLock === "setup" ? t("registrar.settingsPasswordCreate") : t("registrar.settingsPasswordUnlock")}</button>
            {settingsLock === "locked" && !resetConfirmation && <button type="button" disabled={settingsPending} onClick={() => setResetConfirmation(true)} className="rounded-md border border-destructive/50 px-4 py-2 text-sm font-medium text-destructive disabled:opacity-50">{t("registrar.settingsReset")}</button>}
            {resetConfirmation && <><button type="button" disabled={resettingSettings || settingsPending} onClick={() => void resetSettings()} className="rounded-md border border-destructive/50 bg-destructive/10 px-4 py-2 text-sm font-medium text-destructive disabled:opacity-50">{t("registrar.settingsResetConfirm")}</button><button type="button" disabled={resettingSettings || settingsPending} onClick={() => setResetConfirmation(false)} className="rounded-md border border-border px-4 py-2 text-sm font-medium disabled:opacity-50">{t("common.cancel")}</button></>}
          </div>
        </div>
      </RegistrarModal>}
      {launchNoticeOpen && <RegistrarModal title={t("registrar.launchNotice.title")} onClose={() => {}}>
        <div onScroll={(event) => { const node = event.currentTarget; if (node.scrollTop + node.clientHeight >= node.scrollHeight - 4) setLaunchNoticeRead(true); }} tabIndex={0} className="max-h-[55vh] space-y-5 overflow-y-auto pr-3 leading-6">
          <p>{t("registrar.launchNotice.intro")}</p>
          <p>{t("registrar.launchNotice.purpose")}</p>
          <div className="space-y-3">
            <p className="font-semibold text-foreground">{t("registrar.launchNotice.liability")}</p>
            <ul className="list-disc space-y-2 pl-5">
              {["blocked", "trust", "access", "actions", "consequences", "serviceChanges"].map((key) => <li key={key}>{t(`registrar.launchNotice.${key}`)}</li>)}
            </ul>
          </div>
          <p>{t("registrar.launchNotice.quality")}</p>
          <div className="space-y-4">
            <p>{t("registrar.launchNotice.factors")}</p>
            {["numbers", "proxies", "geography"].map((key) => <div key={key} className="space-y-2">
              <h3 className="font-semibold text-foreground">{t(`registrar.launchNotice.${key}Title`)}</h3>
              <p>{t(`registrar.launchNotice.${key}Description`)}</p>
            </div>)}
          </div>
          <p>{t("registrar.launchNotice.guaranteeBefore")} <strong className="text-foreground">{t("registrar.launchNotice.guaranteeEmphasis")}</strong> {t("registrar.launchNotice.guaranteeAfter")}</p>
          <p>{t("registrar.launchNotice.responsibility")}</p>
          <p className="font-semibold text-foreground">{t("registrar.launchNotice.acceptance")}</p>
        </div>
        <div className="mt-5 flex justify-between gap-3"><button type="button" disabled={!launchNoticeRead} onClick={() => { setLaunchNoticeDismissed(true); setLaunchNoticeOpen(false); }} className="rounded-md border border-destructive/50 px-4 py-2 text-sm font-medium text-destructive disabled:opacity-50">{t("registrar.doNotShowAgain")}</button><button type="button" disabled={!launchNoticeRead} onClick={() => { setLaunchNoticeOpen(false); void startTask(); }} className="rounded-md border border-primary/50 bg-primary/10 px-4 py-2 text-sm font-medium text-primary disabled:opacity-50">{t("registrar.understood")}</button></div>
      </RegistrarModal>}
    </div>
  );
}

const inputClass = "h-9 w-full rounded-md border border-input bg-background px-3 text-sm outline-none transition focus:border-primary/50";

function RegistrarSelect({ label, value, onChange, options, className = "w-full" }: { label: string; value: string; onChange: (value: string) => void; options: { value: string; label: string }[]; className?: string }) {
  return <Select value={value} onValueChange={onChange}>
    <SelectTrigger aria-label={label} className={`${className} bg-background font-medium shadow-none focus:ring-0 focus:border-primary/50`}><SelectValue placeholder={label} /></SelectTrigger>
    <SelectContent position="popper" sideOffset={8} className="max-h-64 border-border bg-card text-foreground shadow-lg data-[side=bottom]:translate-y-0 [&_[data-radix-select-viewport]]:!h-auto">
      {options.map((option) => <SelectItem key={option.value} value={option.value}>{option.label}</SelectItem>)}
    </SelectContent>
  </Select>;
}

function ModeButton({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return <button type="button" onClick={onClick} className={`rounded-md border px-3 py-1 text-xs font-medium transition ${active ? "border-primary/50 bg-primary/10 text-primary" : "border-border bg-background text-muted-foreground hover:border-primary/30"}`}>{children}</button>;
}

function PickerButton({ onClick, icon, label, value }: { onClick: () => void; icon: "file" | "folder"; label: string; value: string }) {
  const t = useT();
  const Icon = icon === "folder" ? FolderOpen : FileText;
  return <div className="flex items-center gap-2"><button type="button" onClick={onClick} className="flex shrink-0 items-center gap-2 rounded-md border border-border bg-background px-3 py-1.5 text-xs font-medium transition hover:border-primary/50"><Icon className="h-3.5 w-3.5" />{label}</button><span className="max-w-[180px] truncate text-xs text-muted-foreground" title={value || undefined}>{value ? value.split(/[/\\]/).pop() : t("common.notSelected")}</span></div>;
}

function Section({ icon: Icon, title, children, headingClassName = "mb-5" }: { icon: typeof WalletCards; title: string; children: React.ReactNode; headingClassName?: string }) {
  return <section className="rounded-xl border border-border bg-card p-6"><div className={`${headingClassName} flex items-center gap-2`}><Icon className="h-4 w-4 text-primary" /><h2 className="text-base font-semibold">{title}</h2></div>{children}</section>;
}

function Field({ label, hint, children, className }: { label: string; hint?: string; children: React.ReactNode; className?: string }) {
  return <div className={className}><label className="text-sm font-medium">{label}</label>{hint && <p className="mt-0.5 text-xs text-muted-foreground">{hint}</p>}<div className="mt-auto pt-1.5">{children}</div></div>;
}

function ToggleRow({ checked, onChange, label, hint, help }: { checked: boolean; onChange: (checked: boolean) => void; label: string; hint?: string; help?: string }) {
  const inputId = useId();
  return <div className="grid grid-cols-[1rem_minmax(0,1fr)] items-center gap-x-3"><input id={inputId} type="checkbox" checked={checked} onChange={(event) => onChange(event.target.checked)} className="m-0 h-4 w-4 rounded border-border accent-primary" /><div><div className="flex items-center gap-1.5"><label htmlFor={inputId} className="cursor-pointer text-sm font-medium leading-5">{label}</label>{help && <InfoTooltip text={help} />}</div>{hint && <span className="mt-0.5 block text-xs text-muted-foreground">{hint}</span>}</div></div>;
}

function InfoTooltip({ text }: { text: string }) {
  return <TooltipProvider><Tooltip><TooltipTrigger asChild><button type="button" aria-label={text} className="inline-flex rounded-sm text-muted-foreground transition hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/50"><CircleHelp className="h-4 w-4" /></button></TooltipTrigger><TooltipContent side="right" className="max-w-sm leading-5">{text}</TooltipContent></Tooltip></TooltipProvider>;
}

function CheckRow({ checked, onChange, label }: { checked: boolean; onChange: () => void; label: string }) {
  return <label className="flex cursor-pointer items-center gap-2 rounded-md border border-border bg-background/40 px-3 py-2 text-sm hover:border-primary/40"><input type="checkbox" checked={checked} onChange={onChange} className="m-0 h-4 w-4 shrink-0 rounded border-border accent-primary" /><span className="min-w-0 leading-5">{label}</span></label>;
}

function Choice({ active, onClick, label }: { active: boolean; onClick: () => void; label: string }) {
  return <button type="button" onClick={onClick} className={`rounded-md border px-3 py-1.5 text-sm font-medium transition ${active ? "border-primary/50 bg-primary/10 text-primary" : "border-border bg-background text-muted-foreground hover:border-primary/40"}`}>{label}</button>;
}

function ProviderButton({ label, disabled, onClick }: { label: string; disabled: boolean; onClick: () => void }) {
  return <button type="button" disabled={disabled} onClick={onClick} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium text-muted-foreground hover:border-primary/50 hover:text-foreground disabled:opacity-50"><RefreshCw className="h-4 w-4" />{label}</button>;
}

function RegistrarModal({ title, children, onClose }: { title: string; children: React.ReactNode; onClose: () => void }) {
  return <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={(event) => { if (event.target === event.currentTarget) onClose(); }}><div className="w-full max-w-3xl rounded-xl border border-border bg-card p-6 shadow-xl"><h2 className="text-lg font-semibold">{title}</h2><div className="mt-4 text-sm text-muted-foreground">{children}</div></div></div>;
}
