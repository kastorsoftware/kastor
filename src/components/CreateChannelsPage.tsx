import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Play, Square, FolderOpen } from "lucide-react";
import { AccountPickerModal } from "@/components/AccountPickerModal";
import { useT } from "@/i18n";
import { isDone, isError, isChannelCreated } from "@/lib/eventParser";

type ChannelType = "channel_public" | "channel_private" | "group_public" | "group_private";
type TitleMode = "single" | "from_file";
type TextMode = "single" | "from_file";
type PhotoMode = "single" | "from_folder";
type UsernameMode = "random" | "from_file";
type PostMode = "text" | "image" | "forward";

interface Config {
  channelType: ChannelType;
  channelsMin: number;
  channelsMax: number;
  outputPath: string;

  titleMode: TitleMode;
  titleSingle: string;
  titleFilePath: string;

  setDescription: boolean;
  descriptionMode: TextMode;
  descriptionSingle: string;
  descriptionFilePath: string;

  setPhoto: boolean;
  photoMode: PhotoMode;
  photoSinglePath: string;
  photoFolderPath: string;

  setUsername: boolean;
  usernameMode: UsernameMode;
  usernameFilePath: string;

  setProfileChannel: boolean;

  addAdmins: boolean;
  adminIds: string;

  postEnabled: boolean;
  postMode: PostMode;
  postText: string;
  postImagePath: string;
  postVideoPath: string;
  postForwardLink: string;
  postRandomize: boolean;
  postLlmRewrite: boolean;

  delayMin: number;
  delayMax: number;
  maxFloodWait: number;
}

const defaultConfig: Config = {
  channelType: "channel_private",
  channelsMin: 1,
  channelsMax: 1,
  outputPath: "",

  titleMode: "single",
  titleSingle: "",
  titleFilePath: "",

  setDescription: false,
  descriptionMode: "single",
  descriptionSingle: "",
  descriptionFilePath: "",

  setPhoto: false,
  photoMode: "from_folder",
  photoSinglePath: "",
  photoFolderPath: "",

  setUsername: false,
  usernameMode: "random",
  usernameFilePath: "",

  setProfileChannel: false,

  addAdmins: false,
  adminIds: "",

  postEnabled: false,
  postMode: "text",
  postText: "",
  postImagePath: "",
  postVideoPath: "",
  postForwardLink: "",
  postRandomize: false,
  postLlmRewrite: false,

  delayMin: 2000,
  delayMax: 5000,
  maxFloodWait: 60,
};

const STORAGE_KEY = "create_channels_config";
const IS_DEV = !("__TAURI_INTERNALS__" in window);

function loadSavedConfig(): Config {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return { ...defaultConfig, ...JSON.parse(saved) };
  } catch {}
  return defaultConfig;
}

