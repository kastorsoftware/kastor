import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Play, Square, FolderOpen, FileText, Image as ImageIcon } from "lucide-react";
import { AccountPickerModal } from "@/components/AccountPickerModal";
import { MessageEditorModal } from "@/components/MessageEditorModal";
import { ThreadInput } from "@/components/ThreadInput";
import { useT } from "@/i18n";

type MailingMode = "dialogs" | "contacts" | "usernames" | "chats" | "comments" | "stories" | "phones";
type MessageType = "text" | "postbot" | "forward" | "voice";
type TextModify = "none" | "llm_rewrite" | "randomize";

interface MailingConfig {
  mode: MailingMode;
  messageType: MessageType;
  messageText: string;
  messageImagePath: string;
  messageVideoPath: string;
  textModify: TextModify;
  postbotHash: string;
  forwardMsgId: string;
  voicePath: string;
  usernamesPath: string;
  chatsList: string;
  commentsTarget: string;
  maxPerAccount: number;
  maxFloodWait: number;
  threads: number;
  silent: boolean;
  scheduledTime: string;
  noWebpage: boolean;
  deleteDialog: boolean;
  pinMessage: boolean;
  kolMin: number;
  kolMax: number;
  storyLink: string;
  phonesPath: string;
  videoNote: boolean;
  fileTtl: number;
  autostopEnabled: boolean;
  autostopBan: number;
  autostopSpamblock: number;
  autostopFlood: number;
  autoRepost: boolean;
  outputPath: string;
}

const defaultConfig: MailingConfig = {
  mode: "dialogs",
  messageType: "text",
  messageText: "",
  messageImagePath: "",
  messageVideoPath: "",
  textModify: "none",
  postbotHash: "",
  forwardMsgId: "",
  voicePath: "",
  usernamesPath: "",
  chatsList: "",
  commentsTarget: "",
  maxPerAccount: 50,
  maxFloodWait: 60,
  threads: 5,
  silent: false,
  scheduledTime: "",
  noWebpage: true,
  deleteDialog: false,
  pinMessage: false,
  kolMin: 0,
  kolMax: 0,
  storyLink: "",
  phonesPath: "",
  videoNote: false,
  fileTtl: 0,
  autostopEnabled: false,
  autostopBan: 3,
  autostopSpamblock: 3,
  autostopFlood: 5,
  autoRepost: false,
  outputPath: "",
};

const STORAGE_KEY = "mailing_config";
const IS_DEV = !("__TAURI_INTERNALS__" in window);

function loadSavedConfig(): MailingConfig {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return { ...defaultConfig, ...JSON.parse(saved) };
  } catch {}
  return defaultConfig;
}

