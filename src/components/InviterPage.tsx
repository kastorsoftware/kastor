import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Play, Square, FolderOpen, Plus, X } from "lucide-react";
import { AccountPickerModal } from "@/components/AccountPickerModal";
import { ThreadInput } from "@/components/ThreadInput";
import { useT } from "@/i18n";
import { isDone, isInvited, isError } from "@/lib/eventParser";

type InviteMode = "normal" | "admin";
type SourceMode = "contacts" | "usernames" | "phones";
type DelayUnit = "seconds" | "minutes";
type OnlineFilter = "any" | "recent" | "week" | "month" | "long";

interface AutoStopRules {
  maxBan: number;
  maxSpamblock: number;
  maxFlood: number;
  maxSequentialErrors: number;
}

interface InviterConfig {
  mode: InviteMode;
  targets: string[];
  threads: number;
  maxPerAccount: number;
  batchSize: number;
  delayMin: number;
  delayMax: number;
  delayUnit: DelayUnit;
  maxFloodWait: number;
  sourceMode: SourceMode;
  usernamesPath: string;
  phonesPath: string;
  onlineFilter: OnlineFilter;
  // admin mode extras
  adminAccountId: string;
  delayAfterAdmin: number;
  revokeAdminAfter: boolean;
  leaveAfterWork: boolean;
  checkUsers: boolean;
  peerFloodLimit: number;
  forceMode: boolean;
  verifyAfterInvite: boolean;
  autostop: AutoStopRules;
}

const defaultConfig: InviterConfig = {
  mode: "normal",
  targets: [""],
  threads: 5,
  maxPerAccount: 50,
  batchSize: 5,
  delayMin: 3,
  delayMax: 5,
  delayUnit: "seconds",
  maxFloodWait: 60,
  sourceMode: "usernames",
  usernamesPath: "",
  phonesPath: "",
  onlineFilter: "any",
  adminAccountId: "",
  delayAfterAdmin: 5,
  revokeAdminAfter: true,
  leaveAfterWork: true,
  checkUsers: true,
  peerFloodLimit: 3,
  forceMode: false,
  verifyAfterInvite: true,
  autostop: { maxBan: 3, maxSpamblock: 5, maxFlood: 5, maxSequentialErrors: 10 },
};

const STORAGE_KEY = "inviter_config";
const IS_DEV = !("__TAURI_INTERNALS__" in window);

function loadSavedConfig(): InviterConfig {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return { ...defaultConfig, ...JSON.parse(saved) };
  } catch {}
  return defaultConfig;
}

