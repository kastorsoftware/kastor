import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Play, Square, AlertTriangle, Check, FolderOpen, FileText } from "lucide-react";
import { AccountPickerModal } from "@/components/AccountPickerModal";
import { ThreadInput } from "@/components/ThreadInput";
import { useT } from "@/i18n";

type UsernameMode = "random" | "from_list" | "prefix_random";

interface ActionConfig {
  deleteUsername: boolean;
  changeUsername: boolean;
  usernameMode: UsernameMode;
  usernamePrefix: string;
  usernameListPath: string;

  setPhoto: boolean;
  photoFolderPath: string;
  deleteAllPhotos: boolean;

  changeName: boolean;
  nameOnly: boolean;
  namesFilePath: string;
  surnamesFilePath: string;

  changeBio: boolean;
  bioFilePath: string;
  bioMode: "random" | "single";
  bioSingle: string;
  deleteBio: boolean;

  setBirthday: boolean;
  birthdayDayRange: string;
  birthdayMonths: number[];
  birthdayYearRange: string;

  setEmojiAvatar: boolean;

  resetPassword: boolean;
  setPassword: boolean;
  passwordValue: string;

  deleteContacts: boolean;
  deleteAllDialogs: boolean;
  deleteBotDialogs: boolean;
  deleteFolders: boolean;
  deleteAllStories: boolean;

  readAllDialogs: boolean;

  hidePhoneNumber: boolean;
  hideOnlineStatus: boolean;

  unsubscribeChannels: boolean;

  logoutAfter: boolean;
  deleteAccount: boolean;

  randomizeOrder: boolean;
  setAutoPhoto: boolean;
  delayBetweenMin: number;
  delayBetweenMax: number;
  accountTtl: number;
  sessionTtl: number;

  maxFloodWait: number;
}

const defaultConfig: ActionConfig = {
  deleteUsername: false,
  changeUsername: false,
  usernameMode: "random",
  usernamePrefix: "",
  usernameListPath: "",

  setPhoto: false,
  photoFolderPath: "",
  deleteAllPhotos: false,

  changeName: false,
  nameOnly: false,
  namesFilePath: "",
  surnamesFilePath: "",

  changeBio: false,
  bioFilePath: "",
  bioMode: "random",
  bioSingle: "",
  deleteBio: false,

  setBirthday: false,
  birthdayDayRange: "1-28",
  birthdayMonths: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
  birthdayYearRange: "1990-2003",

  setEmojiAvatar: false,

  resetPassword: false,
  setPassword: false,
  passwordValue: "",

  deleteContacts: false,
  deleteAllDialogs: false,
  deleteBotDialogs: false,
  deleteFolders: false,
  deleteAllStories: false,

  readAllDialogs: false,

  hidePhoneNumber: false,
  hideOnlineStatus: false,

  unsubscribeChannels: false,

  logoutAfter: false,
  deleteAccount: false,

  randomizeOrder: false,
  setAutoPhoto: false,
  delayBetweenMin: 5,
  delayBetweenMax: 7,
  accountTtl: 0,
  sessionTtl: 0,

  maxFloodWait: 0,
};

const MONTH_KEYS = [
  "accountActions.monthJan", "accountActions.monthFeb", "accountActions.monthMar", "accountActions.monthApr",
  "accountActions.monthMay", "accountActions.monthJun", "accountActions.monthJul", "accountActions.monthAug",
  "accountActions.monthSep", "accountActions.monthOct", "accountActions.monthNov", "accountActions.monthDec",
];

const STORAGE_KEY = "account_actions_config";
const IS_DEV = !("__TAURI_INTERNALS__" in window);

// color from "Невалид" status in accounts panel
const DANGER_COLOR = "oklch(0.55 0.1 25)";

function loadSavedConfig(): ActionConfig {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return { ...defaultConfig, ...JSON.parse(saved) };
  } catch {}
  return defaultConfig;
}

