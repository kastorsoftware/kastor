import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Play, Square, FileText, Image as ImageIcon, Video as VideoIcon } from "lucide-react";
import { AccountPickerModal } from "@/components/AccountPickerModal";
import { MessageEditorModal } from "@/components/MessageEditorModal";
import { ThreadInput } from "@/components/ThreadInput";
import { useT } from "@/i18n";
import { isDone, isError, isCommentSent } from "@/lib/eventParser";

type DelayUnit = "seconds" | "minutes";
type ReplyMode = "static" | "llm";
type TargetMode = "channels" | "subscribed";

interface FirstCommentConfig {
  targetMode: TargetMode;
  targets: string;
  delayMin: number;
  delayMax: number;
  delayUnit: DelayUnit;
  replyMode: ReplyMode;
  staticText: string;
  staticImagePath: string;
  staticVideoPath: string;
  randomizeStatic: boolean;
  llmPrompt: string;
  maxFloodWait: number;
  threads: number;
}

const defaultConfig: FirstCommentConfig = {
  targetMode: "channels",
  targets: "",
  delayMin: 0,
  delayMax: 3,
  delayUnit: "seconds",
  replyMode: "static",
  staticText: "",
  staticImagePath: "",
  staticVideoPath: "",
  randomizeStatic: false,
  llmPrompt: "",
  maxFloodWait: 60,
  threads: 5,
};

const STORAGE_KEY = "first_comment_config";
const IS_DEV = !("__TAURI_INTERNALS__" in window);

function loadSavedConfig(): FirstCommentConfig {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return { ...defaultConfig, ...JSON.parse(saved) };
  } catch {}
  return defaultConfig;
}

