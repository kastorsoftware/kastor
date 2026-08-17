import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Play, Square, FolderOpen } from "lucide-react";
import { AccountPickerModal } from "@/components/AccountPickerModal";
import { useT } from "@/i18n";
import { isDone, isError, isBotCreated } from "@/lib/eventParser";

type NameMode = "random" | "from_file" | "single";
type UsernameMode = "random" | "from_file";
type TextMode = "single" | "from_file";
type PhotoMode = "single" | "from_folder";

interface CreateBotsConfig {
  nameMode: NameMode;
  nameSingle: string;
  nameFilePath: string;

  usernameMode: UsernameMode;
  usernameFilePath: string;

  botsMin: number;
  botsMax: number;

  outputPath: string;

  setDescription: boolean;
  descriptionMode: TextMode;
  descriptionSingle: string;
  descriptionFilePath: string;

  setAbout: boolean;
  aboutMode: TextMode;
  aboutSingle: string;
  aboutFilePath: string;

  setPhoto: boolean;
  photoMode: PhotoMode;
  photoSinglePath: string;
  photoFolderPath: string;

  setPrivacy: boolean;

  delayMin: number;
  delayMax: number;
  maxFloodWait: number;
}

const defaultConfig: CreateBotsConfig = {
  nameMode: "random",
  nameSingle: "",
  nameFilePath: "",

  usernameMode: "random",
  usernameFilePath: "",

  botsMin: 1,
  botsMax: 1,

  outputPath: "",

  setDescription: false,
  descriptionMode: "single",
  descriptionSingle: "",
  descriptionFilePath: "",

  setAbout: false,
  aboutMode: "single",
  aboutSingle: "",
  aboutFilePath: "",

  setPhoto: false,
  photoMode: "from_folder",
  photoSinglePath: "",
  photoFolderPath: "",

  setPrivacy: false,

  delayMin: 1000,
  delayMax: 3000,
  maxFloodWait: 60,
};

const STORAGE_KEY = "create_bots_config";
const IS_DEV = !("__TAURI_INTERNALS__" in window);

function loadSavedConfig(): CreateBotsConfig {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return { ...defaultConfig, ...JSON.parse(saved) };
  } catch {}
  return defaultConfig;
}

