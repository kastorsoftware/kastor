import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Play, Square } from "lucide-react";
import { AccountPickerModal } from "@/components/AccountPickerModal";
import { useT } from "@/i18n";
import { ThreadInput } from "@/components/ThreadInput";

type DestinationMode = "channel" | "group";
type SendMode = "repost" | "copy";

interface InterceptorConfig {
  keywords: string;
  targets: string;
  replacements: string;
  destinations: string;
  mode: DestinationMode;
  sendMode: SendMode;
  adminAccountId: string;
  maxFloodWait: number;
  threads: number;
  pollInterval: number;
  revokeAdminAfter: boolean;
  leaveAfterWork: boolean;
}

const defaultConfig: InterceptorConfig = {
  keywords: "",
  targets: "",
  replacements: "",
  destinations: "",
  mode: "group",
  sendMode: "copy",
  adminAccountId: "",
  maxFloodWait: 60,
  threads: 5,
  pollInterval: 1000,
  revokeAdminAfter: true,
  leaveAfterWork: true,
};

const STORAGE_KEY = "interceptor_config";
const IS_DEV = !("__TAURI_INTERNALS__" in window);

function loadSavedConfig(): InterceptorConfig {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return { ...defaultConfig, ...JSON.parse(saved) };
  } catch {}
  return defaultConfig;
}

