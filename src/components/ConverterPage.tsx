import { useState, useRef, useMemo, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { FolderOpen, Play, Square, RefreshCw, AlertTriangle } from "lucide-react";
import { ThreadInput } from "@/components/ThreadInput";
import { useT } from "@/i18n";

type FormatKey = "telethon" | "pyrogram" | "tdata" | "tdata_zip" | "authkey";

const ALL_FORMATS: { key: FormatKey; label: string }[] = [
  { key: "telethon", label: "Telethon (.session)" },
  { key: "pyrogram", label: "Pyrogram (.session)" },
  { key: "tdata", label: "TData (folder)" },
  { key: "tdata_zip", label: "TData ZIP" },
  { key: "authkey", label: "Auth Keys (.txt)" },
];

export function ConverterPage() {
  const t = useT();
  const [fromFormat, setFromFormat] = useState<FormatKey>("telethon");
  const [toFormat, setToFormat] = useState<FormatKey>("tdata");
  const [paths, setPaths] = useState<string[]>([]);
  const [outputDir, setOutputDir] = useState("");
  const [outputFile, setOutputFile] = useState("");
  const [addToPanel, setAddToPanel] = useState(false);
  const [threads, setThreads] = useState(10);
  const [running, setRunning] = useState(false);
  const [logs, setLogs] = useState<string[]>([]);
  const [stats, setStats] = useState({ ok: 0, err: 0 });
  const [outputFileWarning, setOutputFileWarning] = useState(false);
  const logsRef = useRef<HTMLDivElement>(null);

  const IS_DEV = !("__TAURI_INTERNALS__" in window);

  useEffect(() => {
    if (IS_DEV) return;
    invoke<{ converter_threads?: number }>("get_settings").then((s) => {
      if (s.converter_threads) setThreads(s.converter_threads);
    }).catch(() => {});
  }, []);

  // filter output formats: exclude the same as input
  const outputFormats = useMemo(() => {
    return ALL_FORMATS.filter((f) => f.key !== fromFormat);
  }, [fromFormat]);

  // filter input formats: exclude the same as output
  const inputFormats = useMemo(() => {
    return ALL_FORMATS.filter((f) => f.key !== toFormat);
  }, [toFormat]);

  // when from changes, reset paths and fix toFormat if it matches
  const handleFromChange = (val: FormatKey) => {
    setFromFormat(val);
    setPaths([]);
    if (val === toFormat) {
      const first = ALL_FORMATS.find((f) => f.key !== val);
      if (first) setToFormat(first.key);
    }
  };

  // when to changes, fix fromFormat if it matches
  const handleToChange = (val: FormatKey) => {
    setToFormat(val);
    setOutputDir("");
    setOutputFile("");
    if (val === fromFormat) {
      const first = ALL_FORMATS.find((f) => f.key !== val);
      if (first) setFromFormat(first.key);
    }
  };

  const isOutputAuthKey = toFormat === "authkey";
  const isInputAuthKey = fromFormat === "authkey";

  const addLog = (msg: string) => {
    setLogs((prev) => [...prev.slice(-500), msg]);
    setTimeout(() => logsRef.current?.scrollTo(0, logsRef.current.scrollHeight), 50);
  };

  const handleLoadFiles = async () => {
    if (IS_DEV) {
      setPaths((p) => [...p, "C:\\example\\sessions"]);
      addLog("Loaded: C:\\example\\sessions");
      return;
    }
    const { open } = await import("@tauri-apps/plugin-dialog");

    let selected;
    if (fromFormat === "tdata") {
      selected = await open({ multiple: true, directory: true, title: t("common.selectFolder") });
    } else if (fromFormat === "tdata_zip") {
      selected = await open({ multiple: true, filters: [{ name: "ZIP", extensions: ["zip"] }], title: t("common.selectFile") });
    } else {
      selected = await open({ multiple: true, filters: [{ name: "Session", extensions: ["session"] }], title: t("common.selectFile") });
    }

    if (!selected) return;
    const newPaths = Array.isArray(selected) ? selected : [selected];
    setPaths((prev) => [...prev, ...newPaths.filter((p) => !prev.includes(p))]);
    for (const p of newPaths) addLog(`Loaded: ${p}`);
  };

  const handleSelectOutput = async () => {
    if (IS_DEV) {
      if (isOutputAuthKey) setOutputFile("C:\\output\\keys.txt");
      else setOutputDir("C:\\output");
      return;
    }
    const { open, save } = await import("@tauri-apps/plugin-dialog");

    if (isOutputAuthKey) {
      const selected = await save({
        filters: [{ name: "TXT", extensions: ["txt"] }],
        title: t("converter.outputFile"),
      });
      if (selected) {
        setOutputFile(selected);
        // check if file exists and not empty
        // we'll just set warning flag, actual check happens on start
        setOutputFileWarning(true);
      }
    } else {
      const selected = await open({ directory: true, multiple: false, title: t("converter.outputDir") });
      if (selected) setOutputDir(selected as string);
    }
  };

  const handleStart = async () => {
    if (isInputAuthKey && paths.length === 0) {
      addLog(`${t("common.error")}: auth keys not loaded`);
      return;
    }
    if (!isInputAuthKey && paths.length === 0) {
      addLog(`${t("common.error")}: no files loaded`);
      return;
    }
    const outPath = isOutputAuthKey ? outputFile : outputDir;
    if (!outPath) {
      addLog(`${t("common.error")}: ${isOutputAuthKey ? t("converter.outputFile") : t("converter.outputDir")}`);
      return;
    }

    setRunning(true);
    setStats({ ok: 0, err: 0 });
    addLog(`${t("converter.converting")} ${paths.length}: ${fromFormat} -> ${toFormat} (${threads} ${t("common.threads")})...`);

    const unLog = await listen<string>("converter-log", (e) => addLog(e.payload));
    const unStats = await listen<string>("converter-stats", (e) => {
      if (e.payload === "ok") setStats((s) => ({ ...s, ok: s.ok + 1 }));
      else setStats((s) => ({ ...s, err: s.err + 1 }));
    });
    const unDone = await listen<string>("converter-done", () => {
      setRunning(false);
      unLog();
      unStats();
      unDone();
    });

    await invoke("converter_start", {
      paths,
      fromFormat,
      toFormat,
      outputDir: outPath,
      addToPanel,
      threads,
    });
  };

  const handleStop = async () => {
    addLog(t("common.stoppedByUser"));
    await invoke("converter_stop").catch(() => {});
  };

  const handleLoadAuthKeyTxt = async () => {
    if (IS_DEV) {
      setPaths(["C:\\example\\keys.txt"]);
      addLog("Loaded: C:\\example\\keys.txt");
      return;
    }
    try {
      const path = await invoke<string>("get_authkey_txt_path");
      await invoke("open_file_in_editor", { path });
      setPaths([path]);
      addLog(`Loaded: ${path}`);
    } catch (e: any) {
      addLog(`${t("common.error")}: ${e}`);
    }
  };

  const hasOutput = isOutputAuthKey ? !!outputFile : !!outputDir;

  return (
    <div className="space-y-5">
      <p className="text-sm text-muted-foreground">{t("descriptions.converter")}</p>
      {/* format selectors */}
      <div className="flex items-center gap-4 flex-wrap">
        <div className="flex flex-col gap-1">
          <span className="text-xs text-muted-foreground">{t("converter.from")}</span>
          <select
            value={fromFormat}
            onChange={(e) => handleFromChange(e.target.value as FormatKey)}
            disabled={running}
            className="rounded-md border border-border bg-card px-3 py-1.5 text-sm outline-none focus:border-primary/50"
          >
            {inputFormats.map((f) => <option key={f.key} value={f.key}>{f.label}</option>)}
          </select>
        </div>

        <RefreshCw className="h-4 w-4 text-muted-foreground mt-4" />

        <div className="flex flex-col gap-1">
          <span className="text-xs text-muted-foreground">{t("converter.to")}</span>
          <select
            value={toFormat}
            onChange={(e) => handleToChange(e.target.value as FormatKey)}
            disabled={running}
            className="rounded-md border border-border bg-card px-3 py-1.5 text-sm outline-none focus:border-primary/50"
          >
            {outputFormats.map((f) => <option key={f.key} value={f.key}>{f.label}</option>)}
          </select>
        </div>
      </div>

      {/* authkey input info */}
      {isInputAuthKey && (
        <div className="rounded-md border border-border bg-muted/30 px-4 py-2.5 text-xs text-muted-foreground space-y-1">
          <p>{t("converter.authkeyFormat")}</p>
          <p>{t("converter.authkeyFormats")}</p>
          <p>{t("converter.authkeyDcHint")}</p>
        </div>
      )}

      {/* output file warning */}
      {isOutputAuthKey && outputFileWarning && outputFile && (
        <div className="flex items-center gap-2 rounded-md border border-yellow-600/30 bg-yellow-600/5 px-3 py-2 text-xs text-yellow-500">
          <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
          <span>{t("converter.appendWarning")}</span>
          <button onClick={() => setOutputFileWarning(false)} className="ml-auto text-muted-foreground hover:text-foreground">✕</button>
        </div>
      )}

      {/* action bar */}
      <div className="flex items-center gap-3 flex-wrap">
        {isInputAuthKey ? (
          <button
            onClick={handleLoadAuthKeyTxt}
            disabled={running}
            className="flex items-center gap-1.5 rounded-md border border-border bg-card px-3 py-1.5 text-sm font-medium hover:border-primary/50 transition disabled:opacity-50"
          >
            <FolderOpen className="h-3.5 w-3.5" />
            {paths.length > 0 ? `...${paths[0].slice(-25)}` : t("common.selectFile")}
          </button>
        ) : (
          <button
            onClick={handleLoadFiles}
            disabled={running}
            className="flex items-center gap-1.5 rounded-md border border-border bg-card px-3 py-1.5 text-sm font-medium hover:border-primary/50 transition disabled:opacity-50"
          >
            <FolderOpen className="h-3.5 w-3.5" />
            {t("common.selectFile")} ({paths.length})
          </button>
        )}

        <button
          onClick={handleSelectOutput}
          disabled={running}
          className="flex items-center gap-1.5 rounded-md border border-border bg-card px-3 py-1.5 text-sm font-medium hover:border-primary/50 transition disabled:opacity-50"
        >
          <FolderOpen className="h-3.5 w-3.5" />
          {isOutputAuthKey
            ? (outputFile ? `...${outputFile.slice(-25)}` : t("converter.outputFile"))
            : (outputDir ? `...${outputDir.slice(-30)}` : t("converter.outputDir"))
          }
        </button>

        {running ? (
          <button
            onClick={handleStop}
            className="flex items-center gap-1.5 rounded-md border border-red-500/30 bg-red-500/10 px-3 py-1.5 text-sm font-medium text-red-500 hover:bg-red-500/20 transition"
          >
            <Square className="h-3.5 w-3.5" />
            {t("common.stop")}
          </button>
        ) : (
          <button
            onClick={handleStart}
            disabled={paths.length === 0 || !hasOutput}
            className="flex items-center gap-1.5 rounded-md border border-border bg-card px-3 py-1.5 text-sm font-medium hover:border-primary/50 transition disabled:opacity-50"
          >
            <Play className="h-3.5 w-3.5" />
            {t("converter.convertBtn")}
          </button>
        )}

        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <span>{t("common.threads")}:</span>
          <ThreadInput value={threads} onChange={(v) => {
            setThreads(v);
            if (!IS_DEV) invoke("patch_settings", { patch: { converter_threads: v } }).catch(() => {});
          }} min={1} max={1000} />
        </div>
      </div>

      {/* options */}
      {!isOutputAuthKey && (
        <div className="flex flex-col gap-2 border-t border-border pt-3">
          <label className="flex items-center gap-2 text-sm text-muted-foreground cursor-pointer">
            <input
              type="checkbox"
              checked={addToPanel}
              onChange={() => setAddToPanel(!addToPanel)}
              className="rounded border-border accent-primary"
            />
            {t("converter.addToPanel")}
          </label>
        </div>
      )}

      {/* stats */}
      <div className="flex items-center gap-6 text-sm font-semibold">
        <span>{t("common.total")}: <span className="text-[oklch(0.65_0.15_280)]">{stats.ok + stats.err}</span></span>
        <span>{t("common.done")}: <span className="text-[oklch(0.65_0.1_150)]">{stats.ok}</span></span>
        <span>{t("common.errors")}: <span className="text-[oklch(0.55_0.1_25)]">{stats.err}</span></span>
      </div>

      {/* logs */}
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div ref={logsRef} className="h-40 overflow-y-auto scrollbar-thin p-4 font-mono text-xs space-y-0.5">
          {logs.length === 0 ? (
            <div className="text-muted-foreground text-center py-8">{t("common.logsPlaceholder")}</div>
          ) : (
            logs.map((line, i) => (
              <div key={i} className="text-muted-foreground whitespace-pre-wrap break-all">{line}</div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}
