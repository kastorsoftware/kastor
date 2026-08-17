import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Play, Square, FolderOpen } from "lucide-react";
import { AccountPickerModal } from "@/components/AccountPickerModal";
import { useT } from "@/i18n";
import { isDone, extractSearchCount } from "@/lib/eventParser";

interface Config {
  inputPath: string;
  outputPath: string;
  mode: string;
  outputType: string;
  delayMin: number;
  delayMax: number;
  maxFloodWait: number;
  distribution: string;
  kolPerAcc: number;
  typeahead: boolean;
}

const IS_DEV = !("__TAURI_INTERNALS__" in window);
const STORAGE_KEY = "global_search_config";

const defaultConfig: Config = {
  inputPath: "",
  outputPath: "",
  mode: "all",
  outputType: "links",
  delayMin: 1000,
  delayMax: 3000,
  maxFloodWait: 60,
  distribution: "all",
  kolPerAcc: 0,
  typeahead: false,
};

function loadSavedConfig(): Config {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return { ...defaultConfig, ...JSON.parse(saved) };
  } catch {}
  return { ...defaultConfig };
}

export function GlobalSearchPage() {
  const t = useT();
  const [config, setConfig] = useState<Config>(loadSavedConfig());
  const [running, setRunning] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [foundCount, setFoundCount] = useState(0);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [threadCount, setThreadCount] = useState(0);
  const logsRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (IS_DEV) return;
    const unlisten = listen<string>("global-search-log", (e) => {
      setLogs((prev) => [...prev, e.payload]);
      const msg = e.payload;
      if (isDone(msg)) {
        setRunning(false);
      } else {
        const count = extractSearchCount(msg);
        if (count !== null) setFoundCount((c) => c + count);
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
    const path = await save({ defaultPath: "global_search.db", filters: [{ name: "SQLite DB", extensions: ["db"] }] });
    if (path) set("outputPath", path);
  };

  const handleStart = () => {
    if (!config.inputPath.trim()) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${t("globalSearch.inputFile")}`]);
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
    setFoundCount(0);
    setRunning(true);
    try {
      const tid = await invoke<string>("global_search_start", {
        ids,
        config: {
          input_path: config.inputPath.trim(),
          output_path: config.outputPath.trim(),
          mode: config.mode,
          output_type: config.outputType,
          delay_min: config.delayMin,
          delay_max: config.delayMax,
          max_flood_wait: config.maxFloodWait,
          distribution: config.distribution,
          kol_per_acc: config.kolPerAcc,
          typeahead: config.typeahead,
          use_search_global: true,
          save_to_db: true,
        },
      });
      setTaskId(tid);
    } catch (e: any) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${e}`]);
      setRunning(false);
    }
  };

  const handleStop = async () => {
    if (!IS_DEV && taskId) await invoke("global_search_stop", { taskId }).catch(() => {});
    setRunning(false);
    setLogs((prev) => [...prev, t("common.stoppedByUser")]);
  };

  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">{t("descriptions.globalSearch")}</p>
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div className="p-6 space-y-5">

          <div>
            <label className="text-sm font-medium text-foreground">{t("globalSearch.inputFile")}</label>
            <div className="flex items-center gap-2">
              <button onClick={selectInputFile} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition">
                <FolderOpen className="h-4 w-4" /> {t("common.selectFile")}
              </button>
              <span className="text-xs text-muted-foreground truncate max-w-sm">{config.inputPath ? config.inputPath.split(/[/\\]/).pop() : t("common.notSelected")}</span>
            </div>
          </div>

          <div>
            <label className="text-sm font-medium text-foreground">{t("globalSearch.outputFile")} <span className="text-destructive">*</span></label>
            <div className="flex items-center gap-2 mt-1.5">
              <button onClick={selectOutputFile} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition">
                <FolderOpen className="h-4 w-4" /> {t("common.selectFile")}
              </button>
              <span className="text-xs text-muted-foreground truncate max-w-sm">{config.outputPath ? config.outputPath.split(/[/\\]/).pop() : "global_search.db"}</span>
            </div>
          </div>

          <div>
            <label className="text-sm font-medium text-foreground">{t("globalSearch.title")}</label>
            <div className="flex gap-2 mt-1.5 flex-wrap">
              {([["all", t("globalSearch.modeAll")], ["channels", t("globalSearch.modeChannels")], ["groups", t("globalSearch.modeGroups")], ["users", t("globalSearch.modeUsers")]] as const).map(([val, lbl]) => (
                <button key={val} onClick={() => set("mode", val)}
                  className={`rounded-md border px-3 py-1.5 text-sm transition ${config.mode === val ? "border-primary/50 bg-primary/10 text-primary font-medium" : "border-border bg-background text-muted-foreground hover:border-primary/30"}`}
                >{lbl}</button>
              ))}
            </div>
          </div>

          <div>
            <label className="text-sm font-medium text-foreground">{t("globalSearch.outputLinks")}</label>
            <div className="flex gap-2 mt-1.5">
              {([["links", t("globalSearch.outputLinks")], ["usernames", t("globalSearch.outputUsernames")]] as const).map(([val, lbl]) => (
                <button key={val} onClick={() => set("outputType", val)}
                  className={`rounded-md border px-3 py-1.5 text-sm transition ${config.outputType === val ? "border-primary/50 bg-primary/10 text-primary font-medium" : "border-border bg-background text-muted-foreground hover:border-primary/30"}`}
                >{lbl}</button>
              ))}
            </div>
          </div>

          {/* Distribution mode */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("globalSearch.distributionAll")}</label>
            <div className="flex gap-2 mt-1.5">
              <button onClick={() => set("distribution", "all")}
                className={`rounded-md border px-3 py-1.5 text-sm transition ${config.distribution === "all" ? "border-primary/50 bg-primary/10 text-primary font-medium" : "border-border bg-background text-muted-foreground hover:border-primary/30"}`}>
                {t("globalSearch.distributionAll")}
              </button>
              <button onClick={() => set("distribution", "unic")}
                className={`rounded-md border px-3 py-1.5 text-sm transition ${config.distribution === "unic" ? "border-primary/50 bg-primary/10 text-primary font-medium" : "border-border bg-background text-muted-foreground hover:border-primary/30"}`}>
                {t("globalSearch.distributionUnique")}
              </button>
            </div>
          </div>

          {/* kol_per_acc */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("globalSearch.wordsPerAccount")}</label>
            <input type="number" min={0} max={99999} value={config.kolPerAcc}
              onChange={(e) => set("kolPerAcc", Math.max(0, Number(e.target.value)))}
              className="mt-1.5 w-28 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
          </div>

          {/* Options checkboxes */}
          <div className="space-y-2">
            <label className="flex items-center gap-3 cursor-pointer">
              <input type="checkbox" checked={config.typeahead} onChange={(e) => set("typeahead", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
              <span className="text-sm font-medium text-foreground">{t("globalSearch.typeahead")}</span>
            </label>
          </div>

          <div>
            <label className="text-sm font-medium text-foreground">{t("common.delay")}</label>
            <p className="text-xs text-muted-foreground mb-1.5">{t("common.delay")}</p>
            <div className="flex items-center gap-2">
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
            <label className="text-sm font-medium text-foreground">{t("common.maxFloodWait")}</label>
            <p className="text-xs text-muted-foreground mb-1.5">{t("common.maxFloodWait")}</p>
            <input type="number" min={0} value={config.maxFloodWait}
              onChange={(e) => set("maxFloodWait", Math.max(0, Number(e.target.value)))}
              className="w-24 rounded-md border border-border bg-background px-3 py-2 text-sm" />
          </div>

        </div>
      </div>

      {/* controls */}
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-6 text-sm font-semibold">
          <span>{t("common.results")}: <span className="text-[oklch(0.65_0.1_150)]">{foundCount}</span></span>
          {threadCount > 0 && <span className="text-muted-foreground font-normal">{t("common.threads")}: {threadCount}</span>}
        </div>
        <div className="ml-auto flex items-center gap-3">
          <span className="max-w-[220px] text-right text-xs text-muted-foreground">{t("common.startSelectAccountsHint")}</span>
          <button onClick={handleStart} disabled={running}
            className="flex items-center gap-2 rounded-md border border-primary/50 bg-primary/10 px-4 py-2 text-sm text-primary font-medium hover:bg-primary/20 transition disabled:opacity-50">
            <Play className="h-4 w-4" /> {t("common.start")}
          </button>
          <button onClick={handleStop} disabled={!running}
            className="flex items-center gap-2 rounded-md border px-4 py-2 text-sm font-medium transition disabled:opacity-50"
            style={{ borderColor: "color-mix(in oklch, oklch(0.55 0.1 25) 50%, transparent)", color: "oklch(0.55 0.1 25)", background: "color-mix(in oklch, oklch(0.55 0.1 25) 6%, transparent)" }}>
            <Square className="h-4 w-4" /> {t("common.stop")}
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
