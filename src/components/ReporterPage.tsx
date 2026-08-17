import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Play, Square } from "lucide-react";
import { AccountPickerModal } from "@/components/AccountPickerModal";
import { ThreadInput } from "@/components/ThreadInput";
import { useT } from "@/i18n";
import { isDone, isError, isReportSent } from "@/lib/eventParser";

type ReportMode = "peer" | "channel" | "bot" | "photo";

const REASONS = [
  { key: "spam", label: "Spam" },
  { key: "violence", label: "Violence" },
  { key: "pornography", label: "Pornography" },
  { key: "child_abuse", label: "Child Abuse" },
  { key: "copyright", label: "Copyright" },
  { key: "fake", label: "Fake" },
  { key: "geo_irrelevant", label: "Geo Irrelevant" },
  { key: "illegal_drugs", label: "Illegal Drugs" },
  { key: "personal_details", label: "Personal Details" },
  { key: "other", label: "Other" },
] as const;

type ReasonKey = (typeof REASONS)[number]["key"];

interface ReporterConfig {
  mode: ReportMode;
  target: string;
  targets: string[];
  randomReason: boolean;
  reasons: Record<ReasonKey, boolean>;
  delayMin: number;
  delayMax: number;
  limitPerAccount: number;
  messageMode: "none" | "single" | "from_file";
  messageSingle: string;
  messageFilePaths: Partial<Record<ReasonKey, string>>;
  viewAfterReport: boolean;
  postTarget: "last" | "all";
  postCount: number;
  photoOption: "one" | "all";
}

const defaultConfig: ReporterConfig = {
  mode: "peer",
  target: "",
  targets: [],
  randomReason: true,
  reasons: {
    spam: false,
    violence: false,
    pornography: false,
    child_abuse: false,
    copyright: false,
    fake: false,
    geo_irrelevant: false,
    illegal_drugs: false,
    personal_details: false,
    other: false,
  },
  delayMin: 2,
  delayMax: 10,
  limitPerAccount: 1,
  messageMode: "none",
  messageSingle: "",
  messageFilePaths: {},
  viewAfterReport: true,
  postTarget: "last",
  postCount: 5,
  photoOption: "all",
};

const STORAGE_KEY = "reporter_config";
const IS_DEV = !("__TAURI_INTERNALS__" in window);

function loadSavedConfig(): ReporterConfig {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return { ...defaultConfig, ...JSON.parse(saved) };
  } catch {}
  return defaultConfig;
}