export function InviterPage() {
  const t = useT();
  const [config, setConfig] = useState<InviterConfig>(loadSavedConfig);
  const [running, setRunning] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [stats, setStats] = useState({ invited: 0, errors: 0 });
  const [pickerOpen, setPickerOpen] = useState(false);
  const [adminPickerOpen, setAdminPickerOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [accounts, setAccounts] = useState<{ id: string; phone: string }[]>([]);
  const logsRef = useRef<HTMLDivElement>(null);

  useEffect(() => { localStorage.setItem(STORAGE_KEY, JSON.stringify(config)); }, [config]);

  useEffect(() => {
    if (IS_DEV) return;
    invoke<{ accounts: { id: string; phone: string }[] }>("get_accounts_with_stats")
      .then((d) => setAccounts(d.accounts))
      .catch(() => {});
  }, []);

  useEffect(() => {
    if (IS_DEV) return;
    const unlisten = listen<string>("inviter-log", (e) => {
      setLogs((prev) => [...prev, e.payload]);
      const msg = e.payload;
      if (isDone(msg)) setRunning(false);
      else if (isInvited(msg)) setStats((s) => ({ ...s, invited: s.invited + 1 }));
      else if (isError(msg) || msg.includes("PEER_FLOOD")) setStats((s) => ({ ...s, errors: s.errors + 1 }));
    });
    return () => { unlisten.then((f) => f()); };
  }, []);

  useEffect(() => {
    const el = logsRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    if (atBottom) el.scrollTop = el.scrollHeight;
  }, [logs]);

  const set = <K extends keyof InviterConfig>(key: K, value: InviterConfig[K]) => {
    setConfig((prev) => ({ ...prev, [key]: value }));
    setError(null);
  };

  const setAutostop = <K extends keyof AutoStopRules>(key: K, value: AutoStopRules[K]) => {
    setConfig((prev) => ({ ...prev, autostop: { ...prev.autostop, [key]: value } }));
  };

  const addTarget = () => set("targets", [...config.targets, ""]);
  const removeTarget = (idx: number) => {
    const t = [...config.targets];
    t.splice(idx, 1);
    if (t.length === 0) t.push("");
    set("targets", t);
  };
  const updateTarget = (idx: number, val: string) => {
    const t = [...config.targets];
    t[idx] = val;
    set("targets", t);
  };

  const selectUsernames = async () => {
    if (IS_DEV) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open({ multiple: false, filters: [{ name: "Text", extensions: ["txt"] }] });
    if (path) set("usernamesPath", path as string);
  };

  const selectPhones = async () => {
    if (IS_DEV) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open({ multiple: false, filters: [{ name: "Text", extensions: ["txt"] }] });
    if (path) set("phonesPath", path as string);
  };

  const validate = (): string | null => {
    if (config.targets.every((t) => !t.trim())) return t("validation.specifyTarget");
    if (config.sourceMode === "usernames" && !config.usernamesPath) return t("validation.selectUsernamesFile");
    if (config.sourceMode === "phones" && !config.phonesPath) return t("validation.selectPhonesFile");
    if (config.mode === "admin" && !config.adminAccountId) return t("validation.selectAdminAccount");
    return null;
  };

  const handleStart = () => {
    const err = validate();
    if (err) { setError(err); return; }
    setPickerOpen(true);
  };

  const handleAccountsSelected = async (ids: string[]) => {
    if (ids.length === 0) return;
    setLogs([]); setStats({ invited: 0, errors: 0 }); setRunning(true); setError(null);
    try {
      const tid = await invoke<string>("inviter_start", {
        ids,
        config: {
          mode: config.mode,
          targets: config.targets.map((t) => t.trim()).filter(Boolean),
          target: "",
          max_per_account: config.maxPerAccount,
          batch_size: config.batchSize,
          delay_min: config.delayMin,
          delay_max: config.delayMax,
          delay_unit: config.delayUnit,
          max_flood_wait: config.maxFloodWait,
          source_mode: config.sourceMode,
          usernames_path: config.sourceMode === "usernames" ? config.usernamesPath : "",
          phones_path: config.sourceMode === "phones" ? config.phonesPath : "",
          online_filter: config.onlineFilter,
          admin_account_id: config.mode === "admin" ? config.adminAccountId : "",
          delay_after_admin: config.delayAfterAdmin,
          revoke_admin_after: config.revokeAdminAfter,
          leave_after_work: config.leaveAfterWork,
          check_users: true,
          peer_flood_limit: config.peerFloodLimit,
          force_mode: config.forceMode,
          verify_after_invite: config.verifyAfterInvite,
          autostop: {
            max_ban: config.autostop.maxBan,
            max_spamblock: config.autostop.maxSpamblock,
            max_flood: config.autostop.maxFlood,
            max_sequential_errors: config.autostop.maxSequentialErrors,
          },
        },
        threads: config.threads,
      });
      setTaskId(tid);
    } catch (e: any) { setLogs((prev) => [...prev, `${t("common.error")}: ${e}`]); setRunning(false); }
  };

  const handleStop = async () => {
    if (!IS_DEV && taskId) await invoke("inviter_stop", { taskId }).catch(() => {});
    setRunning(false); setLogs((prev) => [...prev, t("common.stoppedByUser")]);
  };

  const handleAdminSelected = (ids: string[]) => {
    if (ids.length > 0) set("adminAccountId", ids[0]);
  };

  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">{t("descriptions.inviter")}</p>
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div className="p-6 space-y-5">

          {/* mode */}
          <div>
            <label className="block text-sm font-medium text-foreground mb-1">{t("inviter.title")}</label>
            <div className="flex items-center gap-2">
              <ModeButton active={config.mode === "normal"} onClick={() => set("mode", "normal")}>{t("inviter.modeNormal")}</ModeButton>
              <ModeButton active={config.mode === "admin"} onClick={() => set("mode", "admin")}>{t("inviter.modeAdmin")}</ModeButton>
            </div>
          </div>

          {/* targets (multiple groups) */}
          <div>
            <label className="block text-sm font-medium text-foreground mb-1">
              {t("inviter.target")}
            </label>
            <div className="space-y-2">
              {config.targets.map((t, idx) => (
                <div key={idx} className="flex items-center gap-2">
                  <input
                    value={t}
                    onChange={(e) => updateTarget(idx, e.target.value)}
                    placeholder="https://t.me/group / @username / t.me/+invite"
                    className="flex-1 rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50"
                  />
                  {config.targets.length > 1 && (
                    <button onClick={() => removeTarget(idx)} className="p-1 text-muted-foreground hover:text-destructive transition">
                      <X className="h-4 w-4" />
                    </button>
                  )}
                </div>
              ))}
              <button onClick={addTarget} className="flex items-center gap-1.5 text-xs text-primary hover:text-primary/80 transition mt-1">
                <Plus className="h-3.5 w-3.5" />
              </button>
            </div>
            {config.targets.length > 1 && (
              <p className="text-xs text-muted-foreground mt-1">{t("inviter.roundRobinHint")}</p>
            )}
            {config.mode === "normal" && <p className="text-xs text-muted-foreground mt-1">{t("inviter.groupsOnlyHint")}</p>}
          </div>

          {/* admin account picker */}
          {config.mode === "admin" && (
            <div>
              <label className="block text-sm font-medium text-foreground mb-1">{t("inviter.modeAdmin")}</label>
              <div className="flex items-center gap-2">
                <button onClick={() => setAdminPickerOpen(true)} className="rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition">
                  {config.adminAccountId ? (() => {
                    const idx = accounts.findIndex(a => a.id === config.adminAccountId);
                    const acc = accounts[idx];
                    return acc ? `#${idx + 1} +${acc.phone || "?"}` : `#? (✓)`;
                  })() : t("common.selectAccounts")}
                </button>
              </div>
            </div>
          )}

          {/* source */}
          <div>
            <label className="block text-sm font-medium text-foreground mb-1">{t("inviter.sourceContacts")}</label>
            <div className="flex items-center gap-2">
              <ModeButton active={config.sourceMode === "contacts"} onClick={() => set("sourceMode", "contacts")}>{t("inviter.sourceContacts")}</ModeButton>
              <ModeButton active={config.sourceMode === "usernames"} onClick={() => set("sourceMode", "usernames")}>{t("inviter.sourceUsernames")}</ModeButton>
              <ModeButton active={config.sourceMode === "phones"} onClick={() => set("sourceMode", "phones")}>{t("inviter.sourcePhones")}</ModeButton>
            </div>
            {config.sourceMode === "usernames" && (
              <div className="mt-2 flex items-center gap-2">
                <button onClick={selectUsernames} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition">
                  <FolderOpen className="h-4 w-4" /> {t("inviter.sourceUsernames")}
                </button>
                <span className="text-xs text-muted-foreground truncate max-w-sm">{config.usernamesPath ? config.usernamesPath.split(/[/\\]/).pop() : t("common.notSelected")}</span>
              </div>
            )}
            {config.sourceMode === "phones" && (
              <div className="mt-2 flex items-center gap-2">
                <button onClick={selectPhones} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition">
                  <FolderOpen className="h-4 w-4" /> {t("inviter.sourcePhones")}
                </button>
                <span className="text-xs text-muted-foreground truncate max-w-sm">{config.phonesPath ? config.phonesPath.split(/[/\\]/).pop() : t("common.notSelected")}</span>
              </div>
            )}
          </div>

          {/* online filter */}
          {config.sourceMode === "contacts" && (
            <div>
              <label className="block text-sm font-medium text-foreground mb-1">{t("inviter.onlineFilterLabel")}</label>
              <select value={config.onlineFilter} onChange={(e) => set("onlineFilter", e.target.value as OnlineFilter)} className="w-full max-w-xs rounded-md border border-border bg-card px-3 py-2 text-sm outline-none focus:border-primary/50">
                <option value="any">{t("inviter.onlineOptionAny")}</option>
                <option value="recent">{t("inviter.onlineOptionRecent")}</option>
                <option value="week">{t("inviter.onlineOptionWeek")}</option>
                <option value="month">{t("inviter.onlineOptionMonth")}</option>
                <option value="long">{t("inviter.onlineOptionLong")}</option>
              </select>
            </div>
          )}

          {/* max per account + batch size */}
          <div className="flex items-center gap-6">
            <div>
              <label className="block text-sm font-medium text-foreground mb-1">{t("inviter.maxInvitesLabel")}</label>
              <input type="number" min={1} max={9999} value={config.maxPerAccount} onChange={(e) => set("maxPerAccount", Math.max(1, Number(e.target.value)))} className="w-28 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
            </div>
            <div>
              <label className="block text-sm font-medium text-foreground mb-1">{t("inviter.batchSizeLabel")}</label>
              <input type="number" min={1} max={50} value={config.batchSize} onChange={(e) => set("batchSize", Math.max(1, Math.min(50, Number(e.target.value))))} className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
              <p className="text-xs text-muted-foreground mt-1">{t("inviter.batchSizeHint")}</p>
            </div>
          </div>

          {/* delay */}
          <div>
            <label className="block text-sm font-medium text-foreground mb-1">{t("inviter.delayLabel")}</label>
            <div className="flex items-center gap-2">
              <input type="number" min={0} max={9999} value={config.delayMin} onChange={(e) => set("delayMin", Math.max(0, Number(e.target.value)))} className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
              <span className="text-muted-foreground text-sm">—</span>
              <input type="number" min={0} max={9999} value={config.delayMax} onChange={(e) => set("delayMax", Math.max(0, Number(e.target.value)))} className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
              <select value={config.delayUnit} onChange={(e) => set("delayUnit", e.target.value as DelayUnit)} className="rounded-md border border-border bg-card px-2.5 py-1.5 text-sm outline-none focus:border-primary/50 appearance-none bg-[url('data:image/svg+xml;charset=utf-8,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%20width%3D%2216%22%20height%3D%2216%22%20viewBox%3D%220%200%2024%2024%22%20fill%3D%22none%22%20stroke%3D%22%23888%22%20stroke-width%3D%222%22%3E%3Cpath%20d%3D%22m6%209%206%206%206-6%22%2F%3E%3C%2Fsvg%3E')] bg-[length:16px] bg-[right_8px_center] bg-no-repeat pr-8">
                <option value="seconds">{t("common.seconds")}</option>
                <option value="minutes">{t("common.minutes")}</option>
              </select>
            </div>
          </div>

          {/* force mode + verify */}
          <div className="space-y-2">
            <label className="flex items-center gap-3 cursor-pointer">
              <input type="checkbox" checked={config.forceMode} onChange={(e) => set("forceMode", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
              <span className="text-sm font-medium text-foreground">{t("inviter.forceModeLabel")}</span>
            </label>
            <label className="flex items-center gap-3 cursor-pointer">
              <input type="checkbox" checked={config.verifyAfterInvite} onChange={(e) => set("verifyAfterInvite", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
              <span className="text-sm font-medium text-foreground">{t("inviter.verifyLabel")}</span>
            </label>
          </div>

          {/* AutoStop rules */}
          <div className="border-t border-border pt-4">
            <label className="block text-sm font-medium text-foreground mb-1">{t("inviter.autostopLabel")}</label>
            <p className="text-xs text-muted-foreground mb-2">{t("inviter.autostopHint")}</p>
            <div className="grid grid-cols-2 gap-3">
              <div>
                <label className="text-xs text-muted-foreground">{t("inviter.maxBans")}</label>
                <input type="number" min={0} max={999} value={config.autostop.maxBan} onChange={(e) => setAutostop("maxBan", Math.max(0, Number(e.target.value)))} className="mt-1 w-full rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
              </div>
              <div>
                <label className="text-xs text-muted-foreground">{t("inviter.maxRestrictions")}</label>
                <input type="number" min={0} max={999} value={config.autostop.maxSpamblock} onChange={(e) => setAutostop("maxSpamblock", Math.max(0, Number(e.target.value)))} className="mt-1 w-full rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
              </div>
              <div>
                <label className="text-xs text-muted-foreground">{t("inviter.maxPeerFlood")}</label>
                <input type="number" min={0} max={999} value={config.peerFloodLimit} onChange={(e) => set("peerFloodLimit", Math.max(0, Number(e.target.value)))} className="mt-1 w-full rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
              </div>
              <div>
                <label className="text-xs text-muted-foreground">{t("inviter.maxFloodWait")}</label>
                <input type="number" min={0} max={999} value={config.autostop.maxFlood} onChange={(e) => setAutostop("maxFlood", Math.max(0, Number(e.target.value)))} className="mt-1 w-full rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
              </div>
              <div>
                <label className="text-xs text-muted-foreground">{t("inviter.maxSequential")}</label>
                <input type="number" min={0} max={999} value={config.autostop.maxSequentialErrors} onChange={(e) => setAutostop("maxSequentialErrors", Math.max(0, Number(e.target.value)))} className="mt-1 w-full rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
              </div>
            </div>
          </div>

          {/* admin mode extras */}
          {config.mode === "admin" && (
            <div className="space-y-4 border-t border-border pt-4">
              <div>
                <label className="block text-sm font-medium text-foreground mb-1">{t("inviter.adminDelay")}</label>
                <input type="number" min={0} max={300} value={config.delayAfterAdmin} onChange={(e) => set("delayAfterAdmin", Math.max(0, Number(e.target.value)))} className="w-28 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
              </div>
              <label className="flex items-center gap-3 cursor-pointer">
                <input type="checkbox" checked={config.revokeAdminAfter} onChange={(e) => set("revokeAdminAfter", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
                <span className="text-sm font-medium text-foreground">{t("inviter.revokeAdminLabel")}</span>
              </label>
              <label className="flex items-center gap-3 cursor-pointer">
                <input type="checkbox" checked={config.leaveAfterWork} onChange={(e) => set("leaveAfterWork", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
                <span className="text-sm font-medium text-foreground">{t("inviter.leaveAfterLabel")}</span>
              </label>
            </div>
          )}

          {/* flood wait */}
          <div className="border-t border-border pt-4">
            <label className="block text-sm font-medium text-foreground mb-1">{t("inviter.floodWaitLabel")}</label>
            <div className="flex items-center gap-2">
              <input type="number" min={0} max={86400} value={config.maxFloodWait} onChange={(e) => set("maxFloodWait", Math.max(0, Math.min(86400, Number(e.target.value))))} className="w-28 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
              <span className="text-xs text-muted-foreground">{t("inviter.floodWaitZero")}</span>
            </div>
          </div>

        </div>
      </div>

      {error && (
        <div className="flex items-center gap-3 rounded-md border border-destructive/30 bg-destructive/5 px-4 py-3 text-sm">
          <span className="text-destructive">{error}</span>
          <button onClick={() => setError(null)} className="ml-auto text-muted-foreground hover:text-foreground">✕</button>
        </div>
      )}

      {/* controls */}
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-6 text-sm font-semibold">
          <span>{t("inviter.invited")}: <span className="text-[oklch(0.65_0.1_150)]">{stats.invited}</span></span>
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

      {/* logs */}
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
