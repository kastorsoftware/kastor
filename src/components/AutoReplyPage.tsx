import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Play, Square, FolderOpen, FileText } from "lucide-react";
import { AccountPickerModal } from "@/components/AccountPickerModal";
import { MessageEditorModal } from "@/components/MessageEditorModal";
import { ThreadInput } from "@/components/ThreadInput";
import { useT } from "@/i18n";
import { isDone, isError, isReplySent, isSkipped } from "@/lib/eventParser";

type DelayUnit = "seconds" | "minutes";
type ReplyMode = "infinite" | "limit" | "whitelist";
type MessageType = "text" | "forward" | "voice";
type TextModify = "none" | "llm_rewrite" | "randomize";

interface AutoReplyConfig {
  delayMin: number;
  delayMax: number;
  delayUnit: DelayUnit;

  messageType: MessageType;
  replyText: string;
  imagePath: string;
  videoPath: string;
  textModify: TextModify;
  useVoice: boolean;
  voicePath: string;
  forwardMsgId: string;

  maxFloodWait: number;
  threads: number;

  banWords: string;

  replyMode: ReplyMode;
  replyLimit: number;
  whitelistPath: string;

  keepOnline: boolean;
  silent: boolean;
  noWebpage: boolean;
  markRead: boolean;

  autostopEnabled: boolean;
  autostopBan: number;
  autostopSpamblock: number;
  autostopFlood: number;

  outputPath: string;
}

const defaultConfig: AutoReplyConfig = {
  delayMin: 0,
  delayMax: 0,
  delayUnit: "seconds",

  messageType: "text",
  replyText: "",
  imagePath: "",
  videoPath: "",
  textModify: "none",
  useVoice: false,
  voicePath: "",
  forwardMsgId: "",

  maxFloodWait: 60,
  threads: 5,

  banWords: "",

  replyMode: "infinite",
  replyLimit: 100,
  whitelistPath: "",

  keepOnline: false,
  silent: false,
  noWebpage: false,
  markRead: true,

  autostopEnabled: false,
  autostopBan: 3,
  autostopSpamblock: 3,
  autostopFlood: 5,

  outputPath: "",
};

const STORAGE_KEY = "auto_reply_config";
const IS_DEV = !("__TAURI_INTERNALS__" in window);

function loadSavedConfig(): AutoReplyConfig {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return { ...defaultConfig, ...JSON.parse(saved) };
  } catch {}
  return defaultConfig;
}

