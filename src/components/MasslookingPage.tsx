import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Play, Square, FolderOpen } from "lucide-react";
import { AccountPickerModal } from "@/components/AccountPickerModal";
import { ThreadInput } from "@/components/ThreadInput";
import { useT } from "@/i18n";

type SourceMode = "usernames" | "inbox" | "chat";
type ReplyTextMode = "none" | "llm_rewrite" | "randomize";

const REACTIONS = ["👍", "❤️", "🔥", "👏", "😍", "🎉", "💯", "😢", "🤔", "👎"];

interface MasslookingConfig {
  sourceMode: SourceMode;
  usernamesPath: string;
  chatTarget: string;

  reactAfterView: boolean;
  reaction: string;

  replyToStory: boolean;
  replyTextMode: ReplyTextMode;
  replyText: string;

  maxFloodWait: number;
  maxPerAccount: number;
  threads: number;
}

const defaultConfig: MasslookingConfig = {
  sourceMode: "usernames",
  usernamesPath: "",
  chatTarget: "",

  reactAfterView: false,
  reaction: "👍",

  replyToStory: false,
  replyTextMode: "none",
  replyText: "",

  maxFloodWait: 60,
  maxPerAccount: 100,
  threads: 5,
};

const STORAGE_KEY = "masslooking_config";
const IS_DEV = !("__TAURI_INTERNALS__" in window);

function loadSavedConfig(): MasslookingConfig {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return { ...defaultConfig, ...JSON.parse(saved) };
  } catch {}
  return defaultConfig;
}