export function CreateBotsPage() {
  const t = useT();
  const [config, setConfig] = useState<CreateBotsConfig>(loadSavedConfig);
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
    const unlisten = listen<string>("create-bots-log", (e) => {
      setLogs((prev) => [...prev, e.payload]);
      const msg = e.payload;
      if (isDone(msg)) {
        setRunning(false);
      } else if (isError(msg)) {
        setStats((s) => ({ ...s, errors: s.errors + 1 }));
      } else if (isBotCreated(msg)) {
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

  const set = <K extends keyof CreateBotsConfig>(key: K, value: CreateBotsConfig[K]) => {
    setConfig((prev) => ({ ...prev, [key]: value }));
  };

  const handleStart = () => {
    if (config.nameMode === "single" && !config.nameSingle.trim()) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${t("createBots.nameLabel")}`]);
      return;
    }
    if (config.nameMode === "from_file" && !config.nameFilePath) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${t("createBots.nameFromFile")}`]);
      return;
    }
    if (config.usernameMode === "from_file" && !config.usernameFilePath) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${t("createBots.usernameFromFile")}`]);
      return;
    }
    if (config.setDescription && config.descriptionMode === "single" && !config.descriptionSingle.trim()) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${t("createBots.description")}`]);
      return;
    }
    if (config.setDescription && config.descriptionMode === "from_file" && !config.descriptionFilePath) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${t("createBots.description")}`]);
      return;
    }
    if (config.setAbout && config.aboutMode === "single" && !config.aboutSingle.trim()) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${t("createBots.about")}`]);
      return;
    }
    if (config.setAbout && config.aboutMode === "from_file" && !config.aboutFilePath) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${t("createBots.about")}`]);
      return;
    }
    if (config.setPhoto && config.photoMode === "single" && !config.photoSinglePath) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${t("createBots.photo")}`]);
      return;
    }
    if (config.setPhoto && config.photoMode === "from_folder" && !config.photoFolderPath) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${t("createBots.photo")}`]);
      return;
    }
    setPickerOpen(true);
  };

  const handleAccountsSelected = async (ids: string[]) => {
    if (ids.length === 0) return;
    setLogs([]);
    setStats({ done: 0, errors: 0, inProgress: ids.length * config.botsMax });
    setRunning(true);
    try {
      const tid = await invoke<string>("create_bots_start", {
        ids,
        config: {
          name_mode: config.nameMode,
          name_single: config.nameSingle,
          name_file_path: config.nameFilePath,
          username_mode: config.usernameMode,
          username_file_path: config.usernameFilePath,
          bots_min: config.botsMin,
          bots_max: config.botsMax,
          output_path: config.outputPath,
          set_description: config.setDescription,
          description_mode: config.descriptionMode,
          description_single: config.descriptionSingle,
          description_file_path: config.descriptionFilePath,
          set_about: config.setAbout,
          about_mode: config.aboutMode,
          about_single: config.aboutSingle,
          about_file_path: config.aboutFilePath,
          set_photo: config.setPhoto,
          photo_mode: config.photoMode,
          photo_single_path: config.photoSinglePath,
          photo_folder_path: config.photoFolderPath,
          set_privacy: config.setPrivacy,
          delay_min: config.delayMin,
          delay_max: config.delayMax,
        },
        maxFloodWait: config.maxFloodWait,
      });
      setTaskId(tid);
    } catch (e: any) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${e}`]);
      setRunning(false);
    }
  };

  const handleStop = async () => {
    if (!IS_DEV && taskId) await invoke("create_bots_stop", { taskId }).catch(() => {});
    setRunning(false);
    setLogs((prev) => [...prev, t("common.stoppedByUser")]);
  };

  const selectFile = async (key: keyof CreateBotsConfig) => {
    if (IS_DEV) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open({ multiple: false, filters: [{ name: "Text", extensions: ["txt"] }] });
    if (path) set(key, path as any);
  };

  const selectPhoto = async () => {
    if (IS_DEV) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open({ multiple: false, filters: [{ name: "Images", extensions: ["jpg", "jpeg", "png", "webp"] }] });
    if (path) set("photoSinglePath", path as string);
  };

  const selectFolder = async (key: keyof CreateBotsConfig) => {
    if (IS_DEV) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open({ directory: true });
    if (path) set(key, path as any);
  };

  const selectOutputFile = async () => {
    if (IS_DEV) return;
    const { save } = await import("@tauri-apps/plugin-dialog");
    const path = await save({ defaultPath: "bots.db", filters: [{ name: "SQLite DB", extensions: ["db"] }] });
    if (path) set("outputPath", path);
  };

  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">{t("descriptions.createBots")}</p>
      {/* config panel */}
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div className="p-6 space-y-5">

          {/* name */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("createBots.nameLabel")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <ModeButton active={config.nameMode === "random"} onClick={() => set("nameMode", "random")}>{t("createBots.nameRandom")}</ModeButton>
              <ModeButton active={config.nameMode === "from_file"} onClick={() => set("nameMode", "from_file")}>{t("createBots.nameFromFile")}</ModeButton>
              <ModeButton active={config.nameMode === "single"} onClick={() => set("nameMode", "single")}>{t("createBots.nameSingle")}</ModeButton>
            </div>
            {config.nameMode === "single" && (
              <div>
                <input
                  value={config.nameSingle}
                  onChange={(e) => set("nameSingle", e.target.value)}
                  placeholder="{My|Super|Cool} {bot|helper}"
                  className="mt-2 w-full max-w-md rounded-md border border-border bg-background px-3 py-1.5 text-sm outline-none focus:border-primary/50"
                />
                <p className="text-xs text-muted-foreground mt-1">{t("createChannels.spintaxHint")}</p>
              </div>
            )}
            {config.nameMode === "from_file" && (
              <div className="flex items-center gap-2 mt-2">
                <button onClick={() => selectFile("nameFilePath")} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition"><FolderOpen className="h-4 w-4" /> {t("common.selectFile")}</button>
                <span className="text-xs text-muted-foreground truncate max-w-sm">{config.nameFilePath ? config.nameFilePath.split(/[/\\]/).pop() : t("common.notSelected")}</span>
              </div>
            )}
          </div>

          {/* username */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("createBots.usernameLabel")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <ModeButton active={config.usernameMode === "random"} onClick={() => set("usernameMode", "random")}>{t("createBots.usernameRandom")}</ModeButton>
              <ModeButton active={config.usernameMode === "from_file"} onClick={() => set("usernameMode", "from_file")}>{t("createBots.usernameFromFile")}</ModeButton>
            </div>
            {config.usernameMode === "from_file" && (
              <div className="mt-2">
                <div className="flex items-center gap-2 mt-2">
                  <button onClick={() => selectFile("usernameFilePath")} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition"><FolderOpen className="h-4 w-4" /> {t("common.selectFile")}</button>
                  <span className="text-xs text-muted-foreground truncate max-w-sm">{config.usernameFilePath ? config.usernameFilePath.split(/[/\\]/).pop() : t("common.notSelected")}</span>
                </div>
              </div>
            )}
          </div>

          {/* bots per account */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("createBots.botsPerAccount")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <input
                type="number" min={1} max={20} value={config.botsMin}
                onChange={(e) => set("botsMin", Math.max(1, Math.min(20, Number(e.target.value))))}
                className="w-16 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm text-center outline-none focus:border-primary/50"
              />
              <span className="text-sm text-muted-foreground">—</span>
              <input
                type="number" min={1} max={20} value={config.botsMax}
                onChange={(e) => set("botsMax", Math.max(1, Math.min(20, Number(e.target.value))))}
                className="w-16 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm text-center outline-none focus:border-primary/50"
              />
            </div>
            <p className="text-xs text-muted-foreground mt-1">{t("createBots.botsPerAccount")}</p>
          </div>

          {/* delay */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("common.delay")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <input type="number" min={0} value={config.delayMin}
                onChange={(e) => set("delayMin", Math.max(0, Number(e.target.value)))}
                className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm text-center outline-none focus:border-primary/50" />
              <span className="text-sm text-muted-foreground">—</span>
              <input type="number" min={0} value={config.delayMax}
                onChange={(e) => set("delayMax", Math.max(0, Number(e.target.value)))}
                className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm text-center outline-none focus:border-primary/50" />
            </div>
          </div>

          {/* max flood wait */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("common.maxFloodWait")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <input type="number" min={0} value={config.maxFloodWait}
                onChange={(e) => set("maxFloodWait", Math.max(0, Number(e.target.value)))}
                className="w-24 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm text-center outline-none focus:border-primary/50" />
              <span className="text-xs text-muted-foreground">0 = ∞</span>
            </div>
          </div>

          {/* output */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("createBots.outputPath")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <button onClick={selectOutputFile} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition"><FolderOpen className="h-4 w-4" /> {t("common.selectFile")}</button>
              <span className="text-xs text-muted-foreground truncate max-w-sm">{config.outputPath ? config.outputPath.split(/[/\\]/).pop() : "bots.db"}</span>
            </div>
          </div>

          {/* description */}
          <div>
            <label className="flex items-center gap-3 cursor-pointer">
              <input type="checkbox" checked={config.setDescription} onChange={(e) => set("setDescription", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
              <span className="text-sm font-medium text-foreground">{t("createBots.description")}</span>
            </label>
            {config.setDescription && (
              <div className="ml-7 mt-2">
                <div className="flex items-center gap-2">
                  <ModeButton active={config.descriptionMode === "single"} onClick={() => set("descriptionMode", "single")}>{t("createBots.nameSingle")}</ModeButton>
                  <ModeButton active={config.descriptionMode === "from_file"} onClick={() => set("descriptionMode", "from_file")}>{t("createBots.nameFromFile")}</ModeButton>
                </div>
                {config.descriptionMode === "single" && (
                  <input
                    value={config.descriptionSingle}
                    onChange={(e) => set("descriptionSingle", e.target.value)}
                    placeholder={t("createBots.description")}
                    className="mt-2 w-full max-w-md rounded-md border border-border bg-background px-3 py-1.5 text-sm outline-none focus:border-primary/50"
                  />
                )}
                {config.descriptionMode === "from_file" && (
                  <div className="flex items-center gap-2 mt-2">
                    <button onClick={() => selectFile("descriptionFilePath")} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition"><FolderOpen className="h-4 w-4" /> {t("common.selectFile")}</button>
                    <span className="text-xs text-muted-foreground truncate max-w-sm">{config.descriptionFilePath ? config.descriptionFilePath.split(/[/\\]/).pop() : t("common.notSelected")}</span>
                  </div>
                )}
              </div>
            )}
          </div>

          {/* about */}
          <div>
            <label className="flex items-center gap-3 cursor-pointer">
              <input type="checkbox" checked={config.setAbout} onChange={(e) => set("setAbout", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
              <span className="text-sm font-medium text-foreground">{t("createBots.about")}</span>
            </label>
            {config.setAbout && (
              <div className="ml-7 mt-2">
                <div className="flex items-center gap-2">
                  <ModeButton active={config.aboutMode === "single"} onClick={() => set("aboutMode", "single")}>{t("createBots.nameSingle")}</ModeButton>
                  <ModeButton active={config.aboutMode === "from_file"} onClick={() => set("aboutMode", "from_file")}>{t("createBots.nameFromFile")}</ModeButton>
                </div>
                {config.aboutMode === "single" && (
                  <input
                    value={config.aboutSingle}
                    onChange={(e) => set("aboutSingle", e.target.value)}
                    placeholder={t("createBots.about")}
                    className="mt-2 w-full max-w-md rounded-md border border-border bg-background px-3 py-1.5 text-sm outline-none focus:border-primary/50"
                  />
                )}
                {config.aboutMode === "from_file" && (
                  <div className="flex items-center gap-2 mt-2">
                    <button onClick={() => selectFile("aboutFilePath")} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition"><FolderOpen className="h-4 w-4" /> {t("common.selectFile")}</button>
                    <span className="text-xs text-muted-foreground truncate max-w-sm">{config.aboutFilePath ? config.aboutFilePath.split(/[/\\]/).pop() : t("common.notSelected")}</span>
                  </div>
                )}
              </div>
            )}
          </div>

          {/* photo */}
          <div>
            <label className="flex items-center gap-3 cursor-pointer">
              <input type="checkbox" checked={config.setPhoto} onChange={(e) => set("setPhoto", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
              <span className="text-sm font-medium text-foreground">{t("createBots.photo")}</span>
            </label>
            {config.setPhoto && (
              <div className="ml-7 mt-2">
                <div className="flex items-center gap-2">
                  <ModeButton active={config.photoMode === "single"} onClick={() => set("photoMode", "single")}>{t("createBots.nameSingle")}</ModeButton>
                  <ModeButton active={config.photoMode === "from_folder"} onClick={() => set("photoMode", "from_folder")}>{t("createBots.nameFromFile")}</ModeButton>
                </div>
                {config.photoMode === "single" && (
                  <div className="flex items-center gap-2 mt-2">
                    <button onClick={selectPhoto} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition"><FolderOpen className="h-4 w-4" /> {t("common.selectFile")}</button>
                    <span className="text-xs text-muted-foreground truncate max-w-sm">{config.photoSinglePath ? config.photoSinglePath.split(/[/\\]/).pop() : t("common.notSelected")}</span>
                  </div>
                )}
                {config.photoMode === "from_folder" && (
                  <div className="flex items-center gap-2 mt-2">
                    <button onClick={() => selectFolder("photoFolderPath")} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition"><FolderOpen className="h-4 w-4" /> {t("common.selectFolder")}</button>
                    <span className="text-xs text-muted-foreground truncate max-w-sm">{config.photoFolderPath ? config.photoFolderPath.split(/[/\\]/).pop() : t("common.notSelected")}</span>
                  </div>
                )}
              </div>
            )}
          </div>

          {/* privacy */}
          <div>
            <label className="flex items-center gap-3 cursor-pointer">
              <input type="checkbox" checked={config.setPrivacy} onChange={(e) => set("setPrivacy", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
              <span className="text-sm font-medium text-foreground">{t("createBots.setPrivacy")}</span>
            </label>
          </div>

        </div>
      </div>

      {/* controls */}
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-6 text-sm font-semibold">
          <span>{t("common.done")}: <span className="text-[oklch(0.65_0.1_150)]">{stats.done}</span></span>
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

function ModeButton({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button onClick={onClick} className={`rounded-md border px-3 py-1 text-xs font-medium transition ${active ? "border-primary/50 bg-primary/10 text-primary" : "border-border bg-background text-muted-foreground hover:border-primary/30"}`}>
      {children}
    </button>
  );
}