export function AutoReplyPage() {
  const t = useT();
  const [config, setConfig] = useState<AutoReplyConfig>(loadSavedConfig);
  const [running, setRunning] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [stats, setStats] = useState({ replied: 0, skipped: 0, errors: 0 });
  const [pickerOpen, setPickerOpen] = useState(false);
  const [messageEditorOpen, setMessageEditorOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const logsRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(config));
  }, [config]);

  useEffect(() => {
    if (IS_DEV) return;
    const unlisten = listen<string>("auto-reply-log", (e) => {
      setLogs((prev) => [...prev, e.payload]);
      const msg = e.payload;
      if (isDone(msg)) {
        setRunning(false);
      } else if (isError(msg)) {
        setStats((s) => ({ ...s, errors: s.errors + 1 }));
      } else if (isReplySent(msg)) {
        setStats((s) => ({ ...s, replied: s.replied + 1 }));
      } else if (isSkipped(msg)) {
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

  const set = <K extends keyof AutoReplyConfig>(key: K, value: AutoReplyConfig[K]) => {
    setConfig((prev) => ({ ...prev, [key]: value }));
    setError(null);
  };

  const selectVoice = async () => {
    if (IS_DEV) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const selected = await open({
      multiple: false,
      filters: [{ name: "Аудио", extensions: ["ogg", "mp3", "wav", "m4a"] }],
    });
    if (selected) set("voicePath", selected as string);
  };

  const selectWhitelist = async () => {
    if (IS_DEV) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const selected = await open({
      multiple: false,
      filters: [{ name: "Текстовые файлы", extensions: ["txt"] }],
    });
    if (selected) set("whitelistPath", selected as string);
  };

  const selectOutput = async () => {
    if (IS_DEV) return;
    const { save } = await import("@tauri-apps/plugin-dialog");
    const path = await save({ defaultPath: "auto_reply.db", filters: [{ name: "SQLite DB", extensions: ["db"] }] });
    if (path) set("outputPath", path);
  };

  const validate = (): string | null => {
    if (config.messageType === "text" && !config.replyText.trim() && !config.imagePath && !config.videoPath) return t("validation.specifyCommentText");
    if (config.messageType === "voice" && !config.voicePath.trim()) return t("validation.selectVoiceFile");
    if (config.messageType === "forward" && !config.forwardMsgId.trim()) return t("validation.enterForwardMsgId");
    if (config.replyMode === "whitelist" && !config.whitelistPath.trim()) return t("validation.selectInputFile");
    if (config.replyMode === "limit" && config.replyLimit < 1) return t("validation.minGreaterThanMax");
    if (config.delayMin > config.delayMax) return t("validation.delayMinGreaterMax");
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
    setStats({ replied: 0, skipped: 0, errors: 0 });
    setRunning(true);
    setError(null);
    try {
      const tid = await invoke<string>("auto_reply_start", {
        ids,
        config: {
          ...config,
          message_type: config.messageType,
          reply_text: config.replyText.trim(),
          image_path: config.messageType === "text" ? config.imagePath : "",
          video_path: config.messageType === "text" ? config.videoPath : "",
          text_modify: config.textModify,
          voice_path: config.messageType === "voice" ? config.voicePath.trim() : "",
          forward_msg_id: config.messageType === "forward" ? config.forwardMsgId.trim() : "",
          whitelist_path: config.replyMode === "whitelist" ? config.whitelistPath.trim() : "",
          max_flood_wait: Number(config.maxFloodWait) || 0,
          delay_min: config.delayMin,
          delay_max: config.delayMax,
          delay_unit: config.delayUnit,
          ban_words: config.banWords,
          reply_mode: config.replyMode,
          reply_limit: config.replyLimit,
          keep_online: config.keepOnline,
          silent: config.silent,
          no_webpage: config.noWebpage,
          mark_read: config.markRead,
          autostop_enabled: config.autostopEnabled,
          autostop_ban: config.autostopBan,
          autostop_spamblock: config.autostopSpamblock,
          autostop_flood: config.autostopFlood,
          output_path: config.outputPath.trim(),
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
    if (!IS_DEV && taskId) await invoke("auto_reply_stop", { taskId }).catch(() => {});
    setRunning(false);
    setLogs((prev) => [...prev, t("common.stoppedByUser")]);
  };

  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">{t("descriptions.autoReply")}</p>
      {/* config panel */}
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div className="p-6 space-y-5">

          {/* delay */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("autoReply.delayLabel")}</label>
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
            <p className="text-xs text-muted-foreground mt-1">{t("autoReply.delayHint")}</p>
          </div>

          {/* message type */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("autoReply.messageType")}</label>
            <div className="flex flex-wrap items-center gap-2 mt-1.5">
              <ModeButton active={config.messageType === "text"} onClick={() => set("messageType", "text")}>{t("autoReply.typeText")}</ModeButton>
              <ModeButton active={config.messageType === "forward"} onClick={() => set("messageType", "forward")}>{t("autoReply.typeForward")}</ModeButton>
              <ModeButton active={config.messageType === "voice"} onClick={() => set("messageType", "voice")}>{t("autoReply.typeVoice")}</ModeButton>
            </div>
          </div>

          {/* message content based on type */}
          {config.messageType === "text" && (
            <div className="space-y-3">
              <button
                onClick={() => setMessageEditorOpen(true)}
                className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-left text-sm hover:border-primary/50 transition w-full max-w-md"
              >
                <FileText className="h-4 w-4 text-muted-foreground shrink-0" />
                {config.replyText ? (
                  <span className="truncate text-foreground flex-1">
                    {config.replyText.replace(/[*_~|`+\[\]()]/g, "").split("\n")[0].slice(0, 60) || t("messageEditor.empty")}
                  </span>
                ) : (
                  <span className="text-muted-foreground flex-1">{t("autoReply.editReplyPlaceholder")}</span>
                )}
              </button>
              <p className="text-xs text-muted-foreground">{t("autoReply.placeholders")}</p>
              <div className="flex items-center gap-2">
                <ModeButton active={config.textModify === "none"} onClick={() => set("textModify", "none")}>{t("autoReply.textModifyNone")}</ModeButton>
                <ModeButton active={config.textModify === "llm_rewrite"} onClick={() => set("textModify", "llm_rewrite")}>{t("autoReply.textModifyLlm")}</ModeButton>
                <ModeButton active={config.textModify === "randomize"} onClick={() => set("textModify", "randomize")}>{t("autoReply.textModifyRandomize")}</ModeButton>
              </div>
            </div>
          )}
          {config.messageType === "forward" && (
            <div>
              <label className="text-sm font-medium text-foreground">{t("autoReply.forwardIdLabel")}</label>
              <input value={config.forwardMsgId} onChange={(e) => set("forwardMsgId", e.target.value)} placeholder="12345" className="mt-1.5 w-full max-w-xs rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50" />
              <p className="text-xs text-muted-foreground mt-1">{t("autoReply.forwardIdHint")}</p>
            </div>
          )}
          {config.messageType === "voice" && (
            <div className="flex items-center gap-2">
              <button onClick={selectVoice} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition">
                <FolderOpen className="h-4 w-4" /> {t("autoReply.selectAudio")}
              </button>
              <span className="text-xs text-muted-foreground truncate max-w-sm">{config.voicePath ? config.voicePath.split(/[/\\]/).pop() : t("common.notSelected")}</span>
            </div>
          )}

          {/* ban words */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("autoReply.banWordsTitle")}</label>
            <textarea
              value={config.banWords}
              onChange={(e) => set("banWords", e.target.value)}
              placeholder={t("autoReply.banWordsPlaceholder")}
              rows={3}
              className="mt-1.5 w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50 resize-y"
            />
            <p className="text-xs text-muted-foreground mt-1">{t("autoReply.banWordsHint")}</p>
          </div>

          {/* reply mode */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("autoReply.replyModeLabel")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <ModeButton active={config.replyMode === "infinite"} onClick={() => set("replyMode", "infinite")}>{t("autoReply.replyInfiniteBtn")}</ModeButton>
              <ModeButton active={config.replyMode === "limit"} onClick={() => set("replyMode", "limit")}>{t("autoReply.replyLimitBtn")}</ModeButton>
              <ModeButton active={config.replyMode === "whitelist"} onClick={() => set("replyMode", "whitelist")}>{t("autoReply.replyWhitelistBtn")}</ModeButton>
            </div>
            {config.replyMode === "limit" && (
              <div className="flex items-center gap-2 mt-2">
                <span className="text-xs text-muted-foreground">{t("autoReply.replyLimitMax")}</span>
                <input
                  type="number"
                  min={1}
                  max={999999}
                  value={config.replyLimit}
                  onChange={(e) => set("replyLimit", Math.max(1, Number(e.target.value)))}
                  className="w-24 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50"
                />
                <span className="text-xs text-muted-foreground">{t("autoReply.replyLimitPeople")}</span>
              </div>
            )}
            {config.replyMode === "whitelist" && (
              <div className="mt-2 flex items-center gap-2">
                <button
                  onClick={selectWhitelist}
                  className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition"
                >
                  <FolderOpen className="h-4 w-4" />
                  {t("autoReply.whitelistFile")}
                </button>
                <span className="text-xs text-muted-foreground truncate max-w-sm">
                  {config.whitelistPath ? config.whitelistPath.split(/[/\\]/).pop() : t("common.notSelected")}
                </span>
              </div>
            )}
          </div>

          {/* autostop */}
          <div className="border-t border-border pt-4">
            <label className="flex items-center gap-3 cursor-pointer">
              <input type="checkbox" checked={config.autostopEnabled} onChange={(e) => set("autostopEnabled", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
              <span className="text-sm font-medium text-foreground">{t("autoReply.autostopLabel")}</span>
            </label>
            {config.autostopEnabled && (
              <div className="ml-7 mt-2 space-y-2">
                <div className="flex items-center gap-2">
                  <span className="text-xs text-muted-foreground w-32">{t("autoReply.autostopBans")}</span>
                  <input type="number" min={0} max={999} value={config.autostopBan} onChange={(e) => set("autostopBan", Math.max(0, Number(e.target.value)))} className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
                </div>
                <div className="flex items-center gap-2">
                  <span className="text-xs text-muted-foreground w-32">{t("autoReply.autostopSpamblocks")}</span>
                  <input type="number" min={0} max={999} value={config.autostopSpamblock} onChange={(e) => set("autostopSpamblock", Math.max(0, Number(e.target.value)))} className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
                </div>
                <div className="flex items-center gap-2">
                  <span className="text-xs text-muted-foreground w-32">{t("autoReply.autostopFloodwaits")}</span>
                  <input type="number" min={0} max={999} value={config.autostopFlood} onChange={(e) => set("autostopFlood", Math.max(0, Number(e.target.value)))} className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
                </div>
                <p className="text-xs text-muted-foreground">{t("autoReply.autostopHint")}</p>
              </div>
            )}
          </div>

          {/* flood wait + options */}
          <div className="border-t border-border pt-4 space-y-3">
            <div>
              <label className="text-sm font-medium text-foreground">{t("autoReply.outputFileLabel")}</label>
              <p className="text-xs text-muted-foreground mb-1.5">{t("autoReply.outputFileHint")}</p>
              <div className="flex items-center gap-2 mt-1.5">
                <button onClick={selectOutput} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition">
                  <FolderOpen className="h-4 w-4" /> {t("common.selectFile")}
                </button>
                <span className="text-xs text-muted-foreground truncate max-w-sm">{config.outputPath ? config.outputPath.split(/[/\\]/).pop() : t("common.notSelected")}</span>
              </div>
            </div>
            <div>
              <label className="text-sm font-medium text-foreground">{t("autoReply.floodWaitLabel")}</label>
              <div className="flex items-center gap-2 mt-1.5">
                <input
                  type="number"
                  min={0}
                  max={86400}
                  value={config.maxFloodWait}
                  onChange={(e) => set("maxFloodWait", Math.max(0, Math.min(86400, Number(e.target.value))))}
                  className="w-28 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50"
                />
                <span className="text-xs text-muted-foreground">{t("autoReply.floodWaitZero")}</span>
              </div>
            </div>
            <label className="flex items-center gap-3 cursor-pointer">
              <input type="checkbox" checked={config.keepOnline} onChange={(e) => set("keepOnline", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
              <span className="text-sm font-medium text-foreground">{t("autoReply.keepOnlineLabel")}</span>
            </label>
            <label className="flex items-center gap-3 cursor-pointer">
              <input type="checkbox" checked={config.silent} onChange={(e) => set("silent", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
              <span className="text-sm font-medium text-foreground">{t("autoReply.silentLabel")}</span>
            </label>
            <label className="flex items-center gap-3 cursor-pointer">
              <input type="checkbox" checked={config.noWebpage} onChange={(e) => set("noWebpage", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
              <span className="text-sm font-medium text-foreground">{t("autoReply.noWebpageLabel")}</span>
            </label>
            <label className="flex items-center gap-3 cursor-pointer">
              <input type="checkbox" checked={config.markRead} onChange={(e) => set("markRead", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
              <span className="text-sm font-medium text-foreground">{t("autoReply.markReadLabel")}</span>
            </label>
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
          <span>{t("autoReply.replied")}: <span className="text-[oklch(0.65_0.1_150)]">{stats.replied}</span></span>
          <span>{t("autoReply.skipped")}: <span className="text-[oklch(0.65_0.1_60)]">{stats.skipped}</span></span>
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
        initialValue={config.replyText}
        initialImagePath={config.imagePath}
        initialVideoPath={config.videoPath}
        title={t("autoReply.editorTitle")}
        withImage
        withVideo
        onClose={() => setMessageEditorOpen(false)}
        onSave={(text, imagePath, videoPath) => {
          set("replyText", text);
          set("imagePath", imagePath || "");
          set("videoPath", videoPath || "");
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
