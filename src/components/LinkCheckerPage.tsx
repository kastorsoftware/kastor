import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Play, Square, FolderOpen } from "lucide-react";
import { AccountPickerModal } from "@/components/AccountPickerModal";
import { useT } from "@/i18n";

interface Config {
  inputPath: string;
  outputPath: string;
  maxFloodWait: number;
  delayMin: number;
  delayMax: number;
  checkPrivateGroups: boolean;
  standardizeLinks: boolean;
  linksPerAccount: number;
}

const IS_DEV = !("__TAURI_INTERNALS__" in window);
const STORAGE_KEY = "link_checker_config";

const defaultConfig: Config = {
  inputPath: "",
  outputPath: "",
  maxFloodWait: 60,
  delayMin: 500,
  delayMax: 1000,
  checkPrivateGroups: true,
  standardizeLinks: false,
  linksPerAccount: 0,
};

function loadSavedConfig(): Config {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return { ...defaultConfig, ...JSON.parse(saved) };
  } catch {}
  return { ...defaultConfig };
}

export function LinkCheckerPage() {
  const t = useT();
  const [config, setConfig] = useState<Config>(loadSavedConfig());
  const [running, setRunning] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [stats, setStats] = useState({ valid: 0, invalid: 0, skipped: 0 });
  const [pickerOpen, setPickerOpen] = useState(false);
  const [threadCount, setThreadCount] = useState(0);
  const logsRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (IS_DEV) return;
    const unlisten = listen<string>("link-checker-log", (e) => {
      setLogs((prev) => [...prev, e.payload]);
      const msg = e.payload;
      if (msg === "Завершено" || msg === "Done") {
        setRunning(false);
      } else if (msg.includes("— валидна") || msg.includes("— valid")) {
        setStats((s) => ({ ...s, valid: s.valid + 1 }));
      } else if (msg.includes("— невалидна") || msg.includes("— invalid")) {
        setStats((s) => ({ ...s, invalid: s.invalid + 1 }));
      } else if (msg.includes("— пропущена") || msg.includes("— skipped")) {
        setStats((s) => ({ ...s, skipped: s.skipped + 1 }));
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
    localStorage.setItem(STORAGE_KEY, JSON.stringify(config));
  }, [config]);

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
    const path = await save({ defaultPath: "link_check.db", filters: [{ name: "SQLite DB", extensions: ["db"] }] });
    if (path) set("outputPath", path);
  };

  const handleStart = () => {
    if (!config.inputPath.trim()) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${t("validation.selectInputFile")}`]);
      return;
    }
    setPickerOpen(true);
  };

  const handleAccountSelected = (ids: string[]) => {
    setThreadCount(ids.length);
    doStart(ids);
  };

  const doStart = async (ids: string[]) => {
    setLogs([]);
    setStats({ valid: 0, invalid: 0, skipped: 0 });
    setRunning(true);
    try {
      const tid = await invoke<string>("link_checker_start", {
        ids,
        config: {
          input_path: config.inputPath.trim(),
          output_path: config.outputPath.trim(),
          max_flood_wait: config.maxFloodWait,
          delay_min: config.delayMin,
          delay_max: config.delayMax,
          check_private_groups: config.checkPrivateGroups,
          standardize_links: config.standardizeLinks,
          links_per_account: config.linksPerAccount,
        },
      });
      setTaskId(tid);
    } catch (e: any) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${e}`]);
      setRunning(false);
    }
  };

  const handleStop = async () => {
    if (!IS_DEV && taskId) await invoke("link_checker_stop", { taskId }).catch(() => {});
    setRunning(false);
    setLogs((prev) => [...prev, t("common.stoppedByUser")]);
  };

  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">{t("descriptions.linkChecker")}</p>
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div className="p-6 space-y-5">

          <div>
            <label className="text-sm font-medium text-foreground">{t("linkChecker.inputFile")}</label>
            <p className="text-xs text-muted-foreground mb-2">{t("linkChecker.inputFileHint")}</p>
            <div className="flex items-center gap-2">
              <button onClick={selectInputFile} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition">
                <FolderOpen className="h-4 w-4" /> {t("linkChecker.selectFile")}
              </button>
              <span className="text-xs text-muted-foreground truncate max-w-sm">{config.inputPath ? config.inputPath.split(/[/\\]/).pop() : t("linkChecker.notSelected")}</span>
            </div>
          </div>

          <div>
            <label className="text-sm font-medium text-foreground">{t("linkChecker.outputFile")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <button onClick={selectOutputFile} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition">
                <FolderOpen className="h-4 w-4" /> {t("linkChecker.selectFile")}
              </button>
              <span className="text-xs text-muted-foreground truncate max-w-sm">{config.outputPath ? config.outputPath.split(/[/\\]/).pop() : "link_check.db (auto)"}</span>
            </div>
          </div>

          <div className="border-t border-border pt-4">
            <div className="space-y-2">
              <label className="flex items-center gap-2.5 cursor-pointer text-sm">
                <input
                  type="checkbox"
                  checked={config.checkPrivateGroups}
                  onChange={(e) => set("checkPrivateGroups", e.target.checked)}
                  className="rounded border-border accent-primary h-4 w-4"
                />
                <span className="text-foreground font-medium">{t("linkChecker.checkPrivate") || "Check private groups"}</span>
              </label>
              <label className="flex items-center gap-2.5 cursor-pointer text-sm">
                <input
                  type="checkbox"
                  checked={config.standardizeLinks}
                  onChange={(e) => set("standardizeLinks", e.target.checked)}
                  className="rounded border-border accent-primary h-4 w-4"
                />
                <span className="text-foreground font-medium">{t("linkChecker.standardize") || "Standardize links"}</span>
              </label>
            </div>
          </div>

          <div className="border-t border-border pt-4">
            <div className="space-y-3">
              <div>
                <label className="text-sm font-medium text-foreground">{t("common.delayMin")} — {t("common.delayMax")} (ms)</label>
                <div className="flex items-center gap-2 mt-1.5">
                  <input type="number" min={0} value={config.delayMin}
                    onChange={(e) => set("delayMin", Math.max(0, Number(e.target.value)))}
                    className="w-20 rounded-md border border-border bg-background px-3 py-2 text-sm text-center" />
                  <span className="text-sm text-muted-foreground">—</span>
                  <input type="number" min={0} value={config.delayMax}
                    onChange={(e) => set("delayMax", Math.max(0, Number(e.target.value)))}
                    className="w-20 rounded-md border border-border bg-background px-3 py-2 text-sm text-center" />
                </div>
              </div>

              <div>
                <label className="text-sm font-medium text-foreground">{t("common.maxFloodWait")} (sec)</label>
                <input
                  type="number"
                  min={0}
                  value={config.maxFloodWait}
                  onChange={(e) => set("maxFloodWait", Math.max(0, Number(e.target.value)))}
                  className="mt-1.5 w-24 rounded-md border border-border bg-background px-3 py-2 text-sm"
                />
              </div>
            </div>
          </div>

        </div>
      </div>

      {/* controls */}
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-6 text-sm font-semibold">
          <span>{t("linkChecker.valid")}: <span className="text-[oklch(0.65_0.1_150)]">{stats.valid}</span></span>
          <span>{t("linkChecker.invalid")}: <span className="text-[oklch(0.55_0.1_25)]">{stats.invalid}</span></span>
          {threadCount > 0 && <span className="text-muted-foreground font-normal">{t("linkChecker.threads")}: {threadCount}</span>}
        </div>
        <div className="ml-auto flex items-center gap-3">
          <span className="max-w-[220px] text-right text-xs text-muted-foreground">{t("common.startSelectAccountsHint")}</span>
          <button
            onClick={handleStart}
            disabled={running}
            className="flex items-center gap-2 rounded-md border border-primary/50 bg-primary/10 px-4 py-2 text-sm text-primary font-medium hover:bg-primary/20 transition disabled:opacity-50"
          >
            <Play className="h-4 w-4" />
            {t("linkChecker.start")}
          </button>
          <button
            onClick={handleStop}
            disabled={!running}
            className="flex items-center gap-2 rounded-md border px-4 py-2 text-sm font-medium transition disabled:opacity-50"
            style={{ borderColor: "color-mix(in oklch, oklch(0.55 0.1 25) 50%, transparent)", color: "oklch(0.55 0.1 25)", background: "color-mix(in oklch, oklch(0.55 0.1 25) 6%, transparent)" }}
          >
            <Square className="h-4 w-4" />
            {t("linkChecker.stop")}
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
      />
    </div>
  );
}