export function FirstCommentPage() {
  const t = useT();
  const [config, setConfig] = useState<FirstCommentConfig>(loadSavedConfig);
  const [running, setRunning] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [stats, setStats] = useState({ comments: 0, errors: 0 });
  const [pickerOpen, setPickerOpen] = useState(false);
  const [messageEditorOpen, setMessageEditorOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const logsRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(config));
  }, [config]);

  useEffect(() => {
    if (IS_DEV) return;
    const unlisten = listen<string>("first-comment-log", (e) => {
      setLogs((prev) => [...prev, e.payload]);
      const msg = e.payload;
      if (isDone(msg)) {
        setRunning(false);
      } else if (isError(msg)) {
        setStats((s) => ({ ...s, errors: s.errors + 1 }));
      } else if (isCommentSent(msg)) {
        setStats((s) => ({ ...s, comments: s.comments + 1 }));
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

  const set = <K extends keyof FirstCommentConfig>(key: K, value: FirstCommentConfig[K]) => {
    setConfig((prev) => ({ ...prev, [key]: value }));
    setError(null);
  };

  const validate = (): string | null => {
    if (config.targetMode === "channels" && !config.targets.trim()) return t("validation.specifyChannel");
    if (config.replyMode === "static" && !config.staticText.trim() && !config.staticImagePath && !config.staticVideoPath) return t("validation.specifyCommentText");
    if (config.replyMode === "llm" && !config.llmPrompt.trim()) return t("validation.specifyLlmPrompt");
    return null;
  };

  const handleStart = () => {
    const err = validate();
    if (err) {
      setError(err);
      return;
    }
    setPickerOpen(true);
  };

  const handleAccountsSelected = async (ids: string[]) => {
    if (ids.length === 0) return;
    setLogs([]);
    setStats({ comments: 0, errors: 0 });
    setRunning(true);
    setError(null);
    try {
      const tid = await invoke<string>("first_comment_start", {
        ids,
        config: {
          target_mode: config.targetMode,
          targets: config.targets.trim().split("\n").map(s => s.trim()).filter(s => s.length > 0),
          delay_min: config.delayMin,
          delay_max: config.delayMax,
          delay_unit: config.delayUnit,
          reply_mode: config.replyMode,
          static_text: config.staticText.trim(),
          static_image_path: config.staticImagePath,
          static_video_path: config.staticVideoPath,
          randomize_static: config.randomizeStatic,
          llm_prompt: config.llmPrompt.trim(),
          max_flood_wait: config.maxFloodWait,
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
    if (!IS_DEV && taskId) await invoke("first_comment_stop", { taskId }).catch(() => {});
    setRunning(false);
    setLogs((prev) => [...prev, t("common.stoppedByUser")]);
  };

  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">{t("descriptions.firstComment")}</p>
      {/* config panel */}
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div className="p-6 space-y-5">

          {/* target mode */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("firstComment.targetModeLabel")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <ModeButton active={config.targetMode === "channels"} onClick={() => set("targetMode", "channels")}>{t("firstComment.targetChannelsBtn")}</ModeButton>
              <ModeButton active={config.targetMode === "subscribed"} onClick={() => set("targetMode", "subscribed")}>{t("firstComment.targetSubscribedBtn")}</ModeButton>
            </div>
          </div>

          {/* target channels list */}
          {config.targetMode === "channels" && (
            <div>
              <label className="text-sm font-medium text-foreground">{t("firstComment.channelsMonitor")}</label>
              <textarea
                value={config.targets}
                onChange={(e) => set("targets", e.target.value)}
                placeholder={"https://t.me/channel1\n@channel2\nt.me/+invite3"}
                rows={4}
                className="mt-1.5 w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50 resize-y"
              />
              <p className="text-xs text-muted-foreground mt-1">{t("firstComment.channelsMonitorHint")}</p>
            </div>
          )}

          {config.targetMode === "subscribed" && (
            <p className="text-xs text-muted-foreground">{t("firstComment.subscribedHint")}</p>
          )}

          {/* delay */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("firstComment.delayAfterPost")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <input
                type="number"
                min={0}
                max={9999}
                value={config.delayMin}
                onChange={(e) => set("delayMin", Math.max(0, Number(e.target.value)))}
                className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50"
              />
              <span className="text-muted-foreground text-sm">—</span>
              <input
                type="number"
                min={0}
                max={9999}
                value={config.delayMax}
                onChange={(e) => set("delayMax", Math.max(0, Number(e.target.value)))}
                className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50"
              />
              <select
                value={config.delayUnit}
                onChange={(e) => set("delayUnit", e.target.value as DelayUnit)}
                className="rounded-md border border-border bg-card px-2.5 py-1.5 text-sm outline-none focus:border-primary/50 appearance-none bg-[url('data:image/svg+xml;charset=utf-8,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%20width%3D%2216%22%20height%3D%2216%22%20viewBox%3D%220%200%2024%2024%22%20fill%3D%22none%22%20stroke%3D%22%23888%22%20stroke-width%3D%222%22%3E%3Cpath%20d%3D%22m6%209%206%206%206-6%22%2F%3E%3C%2Fsvg%3E')] bg-[length:16px] bg-[right_8px_center] bg-no-repeat pr-8"
              >
                <option value="seconds">{t("common.seconds")}</option>
                <option value="minutes">{t("common.minutes")}</option>
              </select>
            </div>
            <p className="text-xs text-muted-foreground mt-1">{t("firstComment.delayHint")}</p>
          </div>

          {/* reply mode */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("firstComment.commentMode")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <ModeButton active={config.replyMode === "static"} onClick={() => set("replyMode", "static")}>{t("firstComment.staticTextBtn")}</ModeButton>
              <ModeButton active={config.replyMode === "llm"} onClick={() => set("replyMode", "llm")}>{t("firstComment.llmBtn")}</ModeButton>
            </div>
          </div>

          {/* static text */}
          {config.replyMode === "static" && (
            <div className="space-y-3">
              <label className="text-sm font-medium text-foreground">{t("firstComment.commentText")}</label>
              <button
                onClick={() => setMessageEditorOpen(true)}
                className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-left text-sm hover:border-primary/50 transition w-full max-w-md"
              >
                <FileText className="h-4 w-4 text-muted-foreground shrink-0" />
                {config.staticText ? (
                  <span className="truncate text-foreground flex-1">
                    {config.staticText.replace(/[*_~|`+\[\]()]/g, "").split("\n")[0].slice(0, 60) || t("firstComment.mediaNoText")}
                  </span>
                ) : (
                  <span className="text-muted-foreground flex-1">{t("firstComment.editComment")}</span>
                )}
                {config.staticImagePath && <ImageIcon className="h-3.5 w-3.5 text-primary" />}
                {config.staticVideoPath && <VideoIcon className="h-3.5 w-3.5 text-primary" />}
              </button>
              <label className="flex items-center gap-2.5 cursor-pointer text-sm">
                <input
                  type="checkbox"
                  checked={config.randomizeStatic}
                  onChange={(e) => set("randomizeStatic", e.target.checked)}
                  className="rounded border-border accent-primary h-4 w-4"
                />
                <span className="text-foreground font-medium">{t("firstComment.randomizeLabel")}</span>
              </label>
              <p className="text-xs text-muted-foreground">{t("firstComment.randomizeHint")}</p>
            </div>
          )}

          {/* llm prompt */}
          {config.replyMode === "llm" && (
            <div>
              <label className="text-sm font-medium text-foreground">{t("firstComment.llmPromptLabel")}</label>
              <input
                value={config.llmPrompt}
                onChange={(e) => set("llmPrompt", e.target.value)}
                placeholder="@channel или https://t.me/channel"
                className="mt-1.5 w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50"
              />
              <p className="text-xs text-muted-foreground mt-1">
                {t("firstComment.llmPromptHint")}
              </p>
            </div>
          )}

          {/* flood wait */}
          <div className="border-t border-border pt-4">
            <label className="text-sm font-medium text-foreground">{t("firstComment.floodWaitLabel")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <input
                type="number"
                min={0}
                max={86400}
                value={config.maxFloodWait}
                onChange={(e) => set("maxFloodWait", Math.max(0, Math.min(86400, Number(e.target.value))))}
                className="w-28 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50"
              />
              <span className="text-xs text-muted-foreground">{t("firstComment.floodWaitZero")}</span>
            </div>
          </div>

        </div>
      </div>

      {/* error */}
      {error && (
        <div className="flex items-center gap-3 rounded-md border border-destructive/30 bg-destructive/5 px-4 py-3 text-sm">
          <span className="text-destructive">{error}</span>
          <button onClick={() => setError(null)} className="ml-auto text-muted-foreground hover:text-foreground">✕</button>
        </div>
      )}

      {/* controls */}
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-6 text-sm font-semibold">
          <span>{t("firstComment.comments")}: <span className="text-[oklch(0.65_0.1_150)]">{stats.comments}</span></span>
          <span>{t("common.errors")}: <span className="text-[oklch(0.55_0.1_25)]">{stats.errors}</span></span>
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
            <ThreadInput value={config.threads} onChange={(v) => set("threads", v)} min={1} max={100} />
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

      <AccountPickerModal open={pickerOpen} onClose={() => setPickerOpen(false)} onSelect={handleAccountsSelected} />
      <MessageEditorModal
        open={messageEditorOpen}
        initialValue={config.staticText}
        initialImagePath={config.staticImagePath}
        initialVideoPath={config.staticVideoPath}
        title={t("firstComment.editorTitle")}
        withImage
        withVideo
        onClose={() => setMessageEditorOpen(false)}
        onSave={(text, imagePath, videoPath) => {
          set("staticText", text);
          set("staticImagePath", imagePath || "");
          set("staticVideoPath", videoPath || "");
        }}
      />
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
