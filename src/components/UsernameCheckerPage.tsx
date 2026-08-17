import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Play, Square, FolderOpen } from "lucide-react";
import { AccountPickerModal } from "@/components/AccountPickerModal";
import { useT } from "@/i18n";

interface Config {
  inputPath: string;
  outputPath: string;
  autoClaim: boolean;
}

const IS_DEV = !("__TAURI_INTERNALS__" in window);

export function UsernameCheckerPage() {
  const t = useT();
  const [config, setConfig] = useState<Config>({ inputPath: "", outputPath: "", autoClaim: false });
  const [running, setRunning] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [stats, setStats] = useState({ free: 0, taken: 0, fragment: 0 });
  const [pickerOpen, setPickerOpen] = useState(false);
  const [, setSelectedIds] = useState<string[]>([]);
  const logsRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (IS_DEV) return;
    const unlisten = listen<string>("username-checker-log", (e) => {
      setLogs((prev) => [...prev, e.payload]);
      const msg = e.payload;
      if (msg === "Завершено" || msg === "Done") {
        setRunning(false);
      } else if (msg.startsWith("[") && (msg.includes("— свободен") || msg.includes("— free"))) {
        setStats((s) => ({ ...s, free: s.free + 1 }));
      } else if (msg.startsWith("[") && (msg.includes("— продаётся") || msg.includes("— продан") || msg.includes("— for sale") || msg.includes("— sold"))) {
        setStats((s) => ({ ...s, fragment: s.fragment + 1 }));
      } else if (msg.startsWith("[") && (msg.includes("— занят") || msg.includes("— taken"))) {
        setStats((s) => ({ ...s, taken: s.taken + 1 }));
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

  const set = <K extends keyof Config>(key: K, value: Config[K]) => {
    setConfig((prev) => ({ ...prev, [key]: value }));
  };

  const selectInputFile = async () => {
    if (IS_DEV) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open({ multiple: false, filters: [{ name: "Text", extensions: ["txt"] }] });
    if (path) set("inputPath", path as string);
  };

  const selectOutputFile = async () => {
    if (IS_DEV) return;
    const { save } = await import("@tauri-apps/plugin-dialog");
    const path = await save({ defaultPath: "username_check.db", filters: [{ name: "SQLite DB", extensions: ["db"] }] });
    if (path) set("outputPath", path);
  };

  const handleStart = () => {
    if (!config.inputPath.trim()) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${t("usernameChecker.inputFile")}`]);
      return;
    }
    if (config.autoClaim) {
      setPickerOpen(true);
    } else {
      doStart([]);
    }
  };

  const handleAccountSelected = (ids: string[]) => {
    setSelectedIds(ids);
    doStart(ids);
  };

  const doStart = async (ids: string[]) => {
    setLogs([]);
    setStats({ free: 0, taken: 0, fragment: 0 });
    setRunning(true);
    try {
      const tid = await invoke<string>("username_checker_start", {
        ids,
        config: {
          input_path: config.inputPath.trim(),
          output_path: config.outputPath.trim(),
          auto_claim: config.autoClaim,
        },
      });
      setTaskId(tid);
    } catch (e: any) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${e}`]);
      setRunning(false);
    }
  };

  const handleStop = async () => {
    if (!IS_DEV && taskId) await invoke("username_checker_stop", { taskId }).catch(() => {});
    setRunning(false);
    setLogs((prev) => [...prev, t("common.stoppedByUser")]);
  };

  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">{t("descriptions.usernameChecker")}</p>
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div className="p-6 space-y-5">

          <div>
            <label className="text-sm font-medium text-foreground">{t("usernameChecker.inputFile")}</label>
            <p className="text-xs text-muted-foreground mb-2">{t("usernameChecker.inputFileHint")}</p>
            <div className="flex items-center gap-2">
              <button onClick={selectInputFile} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition">
                <FolderOpen className="h-4 w-4" /> {t("common.selectFile")}
              </button>
              <span className="text-xs text-muted-foreground truncate max-w-sm">{config.inputPath ? config.inputPath.split(/[/\\]/).pop() : t("common.notSelected")}</span>
            </div>
          </div>

          <div>
            <label className="text-sm font-medium text-foreground">{t("usernameChecker.outputFile")} <span className="text-destructive">*</span></label>
            <p className="text-xs text-muted-foreground mb-1.5">{t("usernameChecker.outputFileHint")}</p>
            <div className="flex items-center gap-2 mt-1.5">
              <button onClick={selectOutputFile} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition">
                <FolderOpen className="h-4 w-4" /> {t("common.selectFile")}
              </button>
              <span className="text-xs text-muted-foreground truncate max-w-sm">{config.outputPath ? config.outputPath.split(/[/\\]/).pop() : "username_check.db"}</span>
            </div>
          </div>

          <label className="flex items-center gap-2.5 cursor-pointer text-sm">
            <input
              type="checkbox"
              checked={config.autoClaim}
              onChange={(e) => set("autoClaim", e.target.checked)}
              className="rounded border-border accent-primary h-4 w-4"
            />
            <span className="text-foreground font-medium">{t("usernameChecker.autoClaim")}</span>
          </label>
          {config.autoClaim && (
            <p className="ml-7 text-xs text-muted-foreground">
              {t("usernameChecker.autoClaimHint")}
            </p>
          )}

        </div>
      </div>

      {/* controls */}
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-6 text-sm font-semibold">
          <span>{t("usernameChecker.free")}: <span className="text-[oklch(0.65_0.1_150)]">{stats.free}</span></span>
          <span>{t("usernameChecker.fragment")}: <span className="text-[oklch(0.7_0.13_85)]">{stats.fragment}</span></span>
          <span>{t("usernameChecker.taken")}: <span className="text-[oklch(0.55_0.1_25)]">{stats.taken}</span></span>
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

      <AccountPickerModal
        open={pickerOpen}
        onClose={() => setPickerOpen(false)}
        onSelect={handleAccountSelected}
        title={t("common.selectAccounts")}
      />
    </div>
  );
}