export function ReporterPage() {
  const t = useT();
  const [config, setConfig] = useState<ReporterConfig>(loadSavedConfig);
  const [threads, setThreads] = useState(5);
  const [running, setRunning] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [stats, setStats] = useState({ done: 0, errors: 0, inProgress: 0 });
  const [pickerOpen, setPickerOpen] = useState(false);
  const logsRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(config));
  }, [config]);

  useEffect(() => {
    if (IS_DEV) return;
    const unlisten = listen<string>("reporter-log", (e) => {
      setLogs((prev) => [...prev, e.payload]);
      const msg = e.payload;
      if (isDone(msg)) {
        setRunning(false);
      } else if (isError(msg)) {
        setStats((s) => ({ ...s, errors: s.errors + 1 }));
      } else if (isReportSent(msg)) {
        setStats((s) => ({ ...s, done: s.done + 1 }));
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

  const set = <K extends keyof ReporterConfig>(key: K, value: ReporterConfig[K]) => {
    setConfig((prev) => ({ ...prev, [key]: value }));
  };

  const setReason = (key: ReasonKey, value: boolean) => {
    setConfig((prev) => ({
      ...prev,
      reasons: { ...prev.reasons, [key]: value },
    }));
  };

  const toggleRandomReason = (v: boolean) => {
    if (v) {
      const cleared: Record<ReasonKey, boolean> = {} as any;
      REASONS.forEach((r) => (cleared[r.key] = false));
      setConfig((prev) => ({ ...prev, randomReason: true, reasons: cleared }));
    } else {
      set("randomReason", false);
    }
  };

  const selectedReasons = REASONS.filter((r) => config.reasons[r.key]);
  const hasValidReasons = config.randomReason || selectedReasons.length > 0;

  const handleStart = () => {
    if (!config.target.trim()) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${config.mode === "bot" ? t("reporter.target") : t("reporter.target")}`]);
      return;
    }
    if (config.mode !== "bot" && !hasValidReasons) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${t("reporter.reasons")}`]);
      return;
    }
    setPickerOpen(true);
  };

  const handleAccountsSelected = async (ids: string[]) => {
    if (ids.length === 0) return;
    setLogs([]);
    setStats({ done: 0, errors: 0, inProgress: ids.length * config.limitPerAccount });
    setRunning(true);
    try {
      const tid = await invoke<string>("reporter_start", {
        ids,
        config: {
          mode: config.mode,
          target: config.target.trim(),
          targets: config.target.split(/[,\n]/).map((t) => t.trim()).filter(Boolean),
          random_reason: config.randomReason,
          reasons: REASONS.filter((r) => config.reasons[r.key]).map((r) => r.key),
          delay_min: config.delayMin,
          delay_max: config.delayMax,
          limit_per_account: config.limitPerAccount,
          message_mode: config.messageMode,
          message_single: config.messageSingle,
          message_file_paths: config.messageFilePaths,
          view_after_report: config.viewAfterReport,
          post_target: config.postTarget,
          post_count: config.postCount,
          photo_option: config.photoOption,
        },
        threads,
      });
      setTaskId(tid);
    } catch (e: any) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${e}`]);
      setRunning(false);
    }
  };

  const handleStop = async () => {
    if (!IS_DEV && taskId) await invoke("reporter_stop", { taskId }).catch(() => {});
    setRunning(false);
    setLogs((prev) => [...prev, t("common.stoppedByUser")]);
  };

  const selectFile = async (reasonKey: ReasonKey) => {
    if (IS_DEV) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open({ multiple: false, filters: [{ name: "Text", extensions: ["txt"] }] });
    if (path) {
      setConfig((prev) => ({
        ...prev,
        messageFilePaths: { ...prev.messageFilePaths, [reasonKey]: path as string },
      }));
    }
  };

  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">{t("descriptions.reporter")}</p>
      {/* mode tabs */}
      <div className="flex gap-2 flex-wrap">
        <ModeTab active={config.mode === "peer"} onClick={() => set("mode", "peer")}>{t("reporter.modePeer")}</ModeTab>
        <ModeTab active={config.mode === "channel"} onClick={() => set("mode", "channel")}>{t("reporter.modeChannel")}</ModeTab>
        <ModeTab active={config.mode === "photo"} onClick={() => set("mode", "photo")}>{t("reporter.modePhoto")}</ModeTab>
        <ModeTab active={config.mode === "bot"} onClick={() => set("mode", "bot")}>{t("reporter.modeBot")}</ModeTab>
      </div>

      {/* config panel */}
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div className="p-6 space-y-5">

          {/* target */}
          {config.mode !== "bot" && (
          <div>
            <label className="text-sm font-medium text-foreground">
              {t("reporter.target")}
            </label>
            <input
              value={config.target}
              onChange={(e) => set("target", e.target.value)}
              placeholder={config.mode === "channel" ? "https://t.me/channel/123" : "@username"}
              className="mt-1.5 w-full max-w-md rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50"
            />
          </div>
          )}

          {/* bot mode: search query */}
          {config.mode === "bot" && (
          <div>
            <label className="text-sm font-medium text-foreground">{t("reporter.target")}</label>
            <input
              value={config.target}
              onChange={(e) => set("target", e.target.value)}
              placeholder="spam, drugs, ..."
              className="mt-1.5 w-full max-w-md rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50"
            />
            <p className="mt-1 text-xs text-muted-foreground">{t("reporter.modeBot")}</p>
          </div>
          )}

          {/* reasons (not for bot mode) */}
          {config.mode !== "bot" && (
          <div>
            <label className="text-sm font-medium text-foreground">{t("reporter.reasons")}</label>
            <div className="mt-2 space-y-2">
              <label className="flex items-center gap-3 cursor-pointer">
                <input type="checkbox" checked={config.randomReason} onChange={(e) => toggleRandomReason(e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
                <span className="text-sm font-medium text-primary">{t("reporter.randomReason")}</span>
              </label>
              <div className={`grid grid-cols-2 gap-2 ${config.randomReason ? "opacity-50" : ""}`}>
                {REASONS.map((r) => (
                  <label key={r.key} className={`flex items-center gap-3 ${config.randomReason ? "cursor-not-allowed" : "cursor-pointer"}`}>
                    <input
                      type="checkbox"
                      checked={config.reasons[r.key]}
                      onChange={(e) => { if (!config.randomReason) setReason(r.key, e.target.checked); }}
                      disabled={config.randomReason}
                      className="rounded border-border accent-primary h-4 w-4 disabled:opacity-50"
                    />
                    <span className="text-sm text-foreground">{r.label}</span>
                  </label>
                ))}
              </div>
            </div>
          </div>
          )}

          {/* delay */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("common.delayMin")} — {t("common.delayMax")} ({t("common.seconds")})</label>
            <div className="flex items-center gap-2 mt-1.5">
              <input
                type="number"
                min={2}
                max={30}
                value={config.delayMin}
                onChange={(e) => set("delayMin", Math.max(2, Math.min(30, Number(e.target.value))))}
                className="w-16 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50"
              />
              <span className="text-xs text-muted-foreground">—</span>
              <input
                type="number"
                min={2}
                max={30}
                value={config.delayMax}
                onChange={(e) => set("delayMax", Math.max(2, Math.min(30, Number(e.target.value))))}
                className="w-16 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50"
              />
            </div>
          </div>

          {/* limit per account */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("reporter.limitPerAccount")}</label>
            <input
              type="number"
              min={1}
              max={100}
              value={config.limitPerAccount}
              onChange={(e) => set("limitPerAccount", Math.max(1, Math.min(100, Number(e.target.value))))}
              className="mt-1.5 w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50"
            />
          </div>

          {/* channel mode: post targeting */}
          {config.mode === "channel" && (
          <div>
            <label className="text-sm font-medium text-foreground">{t("reporter.postTarget")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <ModeButton active={config.postTarget === "last"} onClick={() => set("postTarget", "last")}>{t("reporter.postTargetLast")}</ModeButton>
              <ModeButton active={config.postTarget === "all"} onClick={() => set("postTarget", "all")}>{t("reporter.postTargetAll")}</ModeButton>
            </div>
            {config.postTarget === "last" && (
              <div className="mt-2 flex items-center gap-2">
                <span className="text-xs text-muted-foreground">{t("reporter.postTargetLast")}:</span>
                <input type="number" min={1} max={500} value={config.postCount}
                  onChange={(e) => set("postCount", Math.max(1, Math.min(500, Number(e.target.value))))}
                  className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
              </div>
            )}
          </div>
          )}

          {/* photo mode: option */}
          {config.mode === "photo" && (
          <div>
            <label className="text-sm font-medium text-foreground">{t("reporter.modePhoto")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <ModeButton active={config.photoOption === "one"} onClick={() => set("photoOption", "one")}>{t("reporter.photoOptionOne")}</ModeButton>
              <ModeButton active={config.photoOption === "all"} onClick={() => set("photoOption", "all")}>{t("reporter.photoOptionAll")}</ModeButton>
            </div>
          </div>
          )}

          {/* view after report */}
          {config.mode === "channel" && (
          <label className="flex items-center gap-3 cursor-pointer">
            <input type="checkbox" checked={config.viewAfterReport} onChange={(e) => set("viewAfterReport", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
            <span className="text-sm font-medium text-foreground">{t("reporter.viewAfterReport")}</span>
          </label>
          )}

          {/* message (not for bot mode) */}
          {config.mode !== "bot" && (
          <div>
            <label className="text-sm font-medium text-foreground">{t("reporter.message")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <ModeButton active={config.messageMode === "none"} onClick={() => set("messageMode", "none")}>{t("reporter.messageNone")}</ModeButton>
              <ModeButton active={config.messageMode === "single"} onClick={() => set("messageMode", "single")}>{t("reporter.messageSingle")}</ModeButton>
              <ModeButton active={config.messageMode === "from_file"} onClick={() => set("messageMode", "from_file")}>{t("reporter.messageFromFile")}</ModeButton>
            </div>
            {config.messageMode === "single" && (
              <input
                value={config.messageSingle}
                onChange={(e) => set("messageSingle", e.target.value)}
                placeholder="..."
                className="mt-2 w-full max-w-md rounded-md border border-border bg-background px-3 py-1.5 text-sm outline-none focus:border-primary/50"
              />
            )}
            {config.messageMode === "from_file" && (
              <div className="mt-2 grid grid-cols-2 gap-2">
                {REASONS.filter((r) => config.randomReason || config.reasons[r.key]).map((r) => (
                  <button key={r.key} onClick={() => selectFile(r.key)} className="text-xs text-primary hover:underline text-left">
                    {config.messageFilePaths[r.key]
                      ? `${r.label}: ${(config.messageFilePaths[r.key] as string).split(/[/\\]/).pop()}`
                      : `${r.label}: ${t("common.selectFile")}`}
                  </button>
                ))}
              </div>
            )}
          </div>
          )}

        </div>
      </div>

      {/* controls */}
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-6 text-sm font-semibold">
          <span>{t("common.sent")}: <span className="text-[oklch(0.65_0.1_150)]">{stats.done}</span></span>
          <span>{t("common.errors")}: <span className="text-[oklch(0.55_0.1_25)]">{stats.errors}</span></span>
          <span>{t("common.total")}: <span className="text-[oklch(0.65_0.1_280)]">{Math.max(0, stats.inProgress - stats.done - stats.errors)}</span></span>
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
          <div className="flex items-center gap-2">
            <span className="text-xs text-muted-foreground">{t("common.threads")}:</span>
            <ThreadInput value={threads} onChange={setThreads} min={1} max={100} />
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

      {/* account picker */}
      <AccountPickerModal open={pickerOpen} onClose={() => setPickerOpen(false)} onSelect={handleAccountsSelected} />
    </div>
  );
}

function ModeTab({ active, onClick, children, disabled }: { active: boolean; onClick: () => void; children: React.ReactNode; disabled?: boolean }) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={`rounded-md border px-4 py-2 text-sm font-medium transition ${
        active
          ? "border-primary/50 bg-primary/10 text-primary"
          : disabled
          ? "border-border bg-background text-muted-foreground/50 cursor-not-allowed"
          : "border-border bg-background text-muted-foreground hover:border-primary/30"
      }`}
    >
      {children}
    </button>
  );
}

function ModeButton({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button onClick={onClick} className={`rounded-md border px-3 py-1 text-xs font-medium transition ${active ? "border-primary/50 bg-primary/10 text-primary" : "border-border bg-background text-muted-foreground hover:border-primary/30"}`}>
      {children}
    </button>
  );
}