export function MailingPage() {
  const t = useT();
  const [config, setConfig] = useState<MailingConfig>(loadSavedConfig);
  const [running, setRunning] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [stats, setStats] = useState({ sent: 0, errors: 0 });
  const [pickerOpen, setPickerOpen] = useState(false);
  const [messageEditorOpen, setMessageEditorOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const logsRef = useRef<HTMLDivElement>(null);

  useEffect(() => { localStorage.setItem(STORAGE_KEY, JSON.stringify(config)); }, [config]);

  useEffect(() => {
    if (IS_DEV) return;
    const unlisten = listen<string>("mailing-log", (e) => {
      setLogs((prev) => [...prev, e.payload]);
      const msg = e.payload;
      if (msg === "Завершено" || msg === "Done") setRunning(false);
      else if (msg.includes("отправлено") || msg.includes("переслано") || msg.includes("sent") || msg.includes("forwarded")) setStats((s) => ({ ...s, sent: s.sent + 1 }));
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

  const set = <K extends keyof MailingConfig>(key: K, value: MailingConfig[K]) => {
    setConfig((prev) => ({ ...prev, [key]: value }));
    setError(null);
  };

  const selectFile = async (key: "usernamesPath" | "voicePath" | "phonesPath") => {
    if (IS_DEV) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const ext = key === "voicePath" ? [{ name: t("mailing.typeVoice"), extensions: ["ogg", "mp3", "wav", "m4a"] }] : [{ name: "Text", extensions: ["txt"] }];
    const path = await open({ multiple: false, filters: ext });
    if (path) set(key, path as string);
  };

  const selectOutputFile = async () => {
    if (IS_DEV) return;
    const { save } = await import("@tauri-apps/plugin-dialog");
    const path = await save({ defaultPath: "mailing.db", filters: [{ name: "SQLite DB", extensions: ["db"] }] });
    if (path) set("outputPath", path);
  };

  const validate = (): string | null => {
    if (config.mode === "usernames" && !config.usernamesPath) return t("validation.selectUsernamesFile");
    if (config.mode === "chats" && !config.chatsList.trim()) return t("validation.specifyChatList");
    if (config.mode === "comments" && !config.commentsTarget.trim()) return t("validation.specifyCommentsChannel");
    if (config.mode === "stories" && !config.storyLink.trim()) return t("validation.specifyStoryLink");
    if (config.mode === "stories" && !config.usernamesPath) return t("validation.selectUsernamesForStories");
    if (config.mode === "phones" && !config.phonesPath) return t("validation.selectPhonesFile");
    if (config.messageType === "text" && !config.messageText.trim()) return t("validation.enterMessageText");
    if (config.messageType === "postbot" && !config.postbotHash.trim()) return t("validation.enterPostbotHash");
    if (config.messageType === "forward" && !config.forwardMsgId.trim()) return t("validation.enterForwardMsgId");
    if (config.messageType === "voice" && !config.voicePath) return t("validation.selectVoiceFile");
    if (config.scheduledTime) {
      const parts = config.scheduledTime.split(" ");
      if (parts.length !== 2) return t("validation.invalidDateFormat");
      const [d, m, y] = parts[0].split(".");
      const [hh, mm] = parts[1].split(":");
      if (!d || !m || !y || !hh || !mm) return t("validation.invalidDateFormat");
      const date = new Date(Number(y), Number(m) - 1, Number(d), Number(hh), Number(mm));
      if (isNaN(date.getTime())) return t("validation.invalidDate");
      const now = new Date();
      const minTime = new Date(now.getTime() + 60_000);
      const maxTime = new Date(now.getTime() + 365 * 24 * 60 * 60 * 1000);
      if (date <= minTime) return t("validation.dateMinOneMinute");
      if (date > maxTime) return t("validation.dateMaxOneYear");
    }
    return null;
  };

  const handleStart = () => {
    const err = validate();
    if (err) { setError(err); return; }
    setPickerOpen(true);
  };

  const handleAccountsSelected = async (ids: string[]) => {
    if (ids.length === 0) return;
    setLogs([]); setStats({ sent: 0, errors: 0 }); setRunning(true); setError(null);
    try {
      const tid = await invoke<string>("mailing_start", {
        ids, config: {
          mode: config.mode, message_type: config.messageType,
          message_text: config.messageText.trim(), text_modify: config.textModify,
          message_image_path: config.messageImagePath,
          message_video_path: config.messageVideoPath,
          postbot_hash: config.postbotHash.trim(), forward_msg_id: config.forwardMsgId.trim(),
          voice_path: config.messageType === "voice" ? config.voicePath : "",
          usernames_path: config.mode === "usernames" ? config.usernamesPath : "",
          chats_list: config.mode === "chats" ? config.chatsList.trim() : "",
          comments_target: config.mode === "comments" ? config.commentsTarget.trim() : "",
          max_per_account: config.maxPerAccount, max_flood_wait: config.maxFloodWait,
          silent: config.silent, scheduled_time: config.scheduledTime, no_webpage: config.noWebpage,
          delete_dialog: config.deleteDialog, pin_message: config.pinMessage,
          kol_min: config.kolMin, kol_max: config.kolMax,
          story_link: config.mode === "stories" ? config.storyLink.trim() : "",
          phones_path: config.mode === "phones" ? config.phonesPath : "",
          video_note: config.videoNote,
          file_ttl: config.fileTtl,
          autostop_enabled: config.autostopEnabled,
          autostop_ban: config.autostopBan,
          autostop_spamblock: config.autostopSpamblock,
          autostop_flood: config.autostopFlood,
          auto_repost: config.autoRepost,
          output_path: config.outputPath.trim(),
        }, threads: config.threads,
      });
      setTaskId(tid);
    } catch (e: any) { setLogs((prev) => [...prev, `${t("common.error")}: ${e}`]); setRunning(false); }
  };

  const handleStop = async () => {
    if (!IS_DEV && taskId) await invoke("mailing_stop", { taskId }).catch(() => {});
    setRunning(false); setLogs((prev) => [...prev, t("common.stoppedByUser")]);
  };

  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">{t("descriptions.mailing")}</p>
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div className="p-6 space-y-5">

          {/* mode */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("mailing.title")}</label>
            <div className="flex flex-wrap items-center gap-2 mt-1.5">
              <ModeButton active={config.mode === "dialogs"} onClick={() => set("mode", "dialogs")}>{t("mailing.modeDialogs")}</ModeButton>
              <ModeButton active={config.mode === "contacts"} onClick={() => set("mode", "contacts")}>{t("mailing.modeContacts")}</ModeButton>
              <ModeButton active={config.mode === "usernames"} onClick={() => set("mode", "usernames")}>{t("mailing.modeUsernames")}</ModeButton>
              <ModeButton active={config.mode === "chats"} onClick={() => set("mode", "chats")}>{t("mailing.modeChats")}</ModeButton>
              <ModeButton active={config.mode === "comments"} onClick={() => set("mode", "comments")}>{t("mailing.modeComments")}</ModeButton>
              <ModeButton active={config.mode === "stories"} onClick={() => set("mode", "stories")}>{t("mailing.modeStories")}</ModeButton>
              <ModeButton active={config.mode === "phones"} onClick={() => set("mode", "phones")}>{t("mailing.modePhones")}</ModeButton>
            </div>
          </div>

          {/* mode-specific inputs */}
          {config.mode === "usernames" && (
            <div className="flex items-center gap-2">
              <button onClick={() => selectFile("usernamesPath")} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition">
                <FolderOpen className="h-4 w-4" /> {t("mailing.modeUsernames")}
              </button>
              <span className="text-xs text-muted-foreground truncate max-w-sm">{config.usernamesPath ? config.usernamesPath.split(/[/\\]/).pop() : t("common.notSelected")}</span>
            </div>
          )}
          {config.mode === "chats" && (
            <div>
              <label className="text-sm font-medium text-foreground">{t("mailing.modeChats")}</label>
              <textarea value={config.chatsList} onChange={(e) => set("chatsList", e.target.value)} placeholder={"@chat1\nhttps://t.me/chat2\nt.me/+invite\nt.me/addlist/slug"} rows={4} className="mt-1.5 w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50 resize-y" />
            </div>
          )}
          {config.mode === "comments" && (
            <div>
              <label className="text-sm font-medium text-foreground">{t("mailing.modeComments")}</label>
              <textarea value={config.commentsTarget} onChange={(e) => set("commentsTarget", e.target.value)} placeholder={"@channel1\nhttps://t.me/channel2\nt.me/+invite"} rows={3} className="mt-1.5 w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50 resize-y" />
            </div>
          )}
          {config.mode === "stories" && (
            <div className="space-y-3">
              <div>
                <label className="text-sm font-medium text-foreground">{t("mailing.modeStories")}</label>
                <input value={config.storyLink} onChange={(e) => set("storyLink", e.target.value)} placeholder="https://t.me/username/s/123" className="mt-1.5 w-full max-w-md rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50" />
              </div>
              <div className="flex items-center gap-2">
                <button onClick={() => selectFile("usernamesPath")} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition">
                  <FolderOpen className="h-4 w-4" /> {t("mailing.modeUsernames")}
                </button>
                <span className="text-xs text-muted-foreground truncate max-w-sm">{config.usernamesPath ? config.usernamesPath.split(/[/\\]/).pop() : t("common.notSelected")}</span>
              </div>
            </div>
          )}
          {config.mode === "phones" && (
            <div className="flex items-center gap-2">
              <button onClick={() => selectFile("phonesPath")} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition">
                <FolderOpen className="h-4 w-4" /> {t("mailing.modePhones")}
              </button>
              <span className="text-xs text-muted-foreground truncate max-w-sm">{config.phonesPath ? config.phonesPath.split(/[/\\]/).pop() : t("common.notSelected")}</span>
            </div>
          )}

          {/* message type */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("mailing.messageType")}</label>
            <div className="flex flex-wrap items-center gap-2 mt-1.5">
              <ModeButton active={config.messageType === "text"} onClick={() => set("messageType", "text")}>{t("mailing.typeText")}</ModeButton>
              <ModeButton active={config.messageType === "postbot"} onClick={() => set("messageType", "postbot")}>{t("mailing.typePostbot")}</ModeButton>
              <ModeButton active={config.messageType === "forward"} onClick={() => set("messageType", "forward")}>{t("mailing.typeForward")}</ModeButton>
              <ModeButton active={config.messageType === "voice"} onClick={() => set("messageType", "voice")}>{t("mailing.typeVoice")}</ModeButton>
            </div>
          </div>

          {/* message content */}
          {config.messageType === "text" && (
            <div className="space-y-3">
              <button
                onClick={() => setMessageEditorOpen(true)}
                className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-left text-sm hover:border-primary/50 transition w-full max-w-md"
              >
                <FileText className="h-4 w-4 text-muted-foreground shrink-0" />
                {config.messageText ? (
                  <span className="truncate text-foreground flex-1">
                    {config.messageText.replace(/[*_~|`+\[\]()]/g, "").split("\n")[0].slice(0, 60) || t("messageEditor.empty")}
                  </span>
                ) : (
                  <span className="text-muted-foreground flex-1">{t("mailing.textModify")}</span>
                )}
                {config.messageImagePath && <ImageIcon className="h-3.5 w-3.5 text-primary" />}
              </button>
              <div className="flex items-center gap-2">
                <ModeButton active={config.textModify === "none"} onClick={() => set("textModify", "none")}>{t("mailing.modifyNone")}</ModeButton>
                <ModeButton active={config.textModify === "llm_rewrite"} onClick={() => set("textModify", "llm_rewrite")}>{t("mailing.modifyLlm")}</ModeButton>
                <ModeButton active={config.textModify === "randomize"} onClick={() => set("textModify", "randomize")}>{t("mailing.modifyRandomize")}</ModeButton>
              </div>
            </div>
          )}
          {config.messageType === "postbot" && (
            <div>
              <label className="text-sm font-medium text-foreground">{t("mailing.postbotLabel")}</label>
              <input value={config.postbotHash} onChange={(e) => set("postbotHash", e.target.value)} placeholder="13jwcuo1mpshi7a2" className="mt-1.5 w-full max-w-md rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50" />
              <p className="text-xs text-muted-foreground mt-1">{t("mailing.postbotHint")}</p>
            </div>
          )}
          {config.messageType === "forward" && (
            <div>
              <label className="text-sm font-medium text-foreground">{t("mailing.forwardLabel")}</label>
              <input value={config.forwardMsgId} onChange={(e) => set("forwardMsgId", e.target.value)} placeholder="12345" className="mt-1.5 w-full max-w-xs rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50" />
              <p className="text-xs text-muted-foreground mt-1">{t("mailing.forwardHint")}</p>
            </div>
          )}
          {config.messageType === "voice" && (
            <div className="flex items-center gap-2">
              <button onClick={() => selectFile("voicePath")} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition">
                <FolderOpen className="h-4 w-4" /> {t("common.selectFile")}
              </button>
              <span className="text-xs text-muted-foreground truncate max-w-sm">{config.voicePath ? config.voicePath.split(/[/\\]/).pop() : t("common.notSelected")}</span>
            </div>
          )}

          {/* scheduled + silent */}
          <div className="border-t border-border pt-4 space-y-3">
            <label className="flex items-center gap-3 cursor-pointer">
              <input type="checkbox" checked={config.silent} onChange={(e) => set("silent", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
              <span className="text-sm font-medium text-foreground">{t("mailing.silent")}</span>
            </label>
            <label className="flex items-center gap-3 cursor-pointer">
              <input type="checkbox" checked={config.deleteDialog} onChange={(e) => set("deleteDialog", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
              <span className="text-sm font-medium text-foreground">{t("mailing.deleteDialog")}</span>
            </label>
            <label className="flex items-center gap-3 cursor-pointer">
              <input type="checkbox" checked={config.pinMessage} onChange={(e) => set("pinMessage", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
              <span className="text-sm font-medium text-foreground">{t("mailing.pinMessage")}</span>
            </label>
            <label className="flex items-center gap-3 cursor-pointer">
              <input type="checkbox" checked={config.videoNote} onChange={(e) => set("videoNote", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
              <span className="text-sm font-medium text-foreground">{t("mailing.videoNote")}</span>
            </label>
            <div>
              <label className="text-sm font-medium text-foreground">{t("mailing.mediaTtlLabel")}</label>
              <div className="flex items-center gap-2 mt-1.5">
                <input type="number" min={0} max={604800} value={config.fileTtl} onChange={(e) => set("fileTtl", Math.max(0, Number(e.target.value)))} className="w-24 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
                <span className="text-xs text-muted-foreground">{t("mailing.mediaTtlHint")}</span>
              </div>
            </div>
            <label className="flex items-center gap-3 cursor-pointer">
              <input type="checkbox" checked={config.autostopEnabled} onChange={(e) => set("autostopEnabled", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
              <span className="text-sm font-medium text-foreground">{t("mailing.autostop")}</span>
            </label>
            {config.autostopEnabled && (
              <div className="ml-7 space-y-2">
                <div className="flex items-center gap-2">
                  <span className="text-xs text-muted-foreground w-32">{t("mailing.autostopBan")}:</span>
                  <input type="number" min={0} max={999} value={config.autostopBan} onChange={(e) => set("autostopBan", Math.max(0, Number(e.target.value)))} className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
                </div>
                <div className="flex items-center gap-2">
                  <span className="text-xs text-muted-foreground w-32">{t("mailing.autostopSpamblock")}:</span>
                  <input type="number" min={0} max={999} value={config.autostopSpamblock} onChange={(e) => set("autostopSpamblock", Math.max(0, Number(e.target.value)))} className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
                </div>
                <div className="flex items-center gap-2">
                  <span className="text-xs text-muted-foreground w-32">{t("mailing.autostopFlood")}:</span>
                  <input type="number" min={0} max={999} value={config.autostopFlood} onChange={(e) => set("autostopFlood", Math.max(0, Number(e.target.value)))} className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
                </div>
              </div>
            )}
            <label className="flex items-center gap-3 cursor-pointer">
              <input type="checkbox" checked={config.autoRepost} onChange={(e) => set("autoRepost", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
              <span className="text-sm font-medium text-foreground">{t("mailing.autoRepost")}</span>
            </label>
            {config.autoRepost && (
              <p className="text-xs text-muted-foreground ml-7">{t("mailing.autoRepostHint")}</p>
            )}
            <div>
              <label className="text-sm font-medium text-foreground">{t("mailing.scheduledLabel")}</label>
              <div className="flex items-center gap-2 mt-1.5">
                <input
                  type="text"
                  value={(config.scheduledTime || "").split(" ")[0] || ""}
                  onChange={(e) => {
                    const date = e.target.value;
                    const time = (config.scheduledTime || "").split(" ")[1] || "";
                    set("scheduledTime", [date, time].filter(Boolean).join(" "));
                  }}
                  placeholder="DD.MM.YYYY"
                  maxLength={10}
                  className="w-32 rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50"
                />
                <input
                  type="text"
                  value={(config.scheduledTime || "").split(" ")[1] || ""}
                  onChange={(e) => {
                    const time = e.target.value;
                    const date = (config.scheduledTime || "").split(" ")[0] || "";
                    set("scheduledTime", [date, time].filter(Boolean).join(" "));
                  }}
                  placeholder="HH:MM"
                  maxLength={5}
                  className="w-20 rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50"
                />
              </div>
              <p className="text-xs text-muted-foreground mt-1">{t("mailing.scheduledHint")}</p>
            </div>
          </div>

          {/* limits */}
          <div className="border-t border-border pt-4 space-y-3">
            <div>
              <label className="text-sm font-medium text-foreground">{t("mailing.maxPerAccount")}</label>
              <input type="number" min={1} max={99999} value={config.maxPerAccount} onChange={(e) => set("maxPerAccount", Math.max(1, Number(e.target.value)))} className="mt-1.5 w-28 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
              <p className="text-xs text-muted-foreground mt-1">{t("mailing.randomAmount")}</p>
              <div className="flex items-center gap-2 mt-1">
                <input type="number" min={0} max={99999} value={config.kolMin} onChange={(e) => set("kolMin", Math.max(0, Number(e.target.value)))} placeholder="min" className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
                <span className="text-muted-foreground text-sm">—</span>
                <input type="number" min={0} max={99999} value={config.kolMax} onChange={(e) => set("kolMax", Math.max(0, Number(e.target.value)))} placeholder="max" className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
                <span className="text-xs text-muted-foreground">{t("mailing.randomAmountHint")}</span>
              </div>
            </div>
            <div>
              <label className="text-sm font-medium text-foreground">{t("common.maxFloodWait")}</label>
              <div className="flex items-center gap-2 mt-1.5">
                <input type="number" min={0} max={86400} value={config.maxFloodWait} onChange={(e) => set("maxFloodWait", Math.max(0, Math.min(86400, Number(e.target.value))))} className="w-28 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
              </div>
            </div>
          </div>

          {/* output */}
          <div className="border-t border-border pt-4">
            <label className="block text-sm font-medium text-foreground mb-1">{t("common.selectFile")}</label>
            <div className="flex items-center gap-2">
              <button onClick={selectOutputFile} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition">
                <FolderOpen className="h-4 w-4" /> {t("common.selectFile")}
              </button>
              <span className="text-xs text-muted-foreground truncate max-w-sm">{config.outputPath ? config.outputPath.split(/[/\\]/).pop() : t("mailing.outputAuto")}</span>
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

      <div className="flex items-center gap-3">
        <div className="flex items-center gap-6 text-sm font-semibold">
          <span>{t("common.sent")}: <span className="text-[oklch(0.65_0.1_150)]">{stats.sent}</span></span>
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

      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div ref={logsRef} className="h-40 overflow-y-auto scrollbar-thin p-4 font-mono text-xs space-y-0.5">
          {logs.length === 0 && <div className="text-muted-foreground text-center py-8">{t("common.logsPlaceholder")}</div>}
          {logs.map((line, i) => <div key={i} className="text-muted-foreground whitespace-pre-wrap break-all">{line}</div>)}
        </div>
      </div>

      <AccountPickerModal open={pickerOpen} onClose={() => setPickerOpen(false)} onSelect={handleAccountsSelected} />
      <MessageEditorModal
        open={messageEditorOpen}
        initialValue={config.messageText}
        initialImagePath={config.messageImagePath}
        initialVideoPath={config.messageVideoPath}
        title={t("mailing.messageType")}
        withImage
        withVideo
        onClose={() => setMessageEditorOpen(false)}
        onSave={(text, imagePath, videoPath) => {
          set("messageText", text);
          set("messageImagePath", imagePath || "");
          set("messageVideoPath", videoPath || "");
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
