import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { FolderOpen, Play, RotateCcw, Square } from "lucide-react";
import { AccountPickerModal } from "@/components/AccountPickerModal";
import { ThreadInput } from "@/components/ThreadInput";
import { useT } from "@/i18n";

interface Config {
  outputPath: string;
  regenerateTokens: boolean;
  threads: number;
  maxFloodWait: number;
}

const IS_DEV = !("__TAURI_INTERNALS__" in window);

export function BotParserPage() {
  const t = useT();
  const [config, setConfig] = useState<Config>({
    outputPath: "",
    regenerateTokens: false,
    threads: 3,
    maxFloodWait: 0,
  });
  const [running, setRunning] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [logs, setLogs] = useState<string[]>([]);
  const [stats, setStats] = useState({ done: 0, errors: 0, inProgress: 0 });
  const logsRef = useRef<HTMLDivElement>(null);
  const stopRequestedRef = useRef(false);

  useEffect(() => {
    if (IS_DEV) return;
    const unlisten = listen<string>("bot-parser-log", (e) => {
      const msg = e.payload;
      if (msg.startsWith("__DONE__:")) {
        setStats((s) => ({ ...s, done: s.done + 1, inProgress: Math.max(0, s.inProgress - 1) }));
        return;
      }
      if (msg.startsWith("__ERROR__:")) {
        setStats((s) => ({ ...s, errors: s.errors + 1, inProgress: Math.max(0, s.inProgress - 1) }));
        return;
      }
      if (msg === "__FINISHED__") {
        setRunning(false);
        setTaskId(null);
        stopRequestedRef.current = false;
        setStats((s) => ({ ...s, inProgress: 0 }));
        return;
      }

      setLogs((prev) => [...prev, msg]);
      if (msg === t("common.done") || msg === "Done") {
        setRunning(false);
        setTaskId(null);
        stopRequestedRef.current = false;
        setStats((s) => ({ ...s, inProgress: 0 }));
      }
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [t]);

  useEffect(() => {
    const el = logsRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    if (atBottom) el.scrollTop = el.scrollHeight;
  }, [logs]);

  const set = <K extends keyof Config>(key: K, value: Config[K]) => {
    setConfig((prev) => ({ ...prev, [key]: value }));
  };

  const selectOutputFile = async () => {
    if (IS_DEV) return;
    const { save } = await import("@tauri-apps/plugin-dialog");
    const path = await save({ defaultPath: "bot_tokens.db", filters: [{ name: "SQLite DB", extensions: ["db"] }] });
    if (path) set("outputPath", path);
  };

  const handleStart = () => {
    setPickerOpen(true);
  };

  const startWithAccounts = async (ids: string[]) => {
    if (ids.length === 0) return;
    setLogs([]);
    setStats({ done: 0, errors: 0, inProgress: ids.length });
    setRunning(true);
    setTaskId(null);
    stopRequestedRef.current = false;
    try {
      const tid = await invoke<string>("bot_parser_start", {
        ids,
        config: {
          outputPath: config.outputPath.trim(),
          regenerateTokens: config.regenerateTokens,
          threads: config.threads,
          maxFloodWait: Number(config.maxFloodWait) || 0,
        },
      });
      if (stopRequestedRef.current) {
        await invoke("bot_parser_stop", { taskId: tid }).catch(() => {});
        setTaskId(null);
      } else {
        setTaskId(tid);
      }
    } catch (e: any) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${e}`]);
      setRunning(false);
      setStats((s) => ({ ...s, inProgress: 0 }));
      stopRequestedRef.current = false;
    }
  };

  const handleStop = async () => {
    stopRequestedRef.current = true;
    if (!IS_DEV && taskId) await invoke("bot_parser_stop", { taskId }).catch(() => {});
    setRunning(false);
    setTaskId(null);
    setStats((s) => ({ ...s, inProgress: 0 }));
    setLogs((prev) => [...prev, t("common.stoppedByUser")]);
  };

  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">{t("descriptions.botParser")}</p>

      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div className="p-6 space-y-5">
          <div>
            <label className="text-sm font-medium text-foreground">{t("botParser.outputFile")}</label>
            <p className="text-xs text-muted-foreground mb-2">{t("botParser.outputFileHint")}</p>
            <div className="flex items-center gap-2">
              <button onClick={selectOutputFile} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition">
                <FolderOpen className="h-4 w-4" />
                {t("common.selectFile")}
              </button>
              <span className="text-xs text-muted-foreground truncate max-w-sm">
                {config.outputPath ? config.outputPath.split(/[/\\]/).pop() : "bot_tokens.db"}
              </span>
            </div>
          </div>

          <div className="grid gap-4 md:grid-cols-2">
            <label className="flex items-start gap-3 rounded-lg border border-border bg-background/35 p-4 cursor-pointer">
              <input
                type="checkbox"
                checked={config.regenerateTokens}
                onChange={(e) => set("regenerateTokens", e.target.checked)}
                className="mt-0.5 h-4 w-4 rounded border-border accent-primary"
              />
              <span>
                <span className="flex items-center gap-2 text-sm font-semibold text-foreground">
                  <RotateCcw className="h-4 w-4 text-primary" />
                  {t("botParser.regenerateTokens")}
                </span>
                <span className="mt-1 block text-xs text-muted-foreground">{t("botParser.regenerateTokensHint")}</span>
              </span>
            </label>

            <div className="rounded-lg border border-border bg-background/35 p-4">
              <div className="flex items-center justify-between gap-4">
                <div>
                  <div className="text-sm font-semibold text-foreground">{t("common.threads")}</div>
                  <div className="mt-1 text-xs text-muted-foreground">{t("botParser.threadsHint")}</div>
                </div>
                <ThreadInput value={config.threads} onChange={(v) => set("threads", v)} min={1} max={20} />
              </div>
            </div>
          </div>

          <div className="max-w-xs">
            <label className="text-sm font-medium text-foreground">{t("botParser.maxFloodWait")}</label>
            <input
              type="number"
              min={0}
              value={config.maxFloodWait}
              onChange={(e) => set("maxFloodWait", Number(e.target.value) || 0)}
              className="mt-1.5 w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50"
            />
            <p className="mt-1 text-xs text-muted-foreground">{t("botParser.maxFloodWaitHint")}</p>
          </div>
        </div>
      </div>

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
            style={{ borderColor: "color-mix(in oklch, oklch(0.55 0.1 25) 50%, transparent)", color: "oklch(0.55 0.1 25)", background: "color-mix(in oklch, oklch(0.55 0.1 25) 6%, transparent)" }}
          >
            <Square className="h-4 w-4" />
            {t("common.stop")}
          </button>
        </div>
      </div>

      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div ref={logsRef} className="h-48 overflow-y-auto scrollbar-thin p-4 font-mono text-xs space-y-0.5">
          {logs.length === 0 && <div className="text-muted-foreground text-center py-8">{t("common.logsPlaceholder")}</div>}
          {logs.map((line, i) => (
            <div key={i} className="text-muted-foreground whitespace-pre-wrap break-all">{line}</div>
          ))}
        </div>
      </div>

      <AccountPickerModal
        open={pickerOpen}
        onClose={() => setPickerOpen(false)}
        onSelect={startWithAccounts}
        title={t("common.selectAccounts")}
      />
    </div>
  );
}