export function InterceptorPage() {
  const t = useT();
  const [config, setConfig] = useState<InterceptorConfig>(loadSavedConfig);
  const [running, setRunning] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [stats, setStats] = useState({ intercepted: 0, forwarded: 0, errors: 0 });
  const [pickerOpen, setPickerOpen] = useState(false);
  const [adminPickerOpen, setAdminPickerOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [accounts, setAccounts] = useState<{ id: string; phone: string }[]>([]);
  const logsRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(config));
  }, [config]);

  useEffect(() => {
    if (IS_DEV) return;
    invoke<{ accounts: { id: string; phone: string }[] }>("get_accounts_with_stats")
      .then((d) => setAccounts(d.accounts))
      .catch(() => {});
  }, []);

  useEffect(() => {
    if (IS_DEV) return;
    const unlisten = listen<string>("interceptor-log", (e) => {
      const msg = e.payload;
      setLogs((prev) => [...prev, msg]);
      if (msg === "Завершено" || msg === "Done") {
        setRunning(false);
      } else if (msg.includes("перехвачено") || msg.includes("intercepted")) {
        setStats((s) => ({ ...s, intercepted: s.intercepted + 1 }));
      } else if (msg.includes("переслано") || msg.includes("forwarded")) {
        setStats((s) => ({ ...s, forwarded: s.forwarded + 1 }));
      } else if (msg.includes("ОШИБКА") || msg.includes("ERROR")) {
        setStats((s) => ({ ...s, errors: s.errors + 1 }));
      }
    });
    return () => { unlisten.then((f) => f()); };
  }, []);

  useEffect(() => {
    const el = logsRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    if (atBottom) el.scrollTop = el.scrollHeight;
  }, [logs]);

  const set = <K extends keyof InterceptorConfig>(key: K, value: InterceptorConfig[K]) => {
    setConfig((prev) => ({ ...prev, [key]: value }));
  };

  const validate = (): string | null => {
    const kwLines = config.keywords.split("\n").map((l) => l.trim()).filter((l) => l.length > 0);
    if (kwLines.length === 0) return t("interceptor.errNoKeywords");
    if (kwLines.length > 10000) return t("interceptor.errTooManyKeywords");
    const targetLines = config.targets.split("\n").map((l) => l.trim()).filter((l) => l.length > 0);
    if (targetLines.length === 0) return t("interceptor.errNoTargets");
    const destLines = config.destinations.split("\n").map((l) => l.trim()).filter((l) => l.length > 0);
    if (destLines.length === 0) return t("interceptor.errNoDestinations");
    if (config.mode === "channel" && !config.adminAccountId) return t("interceptor.errNoAdmin");
    // validate source ≠ dest
    const targetSet = new Set(targetLines.map((l) => l.toLowerCase()));
    for (const d of destLines) {
      if (targetSet.has(d.toLowerCase())) return t("interceptor.errOverlap", { link: d });
    }
    return null;
  };

  const handleStart = () => {
    const err = validate();
    if (err) { setError(err); return; }
    setError(null);
    setPickerOpen(true);
  };

  const handleAccountsSelected = async (ids: string[]) => {
    if (ids.length === 0) return;
    setLogs([]);
    setStats({ intercepted: 0, forwarded: 0, errors: 0 });
    setRunning(true);
    try {
      const tid = await invoke<string>("interceptor_start", {
        ids,
        config: {
          keywords: config.keywords.split("\n").map((l) => l.trim()).filter((l) => l.length > 0),
          targets: config.targets.split("\n").map((l) => l.trim()).filter((l) => l.length > 0),
          replacements: config.replacements,
          destinations: config.destinations.split("\n").map((l) => l.trim()).filter((l) => l.length > 0),
          mode: config.mode,
          send_mode: config.sendMode,
          admin_account_id: config.mode === "channel" ? config.adminAccountId : "",
          max_flood_wait: config.maxFloodWait,
          poll_interval: config.pollInterval,
          revoke_admin_after: config.mode === "channel" ? config.revokeAdminAfter : false,
          leave_after_work: config.leaveAfterWork,
        },
        threads: config.threads,
      });
      setTaskId(tid);
    } catch (e: any) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${e}`]);
      setRunning(false);
    }
  };

  const handleStop = async () => {
    if (!IS_DEV && taskId) await invoke("interceptor_stop", { taskId }).catch(() => {});
    setRunning(false);
    setLogs((prev) => [...prev, t("common.stoppedByUser")]);
  };

  const handleAdminSelected = (ids: string[]) => {
    if (ids.length > 0) set("adminAccountId", ids[0]);
  };

  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">{t("descriptions.interceptor")}</p>
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div className="p-6 space-y-5">

          {/* mode */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("interceptor.destinationLabel")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <ModeButton active={config.mode === "group"} onClick={() => set("mode", "group")}>{t("interceptor.modeGroup")}</ModeButton>
              <ModeButton active={config.mode === "channel"} onClick={() => set("mode", "channel")}>{t("interceptor.modeChannel")}</ModeButton>
            </div>
            {config.mode === "channel" && (
              <p className="text-xs text-muted-foreground mt-1">
                {t("interceptor.channelAdminHint")}
              </p>
            )}
          </div>

          {/* destination */}
          <div>
            <label className="text-sm font-medium text-foreground">
              {config.mode === "channel" ? t("interceptor.destinationLabel") : t("interceptor.destinationLabelGroup")}
            </label>
            <textarea
              value={config.destinations}
              onChange={(e) => set("destinations", e.target.value)}
              placeholder="@channel1&#10;https://t.me/channel2&#10;t.me/+invite"
              rows={3}
              className="mt-1.5 w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50 font-mono resize-y block"
            />
            <p className="text-xs text-muted-foreground mt-1">
              {t("interceptor.destinationsHint")}
            </p>
          </div>

          {/* send mode */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("interceptor.sendMode")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <ModeButton active={config.sendMode === "copy"} onClick={() => set("sendMode", "copy")}>{t("interceptor.modeOwn")}</ModeButton>
              <ModeButton active={config.sendMode === "repost"} onClick={() => set("sendMode", "repost")}>{t("interceptor.modeRepost")}</ModeButton>
            </div>
            <p className="text-xs text-muted-foreground mt-1">
              {t("interceptor.ownHint")}
            </p>
          </div>

          {/* admin account picker */}
          {config.mode === "channel" && (
            <div className="space-y-4">
              <div>
                <label className="text-sm font-medium text-foreground">{t("interceptor.adminAccount")}</label>
                <div className="flex items-center gap-2 mt-1.5">
                  <button onClick={() => setAdminPickerOpen(true)} className="rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition">
                    {config.adminAccountId ? (() => {
                      const idx = accounts.findIndex((a) => a.id === config.adminAccountId);
                      const acc = accounts[idx];
                      return acc ? `#${idx + 1} +${acc.phone || "?"}` : `#? (${t("interceptor.selected")})`;
                    })() : t("interceptor.selectAccount")}
                  </button>
                </div>
              </div>
              <label className="flex items-center gap-3 cursor-pointer">
                <input type="checkbox" checked={config.revokeAdminAfter} onChange={(e) => set("revokeAdminAfter", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
                <span className="text-sm font-medium text-foreground">{t("interceptor.revokeAdmin")}</span>
              </label>
              <label className="flex items-center gap-3 cursor-pointer">
                <input type="checkbox" checked={config.leaveAfterWork} onChange={(e) => set("leaveAfterWork", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
                <span className="text-sm font-medium text-foreground">{t("interceptor.leaveAfter")}</span>
              </label>
            </div>
          )}

          {/* keywords */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("interceptor.keywordsLabel")}</label>
            <textarea
              value={config.keywords}
              onChange={(e) => set("keywords", e.target.value)}
              placeholder="keyword1&#10;keyword2&#10;keyword3"
              rows={6}
              className="mt-1.5 w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50 font-mono resize-y block"
            />
            <p className="text-xs text-muted-foreground mt-1">
              {t("interceptor.keywordsHint")}
            </p>
          </div>

          {/* targets */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("interceptor.targetsLabel")}</label>
            <textarea
              value={config.targets}
              onChange={(e) => set("targets", e.target.value)}
              placeholder="@group1&#10;https://t.me/group2&#10;t.me/+invite"
              rows={4}
              className="mt-1.5 w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50 font-mono resize-y block"
            />
            <p className="text-xs text-muted-foreground mt-1">
              {t("interceptor.targetsHint")}
            </p>
          </div>

          {/* replacements */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("interceptor.replacementsLabel")}</label>
            <textarea
              value={config.replacements}
              onChange={(e) => set("replacements", e.target.value)}
              placeholder="old_word:new_word&#10;@oldchannel:@newchannel"
              rows={3}
              className="mt-1.5 w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50 font-mono resize-y block"
            />
            <p className="text-xs text-muted-foreground mt-1">
              {t("interceptor.replacementsHint")}
            </p>
          </div>

          {/* poll interval */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("interceptor.pollInterval")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <input
                type="number"
                min={500}
                max={15000}
                step={100}
                value={config.pollInterval}
                onChange={(e) => set("pollInterval", Math.max(500, Math.min(15000, Number(e.target.value))))}
                className="w-28 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50"
              />
              <span className="text-xs text-muted-foreground">{t("interceptor.pollHint")}</span>
            </div>
          </div>

          {/* max flood wait */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("interceptor.maxFloodWait")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <input
                type="number"
                min={0}
                max={86400}
                value={config.maxFloodWait}
                onChange={(e) => set("maxFloodWait", Math.max(0, Math.min(86400, Number(e.target.value))))}
                className="w-28 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50"
              />
              <span className="text-xs text-muted-foreground">{t("interceptor.floodZero")}</span>
            </div>
          </div>

        </div>
      </div>

      {error && (
        <div className="flex items-center gap-3 rounded-md border border-destructive/30 bg-destructive/5 px-4 py-3 text-sm">
          <span className="text-destructive">{error}</span>
          <button onClick={() => setError(null)} className="ml-auto text-muted-foreground hover:text-foreground">&#10005;</button>
        </div>
      )}

      <div className="flex items-center gap-3">
        <div className="flex items-center gap-6 text-sm font-semibold">
          <span>{t("interceptor.intercepted")}: <span className="text-[oklch(0.65_0.1_150)]">{stats.intercepted}</span></span>
          <span>{t("interceptor.forwarded")}: <span className="text-[oklch(0.65_0.1_280)]">{stats.forwarded}</span></span>
          <span>{t("common.errors")}: <span className="text-[oklch(0.55_0.1_25)]">{stats.errors}</span></span>
        </div>
        <div className="ml-auto flex items-center gap-3">
          <span className="max-w-[220px] text-right text-xs text-muted-foreground">{t("common.startSelectAccountsHint")}</span>
          <button onClick={handleStart} disabled={running} className="flex items-center gap-2 rounded-md border border-primary/50 bg-primary/10 px-4 py-2 text-sm text-primary font-medium hover:bg-primary/20 transition disabled:opacity-50">
            <Play className="h-4 w-4" /> {t("common.start")}
          </button>
          <button onClick={handleStop} disabled={!running} className="flex items-center gap-2 rounded-md border px-4 py-2 text-sm font-medium transition disabled:opacity-50" style={{ borderColor: "color-mix(in oklch, oklch(0.55 0.1 25) 50%, transparent)", color: "oklch(0.55 0.1 25)", background: "color-mix(in oklch, oklch(0.55 0.1 25) 6%, transparent)" }}>
            <Square className="h-4 w-4" /> {t("common.stop")}
          </button>
          <div className="flex items-center gap-2">
            <span className="text-xs text-muted-foreground">{t("common.threads")}:</span>
            <ThreadInput value={config.threads} onChange={(v) => set("threads", v)} min={1} max={100} />
          </div>
        </div>
      </div>

      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div ref={logsRef} className="h-40 overflow-y-auto scrollbar-thin p-4 font-mono text-xs space-y-0.5">
          {logs.length === 0 && <div className="text-muted-foreground text-center py-8">{t("common.logsPlaceholder")}</div>}
          {logs.map((line, i) => <div key={i} className="text-muted-foreground whitespace-pre-wrap break-all">{line}</div>)}
        </div>
      </div>

      <AccountPickerModal open={pickerOpen} onClose={() => setPickerOpen(false)} onSelect={handleAccountsSelected} />
      <AccountPickerModal open={adminPickerOpen} onClose={() => setAdminPickerOpen(false)} onSelect={handleAdminSelected} />
    </div>
  );
}

function ModeButton({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button onClick={onClick} className={`rounded-md border px-3 py-1.5 text-xs font-medium transition ${active ? "border-primary/50 bg-primary/10 text-primary" : "border-border bg-background text-muted-foreground hover:border-primary/30"}`}>
      {children}
    </button>
  );
}