export function AccountActionsPage() {
  const t = useT();
  const [config, setConfig] = useState<ActionConfig>(loadSavedConfig);
  const [threads, setThreads] = useState(5);
  const [running, setRunning] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [stats, setStats] = useState({ done: 0, errors: 0, inProgress: 0 });
  const [pickerOpen, setPickerOpen] = useState(false);
  const [deleteWarning, setDeleteWarning] = useState(false);
  const [logoutWarning, setLogoutWarning] = useState(false);
  const [monthsOpen, setMonthsOpen] = useState(false);
  const logsRef = useRef<HTMLDivElement>(null);
  const monthsRef = useRef<HTMLDivElement>(null);

  // save config to localStorage on every change
  useEffect(() => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(config));
  }, [config]);

  useEffect(() => {
    if (IS_DEV) return;
    invoke<{ account_actions_threads?: number }>("get_settings").then((s) => {
      if (s.account_actions_threads) setThreads(s.account_actions_threads);
    }).catch(() => {});
  }, []);

  useEffect(() => {
    if (IS_DEV) return;
    const unlisten = listen<string>("account-actions-log", (e) => {
      const msg = e.payload;

      // internal marker: account processed successfully
      if (msg.startsWith("__DONE__:")) {
        setStats((s) => ({ ...s, done: s.done + 1, inProgress: Math.max(0, s.inProgress - 1) }));
        return;
      }

      setLogs((prev) => [...prev, msg]);

      if (msg === "Завершено" || msg === "Done") {
        setRunning(false);
        setStats((s) => ({ ...s, inProgress: 0 }));
      } else if (msg.includes("ОШИБКА") || msg.includes("ERROR")) {
        setStats((s) => ({ ...s, errors: s.errors + 1, inProgress: Math.max(0, s.inProgress - 1) }));
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

  useEffect(() => {
    if (!monthsOpen) return;
    const handler = (e: MouseEvent) => {
      if (monthsRef.current && !monthsRef.current.contains(e.target as Node)) setMonthsOpen(false);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [monthsOpen]);

  const set = <K extends keyof ActionConfig>(key: K, value: ActionConfig[K]) => {
    setConfig((prev) => {
      const next = { ...prev, [key]: value };
      // delete account resets everything
      if (key === "deleteAccount" && value) {
        return { ...defaultConfig, deleteAccount: true };
      }
      return next;
    });
  };

  const validateConfig = (): string | null => {
    if (config.changeBio) {
      if (config.bioMode === "single" && !config.bioSingle.trim()) return t("accountActions.errBioEmpty");
      if (config.bioMode === "random" && !config.bioFilePath) return t("accountActions.errBioNoFile");
    }
    if (config.setPassword && !config.passwordValue.trim()) return t("accountActions.errNoPassword");
    if (config.changeName && !config.namesFilePath) return t("accountActions.errNoNamesFile");
    if (config.setPhoto && !config.photoFolderPath) return t("accountActions.errNoPhotoFolder");
    if (config.changeUsername && config.usernameMode === "from_list" && !config.usernameListPath) return t("accountActions.errNoUsernameList");
    return null;
  };

  const handleStart = () => {
    const err = validateConfig();
    if (err) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${err}`]);
      return;
    }
    setPickerOpen(true);
  };

  const handleAccountsSelected = (ids: string[]) => {
    if (ids.length === 0) return;
    startExecution(ids);
  };

  const startExecution = async (ids: string[]) => {
    setLogs([]);
    setStats({ done: 0, errors: 0, inProgress: ids.length });
    setRunning(true);
    try {
      const tid = await invoke<string>("account_actions_start", {
        ids,
        config: { ...config, maxFloodWait: Number(config.maxFloodWait) || 0 },
        threads,
      });
      setTaskId(tid);
    } catch (e: any) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${e}`]);
      setRunning(false);
    }
  };

  const handleStop = async () => {
    if (!IS_DEV && taskId) await invoke("account_actions_stop", { taskId }).catch(() => {});
    setRunning(false);
    setLogs((prev) => [...prev, t("common.stoppedByUser")]);
  };

  const selectFile = async (key: keyof ActionConfig) => {
    if (IS_DEV) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open({ multiple: false, filters: [{ name: "Text", extensions: ["txt"] }] });
    if (path) set(key, path as any);
  };

  const selectFolder = async (key: keyof ActionConfig) => {
    if (IS_DEV) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open({ directory: true });
    if (path) set(key, path as any);
  };

  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">{t("descriptions.accountActions")}</p>
      {/* scrollable actions list */}
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div className="p-6">
          <div className="grid grid-cols-2 gap-x-4 gap-y-3 items-start auto-rows-min">

          {/* delete username */}
          <ActionRow checked={config.deleteUsername} onChange={(v) => set("deleteUsername", v)} label={t("accountActions.deleteUsername")} disabled={config.deleteAccount || config.changeUsername} />

          {/* change username */}
          <ActionRow checked={config.changeUsername} onChange={(v) => set("changeUsername", v)} label={t("accountActions.changeUsername")} disabled={config.deleteAccount || config.deleteUsername}>
            <div className="flex items-center gap-2 mt-2">
              <ModeButton active={config.usernameMode === "random"} onClick={() => set("usernameMode", "random")}>{t("accountActions.usernameRandom")}</ModeButton>
              <ModeButton active={config.usernameMode === "from_list"} onClick={() => set("usernameMode", "from_list")}>{t("accountActions.usernameFromList")}</ModeButton>
              <ModeButton active={config.usernameMode === "prefix_random"} onClick={() => set("usernameMode", "prefix_random")}>{t("accountActions.usernamePrefixRandom")}</ModeButton>
            </div>
            {config.usernameMode === "from_list" && (
              <PickerButton onClick={() => selectFile("usernameListPath")} icon="file" label={t("accountActions.usernameListLabel")} value={config.usernameListPath} />
            )}
            {config.usernameMode === "prefix_random" && (
              <input value={config.usernamePrefix} onChange={(e) => set("usernamePrefix", e.target.value)} placeholder={t("accountActions.prefixPlaceholder")} className="mt-2 rounded-md border border-border bg-background px-3 py-1.5 text-sm outline-none focus:border-primary/50 w-48" />
            )}
          </ActionRow>

          {/* set photo */}
          <ActionRow checked={config.setPhoto} onChange={(v) => set("setPhoto", v)} label={t("accountActions.setPhoto")} disabled={config.deleteAccount || config.deleteAllPhotos || config.setEmojiAvatar}>
            <PickerButton onClick={() => selectFolder("photoFolderPath")} icon="folder" label={t("accountActions.photoFolder")} value={config.photoFolderPath} />
          </ActionRow>

          {/* change name */}
          <ActionRow checked={config.changeName} onChange={(v) => set("changeName", v)} label={t("accountActions.changeName")} disabled={config.deleteAccount}>
            <div className="space-y-2 mt-2">
              <label className="flex items-center gap-2 cursor-pointer">
                <input type="checkbox" checked={config.nameOnly} onChange={(e) => set("nameOnly", e.target.checked)} className="rounded border-border accent-primary h-3.5 w-3.5" />
                <span className="text-xs text-muted-foreground font-medium">{t("accountActions.nameOnly")}</span>
              </label>
              <div className="flex flex-wrap items-center gap-2">
                <PickerButton onClick={() => selectFile("namesFilePath")} icon="file" label={t("accountActions.namesFile")} value={config.namesFilePath} />
                {!config.nameOnly && (
                  <PickerButton onClick={() => selectFile("surnamesFilePath")} icon="file" label={t("accountActions.surnamesFile")} value={config.surnamesFilePath} />
                )}
              </div>
            </div>
          </ActionRow>

          {/* change bio */}
          <ActionRow checked={config.changeBio} onChange={(v) => set("changeBio", v)} label={t("accountActions.changeBio")} disabled={config.deleteAccount || config.deleteBio}>
            <div className="flex items-center gap-2 mt-2">
              <ModeButton active={config.bioMode === "random"} onClick={() => set("bioMode", "random")}>{t("accountActions.bioRandom")}</ModeButton>
              <ModeButton active={config.bioMode === "single"} onClick={() => set("bioMode", "single")}>{t("accountActions.bioSingle")}</ModeButton>
            </div>
            {config.bioMode === "random" && (
              <div className="mt-2">
                <PickerButton onClick={() => selectFile("bioFilePath")} icon="file" label={t("accountActions.bioFile")} value={config.bioFilePath} />
              </div>
            )}
            {config.bioMode === "single" && (
              <input value={config.bioSingle} onChange={(e) => set("bioSingle", e.target.value)} placeholder={t("accountActions.bioPlaceholder")} className="mt-2 rounded-md border border-border bg-background px-3 py-1.5 text-sm outline-none focus:border-primary/50 w-full max-w-md" />
            )}
          </ActionRow>

          {/* delete bio */}
          <ActionRow checked={config.deleteBio} onChange={(v) => set("deleteBio", v)} label={t("accountActions.deleteBio")} disabled={config.deleteAccount || config.changeBio} />

          {/* delete all photos */}
          <ActionRow checked={config.deleteAllPhotos} onChange={(v) => set("deleteAllPhotos", v)} label={t("accountActions.deleteAllPhotos")} disabled={config.deleteAccount || config.setPhoto || config.setEmojiAvatar} />

          {/* emoji avatar */}
          <ActionRow checked={config.setEmojiAvatar} onChange={(v) => set("setEmojiAvatar", v)} label={t("accountActions.emojiAvatar")} disabled={config.deleteAccount || config.setPhoto || config.deleteAllPhotos} />

          {/* birthday */}
          <ActionRow checked={config.setBirthday} onChange={(v) => set("setBirthday", v)} label={t("accountActions.setBirthday")} disabled={config.deleteAccount}>
            <div className="flex items-center gap-3 mt-2">
              <input value={config.birthdayDayRange} onChange={(e) => set("birthdayDayRange", e.target.value)} placeholder="1-28" className="rounded-md border border-border bg-background px-2.5 py-1.5 text-xs outline-none focus:border-primary/50 w-20" />
              <div className="relative" ref={monthsRef}>
                <button onClick={() => setMonthsOpen(!monthsOpen)} className="rounded-md border border-border bg-background px-2.5 py-1.5 text-xs hover:border-primary/50 transition">
                  {t("accountActions.months")} ({config.birthdayMonths.length})
                </button>
                {monthsOpen && (
                  <div className="absolute top-full left-0 mt-1 z-50 w-40 max-h-48 overflow-y-auto scrollbar-thin rounded-md border border-border bg-card shadow-lg py-1">
                    {MONTH_KEYS.map((mk, i) => (
                      <label key={i} className="flex items-center gap-2 px-3 py-1 text-xs cursor-pointer hover:bg-accent/50">
                        <input type="checkbox" checked={config.birthdayMonths.includes(i + 1)} onChange={(e) => {
                          const months = e.target.checked ? [...config.birthdayMonths, i + 1] : config.birthdayMonths.filter((x) => x !== i + 1);
                          set("birthdayMonths", months.sort((a, b) => a - b));
                        }} className="rounded border-border" />
                        {t(mk)}
                      </label>
                    ))}
                  </div>
                )}
              </div>
              <input value={config.birthdayYearRange} onChange={(e) => set("birthdayYearRange", e.target.value)} placeholder="1990-2003" className="rounded-md border border-border bg-background px-2.5 py-1.5 text-xs outline-none focus:border-primary/50 w-28" />
            </div>
          </ActionRow>

          {/* reset 2fa */}
          <ActionRow checked={config.resetPassword} onChange={(v) => set("resetPassword", v)} label={t("accountActions.resetPassword")} disabled={config.deleteAccount || config.setPassword} />

          {/* set 2fa */}
          <ActionRow checked={config.setPassword} onChange={(v) => set("setPassword", v)} label={t("accountActions.setPassword")} disabled={config.deleteAccount || config.resetPassword}>
            <input value={config.passwordValue} onChange={(e) => set("passwordValue", e.target.value)} placeholder={t("accountActions.passwordPlaceholder")} type="password" className="mt-2 rounded-md border border-border bg-background px-3 py-1.5 text-sm outline-none focus:border-primary/50 w-56" />
          </ActionRow>

          {/* delete contacts */}
          <ActionRow checked={config.deleteContacts} onChange={(v) => set("deleteContacts", v)} label={t("accountActions.deleteContacts")} disabled={config.deleteAccount} />

          {/* delete all dialogs */}
          <ActionRow checked={config.deleteAllDialogs} onChange={(v) => set("deleteAllDialogs", v)} label={t("accountActions.deleteAllDialogs")} disabled={config.deleteAccount || config.deleteBotDialogs} />

          {/* delete bot dialogs */}
          <ActionRow checked={config.deleteBotDialogs} onChange={(v) => set("deleteBotDialogs", v)} label={t("accountActions.deleteBotDialogs")} disabled={config.deleteAccount || config.deleteAllDialogs} />

          {/* read all dialogs */}
          <ActionRow checked={config.readAllDialogs} onChange={(v) => set("readAllDialogs", v)} label={t("accountActions.readAllDialogs")} disabled={config.deleteAccount} />

          {/* hide phone number */}
          <ActionRow checked={config.hidePhoneNumber} onChange={(v) => set("hidePhoneNumber", v)} label={t("accountActions.hidePhoneNumber")} disabled={config.deleteAccount} />

          {/* hide online status */}
          <ActionRow checked={config.hideOnlineStatus} onChange={(v) => set("hideOnlineStatus", v)} label={t("accountActions.hideOnlineStatus")} disabled={config.deleteAccount} />

          {/* delete folders */}
          <ActionRow checked={config.deleteFolders} onChange={(v) => set("deleteFolders", v)} label={t("accountActions.deleteFolders")} disabled={config.deleteAccount} />

          {/* unsubscribe from channels */}
          <ActionRow checked={config.unsubscribeChannels} onChange={(v) => set("unsubscribeChannels", v)} label={t("accountActions.unsubscribeChannels")} disabled={config.deleteAccount} />

          {/* delete all stories */}
          <ActionRow checked={config.deleteAllStories} onChange={(v) => set("deleteAllStories", v)} label={t("accountActions.deleteAllStories")} disabled={config.deleteAccount} />

          {/* account TTL */}
          <div className="col-span-2 flex items-center gap-3 py-1">
            <label className="text-sm font-medium text-foreground">{t("accountActions.accountTtlLabel")}</label>
            <select value={config.accountTtl} onChange={(e) => set("accountTtl", Number(e.target.value) as any)} className="rounded-md border border-border bg-card px-2.5 py-1.5 text-sm outline-none focus:border-primary/50">
              <option value={0}>{t("accountActions.ttlNoChange")}</option>
              <option value={30}>{t("accountActions.ttl1Month")}</option>
              <option value={90}>{t("accountActions.ttl3Months")}</option>
              <option value={182}>{t("accountActions.ttl6Months")}</option>
              <option value={365}>{t("accountActions.ttl1Year")}</option>
              <option value={548}>{t("accountActions.ttl1_5Years")}</option>
              <option value={730}>{t("accountActions.ttl2Years")}</option>
            </select>
            <span className="text-xs text-muted-foreground">{t("accountActions.accountTtlHint")}</span>
          </div>

          {/* session TTL */}
          <div className="col-span-2 flex items-center gap-3 py-1">
            <label className="text-sm font-medium text-foreground">{t("accountActions.sessionTtlLabel")}</label>
            <select value={config.sessionTtl} onChange={(e) => set("sessionTtl", Number(e.target.value) as any)} className="rounded-md border border-border bg-card px-2.5 py-1.5 text-sm outline-none focus:border-primary/50">
              <option value={0}>{t("accountActions.ttlNoChange")}</option>
              <option value={7}>{t("accountActions.ttl7Days")}</option>
              <option value={30}>{t("accountActions.ttl1Month")}</option>
              <option value={90}>{t("accountActions.ttl3Months")}</option>
              <option value={183}>{t("accountActions.ttl6Months")}</option>
              <option value={365}>{t("accountActions.ttl1Year")}</option>
            </select>
            <span className="text-xs text-muted-foreground">{t("accountActions.sessionTtlHint")}</span>
          </div>

          {/* delay between actions */}
          <div className="col-span-2 flex items-center gap-3 py-1">
            <label className="text-sm font-medium text-foreground">{t("accountActions.delayBetweenLabel")}</label>
            <input type="number" min={1} max={300} value={config.delayBetweenMin} onChange={(e) => set("delayBetweenMin", Math.max(1, Number(e.target.value)) as any)} className="w-16 rounded-md border border-border bg-background px-2 py-1 text-sm outline-none focus:border-primary/50" />
            <span className="text-muted-foreground text-sm">—</span>
            <input type="number" min={1} max={300} value={config.delayBetweenMax} onChange={(e) => set("delayBetweenMax", Math.max(1, Number(e.target.value)) as any)} className="w-16 rounded-md border border-border bg-background px-2 py-1 text-sm outline-none focus:border-primary/50" />
          </div>

          {/* randomize order */}
          <ActionRow checked={config.randomizeOrder} onChange={(v) => set("randomizeOrder", v)} label={t("accountActions.randomizeOrder")} disabled={config.deleteAccount} />

          {/* logout after */}
          <ActionRow
            checked={config.logoutAfter}
            onChange={(v) => { if (v) setLogoutWarning(true); else set("logoutAfter", false); }}
            label={t("accountActions.logoutAfter")}
            destructive
            disabled={config.deleteAccount}
          />

          {/* delete account — last */}
          <ActionRow
            checked={config.deleteAccount}
            onChange={(v) => { if (v) setDeleteWarning(true); else set("deleteAccount", false); }}
            label={t("accountActions.deleteAccount")}
            destructive
          />
          </div>
        </div>
      </div>

      {/* controls */}
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-6 text-sm font-semibold">
          <span>{t("accountActions.statsDone")} <span className="text-[oklch(0.65_0.1_150)]">{stats.done}</span></span>
          <span>{t("accountActions.statsError")} <span className="text-[oklch(0.55_0.1_25)]">{stats.errors}</span></span>
          <span>{t("accountActions.statsInProgress")} <span className="text-[oklch(0.65_0.1_280)]">{stats.inProgress}</span></span>
        </div>
        <div className="ml-auto flex items-center gap-3">
          <span className="max-w-[220px] text-right text-xs text-muted-foreground">{t("common.startSelectAccountsHint")}</span>
          <button
            onClick={handleStart}
            disabled={running}
            className="flex items-center gap-2 rounded-md border border-primary/50 bg-primary/10 px-4 py-2 text-sm text-primary font-medium hover:bg-primary/20 transition disabled:opacity-50"
          >
            <Play className="h-4 w-4" />
            {t("common.start")}
          </button>
          <button
            onClick={handleStop}
            disabled={!running}
            className="flex items-center gap-2 rounded-md border px-4 py-2 text-sm font-medium transition disabled:opacity-50"
            style={{ borderColor: `color-mix(in oklch, ${DANGER_COLOR} 50%, transparent)`, color: DANGER_COLOR, background: `color-mix(in oklch, ${DANGER_COLOR} 6%, transparent)` }}
          >
            <Square className="h-4 w-4" />
            {t("common.stop")}
          </button>
          <div className="flex items-center gap-2">
            <span className="text-xs text-muted-foreground">{t("accountActions.floodWaitLabel")}</span>
            <input
              type="number"
              min={0}
              max={3600}
              value={config.maxFloodWait}
              onChange={(e) => set("maxFloodWait", Number(e.target.value) as any)}
              className="w-16 rounded-md border border-border bg-background px-2 py-1 text-xs text-center outline-none focus:border-primary/50"
            />
          </div>
          <div className="flex items-center gap-2">
            <span className="text-xs text-muted-foreground">{t("accountActions.threadsLabel")}</span>
            <ThreadInput
              value={threads}
              onChange={(v) => {
                setThreads(v);
                if (!IS_DEV) invoke("patch_settings", { patch: { account_actions_threads: v } }).catch(() => {});
              }}
              min={1}
              max={1000}
            />
          </div>
        </div>
      </div>

      {/* logs */}
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div ref={logsRef} className="h-40 overflow-y-auto scrollbar-thin p-4 font-mono text-xs space-y-0.5">
          {logs.length === 0 && (
            <div className="text-muted-foreground text-center py-8">{t("common.logsPlaceholder")}</div>
          )}
          {logs.map((line, i) => (
            <div key={i} className="text-muted-foreground whitespace-pre-wrap break-all">{line}</div>
          ))}
        </div>
      </div>

      {/* account picker modal */}
      <AccountPickerModal open={pickerOpen} onClose={() => setPickerOpen(false)} onSelect={handleAccountsSelected} />

      {/* delete account warning */}
      {deleteWarning && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div className="rounded-xl border bg-card p-6 w-96 shadow-2xl" style={{ borderColor: `color-mix(in oklch, ${DANGER_COLOR} 30%, transparent)` }}>
            <div className="flex items-center gap-3 mb-3">
              <AlertTriangle className="h-5 w-5" style={{ color: DANGER_COLOR }} />
              <h3 className="text-lg font-semibold">{t("accountActions.deleteAccountWarningTitle")}</h3>
            </div>
            <p className="text-sm text-muted-foreground mb-4">
              {t("accountActions.deleteAccountWarningText")}
            </p>
            <div className="flex gap-2">
              <button onClick={() => setDeleteWarning(false)} className="flex-1 rounded-md border border-border px-3 py-2 text-sm hover:bg-accent/50 transition">{t("common.cancel")}</button>
              <button onClick={() => { set("deleteAccount", true); setDeleteWarning(false); }} className="flex-1 rounded-md border px-3 py-2 text-sm font-medium transition" style={{ borderColor: `color-mix(in oklch, ${DANGER_COLOR} 50%, transparent)`, color: DANGER_COLOR, background: `color-mix(in oklch, ${DANGER_COLOR} 10%, transparent)` }}>{t("common.confirm")}</button>
            </div>
          </div>
        </div>
      )}

      {/* logout warning */}
      {logoutWarning && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div className="rounded-xl border bg-card p-6 w-96 shadow-2xl" style={{ borderColor: `color-mix(in oklch, ${DANGER_COLOR} 30%, transparent)` }}>
            <div className="flex items-center gap-3 mb-3">
              <AlertTriangle className="h-5 w-5" style={{ color: DANGER_COLOR }} />
              <h3 className="text-lg font-semibold">{t("accountActions.logoutWarningTitle")}</h3>
            </div>
            <p className="text-sm text-muted-foreground mb-4">
              {t("accountActions.logoutWarningText")}
            </p>
            <div className="flex gap-2">
              <button onClick={() => setLogoutWarning(false)} className="flex-1 rounded-md border border-border px-3 py-2 text-sm hover:bg-accent/50 transition">{t("common.cancel")}</button>
              <button onClick={() => { set("logoutAfter", true); setLogoutWarning(false); }} className="flex-1 rounded-md border px-3 py-2 text-sm font-medium transition" style={{ borderColor: `color-mix(in oklch, ${DANGER_COLOR} 50%, transparent)`, color: DANGER_COLOR, background: `color-mix(in oklch, ${DANGER_COLOR} 10%, transparent)` }}>{t("accountActions.logoutWarningConfirm")}</button>
            </div>
          </div>
        </div>
      )}

    </div>
  );
}