export function MasslookingPage() {
  const t = useT();
  const [config, setConfig] = useState<MasslookingConfig>(loadSavedConfig);
  const [running, setRunning] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [stats, setStats] = useState({ viewed: 0, reacted: 0, replied: 0, errors: 0 });
  const [pickerOpen, setPickerOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const logsRef = useRef<HTMLDivElement>(null);

  useEffect(() => { localStorage.setItem(STORAGE_KEY, JSON.stringify(config)); }, [config]);

  useEffect(() => {
    if (IS_DEV) return;
    const unlisten = listen<string>("masslooking-log", (e) => {
      setLogs((prev) => [...prev, e.payload]);
      const msg = e.payload;
      if (msg === "Завершено" || msg === "Done") setRunning(false);
      else if (msg.includes("просмотрено") || msg.includes("viewed")) setStats((s) => ({ ...s, viewed: s.viewed + 1 }));
      else if (msg.includes("реакция") || msg.includes("reacted")) setStats((s) => ({ ...s, reacted: s.reacted + 1 }));
      else if (msg.includes("ответ отправлен") || msg.includes("replied")) setStats((s) => ({ ...s, replied: s.replied + 1 }));
      else if (msg.includes("ОШИБКА") || msg.includes("ошибка") || msg.includes("ERROR") || msg.includes("error")) setStats((s) => ({ ...s, errors: s.errors + 1 }));
    });
    return () => { unlisten.then((f) => f()); };
  }, []);

  useEffect(() => {
    const el = logsRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    if (atBottom) el.scrollTop = el.scrollHeight;
  }, [logs]);

  const set = <K extends keyof MasslookingConfig>(key: K, value: MasslookingConfig[K]) => {
    setConfig((prev) => ({ ...prev, [key]: value }));
    setError(null);
  };

  const selectUsernames = async () => {
    if (IS_DEV) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open({ multiple: false, filters: [{ name: "Text", extensions: ["txt"] }] });
    if (path) set("usernamesPath", path as string);
  };

  const validate = (): string | null => {
    if (config.sourceMode === "usernames" && !config.usernamesPath) return t("masslooking.errSelectFile");
    if (config.sourceMode === "chat" && !config.chatTarget.trim()) return t("masslooking.errSpecifyChat");
    if (config.replyToStory && !config.replyText.trim()) return t("masslooking.errReplyEmpty");
    return null;
  };

  const handleStart = () => {
    const err = validate();
    if (err) { setError(err); return; }
    setPickerOpen(true);
  };

  const handleAccountsSelected = async (ids: string[]) => {
    if (ids.length === 0) return;
    setLogs([]); setStats({ viewed: 0, reacted: 0, replied: 0, errors: 0 }); setRunning(true); setError(null);
    try {
      const tid = await invoke<string>("masslooking_start", {
        ids,
        config: {
          source_mode: config.sourceMode,
          usernames_path: config.sourceMode === "usernames" ? config.usernamesPath : "",
          chat_target: config.sourceMode === "chat" ? config.chatTarget.trim() : "",
          react_after_view: config.reactAfterView,
          reaction: config.reaction,
          reply_to_story: config.replyToStory,
          reply_text_mode: config.replyTextMode,
          reply_text: config.replyText.trim(),
          max_flood_wait: config.maxFloodWait,
          max_per_account: config.maxPerAccount,
        },
        threads: config.threads,
      });
      setTaskId(tid);
    } catch (e: any) { setLogs((prev) => [...prev, `${t("common.error")}: ${e}`]); setRunning(false); }
  };

  const handleStop = async () => {
    if (!IS_DEV && taskId) await invoke("masslooking_stop", { taskId }).catch(() => {});
    setRunning(false); setLogs((prev) => [...prev, t("common.stoppedByUser")]);
  };

  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">{t("descriptions.masslooking")}</p>
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div className="p-6 space-y-5">

          {/* source mode */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("masslooking.sourceLabel")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <ModeButton active={config.sourceMode === "usernames"} onClick={() => set("sourceMode", "usernames")}>{t("masslooking.sourceUsernames")}</ModeButton>
              <ModeButton active={config.sourceMode === "inbox"} onClick={() => set("sourceMode", "inbox")}>{t("masslooking.sourceInbox")}</ModeButton>
              <ModeButton active={config.sourceMode === "chat"} onClick={() => set("sourceMode", "chat")}>{t("masslooking.sourceChat")}</ModeButton>
            </div>
          </div>

          {config.sourceMode === "usernames" && (
            <div className="flex items-center gap-2">
              <button onClick={selectUsernames} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition">
                <FolderOpen className="h-4 w-4" /> {t("common.selectFile")}
              </button>
              <span className="text-xs text-muted-foreground truncate max-w-sm">{config.usernamesPath ? config.usernamesPath.split(/[/\\]/).pop() : t("common.notSelected")}</span>
            </div>
          )}

          {config.sourceMode === "inbox" && (
            <p className="text-xs text-muted-foreground">{t("masslooking.inboxHint")}</p>
          )}

          {config.sourceMode === "chat" && (
            <div>
              <input value={config.chatTarget} onChange={(e) => set("chatTarget", e.target.value)} placeholder="https://t.me/chat / @username" className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50" />
              <p className="text-xs text-muted-foreground mt-1">{t("masslooking.chatHint")}</p>
            </div>
          )}

          {/* react after view */}
          <div>
            <label className="flex items-center gap-3 cursor-pointer">
              <input type="checkbox" checked={config.reactAfterView} onChange={(e) => set("reactAfterView", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
              <span className="text-sm font-medium text-foreground">{t("masslooking.reactAfterView")}</span>
            </label>
            {config.reactAfterView && (
              <div className="ml-7 mt-2 flex flex-wrap gap-1.5">
                {REACTIONS.map((r) => (
                  <button key={r} onClick={() => set("reaction", r)} className={`px-2.5 py-1 rounded-md border text-sm transition ${config.reaction === r ? "border-primary/50 bg-primary/10" : "border-border bg-background hover:border-primary/30"}`}>{r}</button>
                ))}
              </div>
            )}
          </div>

          {/* reply to story */}
          <div>
            <label className="flex items-center gap-3 cursor-pointer">
              <input type="checkbox" checked={config.replyToStory} onChange={(e) => set("replyToStory", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
              <span className="text-sm font-medium text-foreground">{t("masslooking.replyToStory")}</span>
            </label>
            {config.replyToStory && (
              <div className="ml-7 mt-2 space-y-3">
                <div className="flex items-center gap-2">
                  <ModeButton active={config.replyTextMode === "none"} onClick={() => set("replyTextMode", "none")}>{t("masslooking.replyNone")}</ModeButton>
                  <ModeButton active={config.replyTextMode === "llm_rewrite"} onClick={() => set("replyTextMode", "llm_rewrite")}>{t("masslooking.replyLlm")}</ModeButton>
                  <ModeButton active={config.replyTextMode === "randomize"} onClick={() => set("replyTextMode", "randomize")}>{t("masslooking.replyRandomize")}</ModeButton>
                </div>
                <textarea value={config.replyText} onChange={(e) => set("replyText", e.target.value)} placeholder={t("masslooking.replyPlaceholder")} rows={3} className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50 resize-y" />
                {config.replyTextMode === "llm_rewrite" && <p className="text-xs text-muted-foreground">{t("masslooking.replyLlmHint")}</p>}
                {config.replyTextMode === "randomize" && <p className="text-xs text-muted-foreground">{t("masslooking.replyRandomizeHint")}</p>}
              </div>
            )}
          </div>

          {/* limits */}
          <div className="border-t border-border pt-4 space-y-4">
            <div>
              <label className="text-sm font-medium text-foreground">{t("masslooking.maxPerAccount")}</label>
              <input type="number" min={1} max={99999} value={config.maxPerAccount} onChange={(e) => set("maxPerAccount", Math.max(1, Number(e.target.value)))} className="mt-1.5 w-28 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
            </div>
            <div>
              <label className="text-sm font-medium text-foreground">{t("common.maxFloodWait")}</label>
              <div className="flex items-center gap-2 mt-1.5">
                <input type="number" min={0} max={86400} value={config.maxFloodWait} onChange={(e) => set("maxFloodWait", Math.max(0, Math.min(86400, Number(e.target.value))))} className="w-28 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
                <span className="text-xs text-muted-foreground">0 = ∞</span>
              </div>
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
          <span>{t("masslooking.viewed")}: <span className="text-[oklch(0.65_0.1_150)]">{stats.viewed}</span></span>
          <span>{t("masslooking.reacted")}: <span className="text-[oklch(0.65_0.1_60)]">{stats.reacted}</span></span>
          <span>{t("masslooking.replied")}: <span className="text-[oklch(0.65_0.1_280)]">{stats.replied}</span></span>
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