export function CreateChannelsPage() {
  const t = useT();
  const [config, setConfig] = useState<Config>(loadSavedConfig);
  const [running, setRunning] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [stats, setStats] = useState({ done: 0, errors: 0 });
  const [pickerOpen, setPickerOpen] = useState(false);
  const logsRef = useRef<HTMLDivElement>(null);

  useEffect(() => { localStorage.setItem(STORAGE_KEY, JSON.stringify(config)); }, [config]);

  useEffect(() => {
    if (IS_DEV) return;
    const unlisten = listen<string>("create-channels-log", (e) => {
      setLogs((prev) => [...prev, e.payload]);
      const msg = e.payload;
      if (isDone(msg)) setRunning(false);
      else if (isError(msg)) setStats((s) => ({ ...s, errors: s.errors + 1 }));
      else if (isChannelCreated(msg)) setStats((s) => ({ ...s, done: s.done + 1 }));
    });
    return () => { unlisten.then((f) => f()); };
  }, []);

  useEffect(() => {
    const el = logsRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    if (atBottom) el.scrollTop = el.scrollHeight;
  }, [logs]);

  const set = <K extends keyof Config>(key: K, value: Config[K]) => setConfig((prev) => ({ ...prev, [key]: value }));

  const selectFile = async (key: keyof Config) => {
    if (IS_DEV) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open({ multiple: false, filters: [{ name: "Text", extensions: ["txt"] }] });
    if (path) set(key, path as any);
  };
  const selectPhoto = async (key: keyof Config) => {
    if (IS_DEV) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open({ multiple: false, filters: [{ name: "Images", extensions: ["jpg", "jpeg", "png", "webp"] }] });
    if (path) set(key, path as any);
  };
  const selectFolder = async (key: keyof Config) => {
    if (IS_DEV) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open({ directory: true });
    if (path) set(key, path as any);
  };
  const selectOutputFile = async () => {
    if (IS_DEV) return;
    const { save } = await import("@tauri-apps/plugin-dialog");
    const path = await save({ defaultPath: "channels.db", filters: [{ name: "SQLite DB", extensions: ["db"] }] });
    if (path) set("outputPath", path);
  };

  const handleStart = () => {
    if (config.titleMode === "single" && !config.titleSingle.trim()) { setLogs((p) => [...p, `${t("common.error")}: ${t("createChannels.titleLabel")}`]); return; }
    if (config.titleMode === "from_file" && !config.titleFilePath) { setLogs((p) => [...p, `${t("common.error")}: ${t("createChannels.titleFromFile")}`]); return; }
    setPickerOpen(true);
  };

  const handleAccountsSelected = async (ids: string[]) => {
    if (ids.length === 0) return;
    setLogs([]); setStats({ done: 0, errors: 0 }); setRunning(true);
    try {
      const tid = await invoke<string>("create_channels_start", {
        ids,
        config: {
          channel_type: config.channelType,
          channels_min: config.channelsMin,
          channels_max: config.channelsMax,
          output_path: config.outputPath,
          title_mode: config.titleMode,
          title_single: config.titleSingle,
          title_file_path: config.titleFilePath,
          set_description: config.setDescription,
          description_mode: config.descriptionMode,
          description_single: config.descriptionSingle,
          description_file_path: config.descriptionFilePath,
          set_photo: config.setPhoto,
          photo_mode: config.photoMode,
          photo_single_path: config.photoSinglePath,
          photo_folder_path: config.photoFolderPath,
          set_username: config.channelType.includes("public") ? true : config.setUsername,
          username_mode: config.usernameMode,
          username_file_path: config.usernameFilePath,
          set_profile_channel: config.setProfileChannel,
          add_admins: config.addAdmins,
          admin_ids: config.adminIds,
          post_enabled: config.postEnabled,
          post_mode: config.postMode,
          post_text: config.postText,
          post_image_path: config.postImagePath,
          post_video_path: config.postVideoPath,
          post_forward_link: config.postForwardLink,
          post_randomize: config.postRandomize,
          post_llm_rewrite: config.postLlmRewrite,
          delay_min: config.delayMin,
          delay_max: config.delayMax,
        },
        maxFloodWait: config.maxFloodWait,
      });
      setTaskId(tid);
    } catch (e: any) { setLogs((p) => [...p, `${t("common.error")}: ${e}`]); setRunning(false); }
  };

  const handleStop = async () => {
    if (!IS_DEV && taskId) await invoke("create_channels_stop", { taskId }).catch(() => {});
    setRunning(false); setLogs((p) => [...p, t("common.stoppedByUser")]);
  };

  const isPublic = config.channelType.includes("public");

  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">{t("descriptions.createChannels")}</p>
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div className="p-6 space-y-5">

          {/* type */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("createChannels.title")}</label>
            <div className="flex flex-wrap gap-2 mt-1.5">
              <ModeButton active={config.channelType === "channel_private"} onClick={() => set("channelType", "channel_private")}>{t("createChannels.typeChannelPrivate")}</ModeButton>
              <ModeButton active={config.channelType === "channel_public"} onClick={() => set("channelType", "channel_public")}>{t("createChannels.typeChannelPublic")}</ModeButton>
              <ModeButton active={config.channelType === "group_private"} onClick={() => set("channelType", "group_private")}>{t("createChannels.typeGroupPrivate")}</ModeButton>
              <ModeButton active={config.channelType === "group_public"} onClick={() => set("channelType", "group_public")}>{t("createChannels.typeGroupPublic")}</ModeButton>
            </div>
          </div>

          {/* count */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("createChannels.countPerAccount")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <input type="number" min={1} max={50} value={config.channelsMin} onChange={(e) => set("channelsMin", Math.max(1, Math.min(50, Number(e.target.value))))} className="w-16 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm text-center outline-none focus:border-primary/50" />
              <span className="text-sm text-muted-foreground">—</span>
              <input type="number" min={1} max={50} value={config.channelsMax} onChange={(e) => set("channelsMax", Math.max(1, Math.min(50, Number(e.target.value))))} className="w-16 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm text-center outline-none focus:border-primary/50" />
            </div>
            <p className="text-xs text-muted-foreground mt-1">{t("createChannels.countPerAccount")}</p>
          </div>

          {/* delay */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("common.delay")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <input type="number" min={0} value={config.delayMin} onChange={(e) => set("delayMin", Math.max(0, Number(e.target.value)))} className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm text-center" />
              <span className="text-sm text-muted-foreground">—</span>
              <input type="number" min={0} value={config.delayMax} onChange={(e) => set("delayMax", Math.max(0, Number(e.target.value)))} className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm text-center" />
            </div>
          </div>

          {/* max flood wait */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("common.maxFloodWait")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <input type="number" min={0} value={config.maxFloodWait} onChange={(e) => set("maxFloodWait", Math.max(0, Number(e.target.value)))} className="w-24 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm text-center" />
              <span className="text-xs text-muted-foreground">0 = ∞</span>
            </div>
          </div>

          {/* title */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("createChannels.titleLabel")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <ModeButton active={config.titleMode === "single"} onClick={() => set("titleMode", "single")}>{t("createChannels.titleSingle")}</ModeButton>
              <ModeButton active={config.titleMode === "from_file"} onClick={() => set("titleMode", "from_file")}>{t("createChannels.titleFromFile")}</ModeButton>
            </div>
            {config.titleMode === "single" && (
              <div><input value={config.titleSingle} onChange={(e) => set("titleSingle", e.target.value)} placeholder="{My|Super} {channel|chat}" className="mt-2 w-full max-w-md rounded-md border border-border bg-background px-3 py-1.5 text-sm outline-none focus:border-primary/50" />
              <p className="text-xs text-muted-foreground mt-1">{t("createChannels.spintaxHint")}</p></div>
            )}
            {config.titleMode === "from_file" && <FilePickerRow path={config.titleFilePath} onPick={() => selectFile("titleFilePath")} />}
          </div>

          {/* username (only for public types) — placed right after title */}
          {isPublic && (
            <div>
              <label className="text-sm font-medium text-foreground">{t("createChannels.username")} <span className="text-destructive">*</span></label>
              <div className="flex items-center gap-2 mt-1.5">
                <ModeButton active={config.usernameMode === "random"} onClick={() => set("usernameMode", "random")}>{t("createChannels.usernameRandom")}</ModeButton>
                <ModeButton active={config.usernameMode === "from_file"} onClick={() => set("usernameMode", "from_file")}>{t("createChannels.usernameFromFile")}</ModeButton>
              </div>
              {config.usernameMode === "from_file" && <FilePickerRow path={config.usernameFilePath} onPick={() => selectFile("usernameFilePath")} />}
            </div>
          )}

          {/* description */}
          <ToggleSection label={t("createChannels.description")} checked={config.setDescription} onChange={(v) => set("setDescription", v)}>
            <div className="flex items-center gap-2">
              <ModeButton active={config.descriptionMode === "single"} onClick={() => set("descriptionMode", "single")}>{t("createChannels.titleSingle")}</ModeButton>
              <ModeButton active={config.descriptionMode === "from_file"} onClick={() => set("descriptionMode", "from_file")}>{t("createChannels.titleFromFile")}</ModeButton>
            </div>
            {config.descriptionMode === "single" && <input value={config.descriptionSingle} onChange={(e) => set("descriptionSingle", e.target.value)} placeholder="..." className="mt-2 w-full max-w-md rounded-md border border-border bg-background px-3 py-1.5 text-sm outline-none focus:border-primary/50" />}
            {config.descriptionMode === "from_file" && <FilePickerRow path={config.descriptionFilePath} onPick={() => selectFile("descriptionFilePath")} />}
          </ToggleSection>

          {/* photo */}
          <ToggleSection label={t("createChannels.photo")} checked={config.setPhoto} onChange={(v) => set("setPhoto", v)}>
            <div className="flex items-center gap-2">
              <ModeButton active={config.photoMode === "single"} onClick={() => set("photoMode", "single")}>{t("createChannels.titleSingle")}</ModeButton>
              <ModeButton active={config.photoMode === "from_folder"} onClick={() => set("photoMode", "from_folder")}>{t("createChannels.titleFromFile")}</ModeButton>
            </div>
            {config.photoMode === "single" && <FilePickerRow path={config.photoSinglePath} onPick={() => selectPhoto("photoSinglePath")} />}
            {config.photoMode === "from_folder" && <FilePickerRow path={config.photoFolderPath} onPick={() => selectFolder("photoFolderPath")} />}
          </ToggleSection>

          {/* profile channel */}
          <CheckRow label={t("createChannels.setProfileChannel")} checked={config.setProfileChannel} onChange={(v) => set("setProfileChannel", v)} />

          {/* admins */}
          <ToggleSection label={t("createChannels.addAdmins")} checked={config.addAdmins} onChange={(v) => set("addAdmins", v)}>
            <input value={config.adminIds} onChange={(e) => set("adminIds", e.target.value)} placeholder="username1,username2" className="mt-1 w-full max-w-md rounded-md border border-border bg-background px-3 py-1.5 text-sm outline-none focus:border-primary/50" />
            <p className="text-xs text-muted-foreground mt-1">{t("createChannels.addAdminsHint")}</p>
          </ToggleSection>

          {/* post */}
          <ToggleSection label={t("createChannels.postAfter")} checked={config.postEnabled} onChange={(v) => set("postEnabled", v)}>
            <div className="flex items-center gap-2 mb-2">
              <ModeButton active={config.postMode === "text"} onClick={() => set("postMode", "text")}>{t("createChannels.postText")}</ModeButton>
              <ModeButton active={config.postMode === "image"} onClick={() => set("postMode", "image")}>{t("createChannels.postImage")}</ModeButton>
              <ModeButton active={config.postMode === "forward"} onClick={() => set("postMode", "forward")}>{t("createChannels.postForward")}</ModeButton>
            </div>
            {config.postMode === "text" && (
              <div>
                <textarea value={config.postText} onChange={(e) => set("postText", e.target.value)} placeholder="markdown, spintax..." rows={3} className="w-full max-w-md rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50 resize-y" />
                <div className="flex items-center gap-4 mt-2">
                  <CheckRow label={t("createChannels.randomizePost")} checked={config.postRandomize} onChange={(v) => set("postRandomize", v)} />
                  <CheckRow label={t("createChannels.llmRewrite")} checked={config.postLlmRewrite} onChange={(v) => set("postLlmRewrite", v)} />
                </div>
              </div>
            )}
            {config.postMode === "image" && (
              <div>
                <textarea value={config.postText} onChange={(e) => set("postText", e.target.value)} placeholder="..." rows={2} className="w-full max-w-md rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50 resize-y" />
                <FilePickerRow path={config.postImagePath} onPick={() => selectPhoto("postImagePath")} />
              </div>
            )}
            {config.postMode === "forward" && (
              <div>
                <input value={config.postForwardLink} onChange={(e) => set("postForwardLink", e.target.value)} placeholder="https://t.me/channel/123" className="w-full max-w-md rounded-md border border-border bg-background px-3 py-1.5 text-sm outline-none focus:border-primary/50" />
                <p className="text-xs text-muted-foreground mt-1">{t("createChannels.postForwardHint")}</p>
              </div>
            )}
          </ToggleSection>

          {/* output */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("createBots.outputPath")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <button onClick={selectOutputFile} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition"><FolderOpen className="h-4 w-4" /> {t("common.selectFile")}</button>
              <span className="text-xs text-muted-foreground truncate max-w-sm">{config.outputPath ? config.outputPath.split(/[/\\]/).pop() : "channels.db"}</span>
            </div>
          </div>

        </div>
      </div>

      {/* controls */}
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-6 text-sm font-semibold">
          <span>{t("common.done")}: <span className="text-[oklch(0.65_0.1_150)]">{stats.done}</span></span>
          <span>{t("common.errors")}: <span className="text-[oklch(0.55_0.1_25)]">{stats.errors}</span></span>
        </div>
        <div className="ml-auto flex items-center gap-3">
          <span className="max-w-[220px] text-right text-xs text-muted-foreground">{t("common.startSelectAccountsHint")}</span>
          <button onClick={handleStart} disabled={running} className="flex items-center gap-2 rounded-md border border-primary/50 bg-primary/10 px-4 py-2 text-sm text-primary font-medium hover:bg-primary/20 transition disabled:opacity-50"><Play className="h-4 w-4" /> {t("common.start")}</button>
          <button onClick={handleStop} disabled={!running} className="flex items-center gap-2 rounded-md border px-4 py-2 text-sm font-medium transition disabled:opacity-50" style={{ borderColor: "color-mix(in oklch, oklch(0.55 0.1 25) 50%, transparent)", color: "oklch(0.55 0.1 25)", background: "color-mix(in oklch, oklch(0.55 0.1 25) 6%, transparent)" }}><Square className="h-4 w-4" /> {t("common.stop")}</button>
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
  return <button onClick={onClick} className={`rounded-md border px-3 py-1 text-xs font-medium transition ${active ? "border-primary/50 bg-primary/10 text-primary" : "border-border bg-background text-muted-foreground hover:border-primary/30"}`}>{children}</button>;
}

function CheckRow({ label, checked, onChange }: { label: string; checked: boolean; onChange: (v: boolean) => void }) {
  return <label className="flex items-center gap-2.5 cursor-pointer text-sm"><input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} className="rounded border-border accent-primary h-4 w-4" /><span className="text-foreground font-medium">{label}</span></label>;
}

function ToggleSection({ label, checked, onChange, children }: { label: string; checked: boolean; onChange: (v: boolean) => void; children: React.ReactNode }) {
  return (
    <div>
      <CheckRow label={label} checked={checked} onChange={onChange} />
      {checked && <div className="ml-7 mt-2 space-y-2">{children}</div>}
    </div>
  );
}

function FilePickerRow({ path, onPick, label: _label }: { path: string; onPick: () => void; label?: string }) {
  const t = useT();
  return (
    <div className="flex items-center gap-2 mt-2">
      <button onClick={onPick} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition"><FolderOpen className="h-4 w-4" /> {t("common.selectFile")}</button>
      <span className="text-xs text-muted-foreground truncate max-w-sm">{path ? path.split(/[/\\]/).pop() : t("common.notSelected")}</span>
    </div>
  );
}