function ActionRow({ checked, onChange, label, children, disabled, destructive }: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
  children?: React.ReactNode;
  disabled?: boolean;
  destructive?: boolean;
}) {
  const activeStyle: React.CSSProperties | undefined = checked
    ? destructive
      ? { borderColor: `color-mix(in oklch, ${DANGER_COLOR} 50%, transparent)`, background: `color-mix(in oklch, ${DANGER_COLOR} 10%, transparent)`, color: DANGER_COLOR }
      : undefined
    : undefined;

  const base = "rounded-lg border px-3.5 py-3 text-sm font-medium transition select-none flex items-start gap-2.5 text-left w-full";
  const stateClass = checked
    ? destructive
      ? ""
      : "border-primary/50 bg-primary/10 text-primary"
    : "border-border bg-background text-foreground hover:border-primary/30";
  const disabledClass = disabled ? "opacity-40 cursor-not-allowed" : "cursor-pointer";

  return (
    <div className={`rounded-lg border ${checked ? "border-transparent" : "border-transparent"}`}>
      <button
        type="button"
        disabled={disabled}
        onClick={() => { if (!disabled) onChange(!checked); }}
        className={`${base} ${stateClass} ${disabledClass}`}
        style={activeStyle}
      >
        <span className={`mt-0.5 flex h-3 w-3 shrink-0 items-center justify-center rounded-sm border transition ${checked ? (destructive ? "border-current" : "border-primary bg-primary text-primary-foreground") : "border-muted-foreground/40"}`} style={checked && destructive ? { background: `color-mix(in oklch, ${DANGER_COLOR} 20%, transparent)` } : undefined}>
          {checked && <Check className="h-2.5 w-2.5" strokeWidth={3} />}
        </span>
        <span className="leading-snug">{label}</span>
      </button>
      {checked && children && <div className="mt-2 ml-1 pl-6">{children}</div>}
    </div>
  );
}

// file/folder picker button matching the username checker / mailing style
function PickerButton({ onClick, icon, label, value }: { onClick: () => void; icon: "file" | "folder"; label: string; value?: string }) {
  const t = useT();
  const Icon = icon === "folder" ? FolderOpen : FileText;
  return (
    <div className="flex items-center gap-2">
      <button onClick={onClick} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-1.5 text-xs font-medium hover:border-primary/50 transition shrink-0">
        <Icon className="h-3.5 w-3.5" /> {label}
      </button>
      <span className="text-xs text-muted-foreground truncate max-w-[180px]">{value ? value.split(/[/\\]/).pop() : t("common.notSelected")}</span>
    </div>
  );
}

function ModeButton({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button onClick={onClick} className={`rounded-md border px-3 py-1 text-xs font-medium transition ${active ? "border-primary/50 bg-primary/10 text-primary" : "border-border bg-background text-muted-foreground hover:border-primary/30"}`}>
      {children}
    </button>
  );
}
