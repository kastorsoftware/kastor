import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Play, Square, FolderOpen } from "lucide-react";
import { AccountPickerModal } from "@/components/AccountPickerModal";
import { ThreadInput } from "@/components/ThreadInput";
import { useT } from "@/i18n";

type MediaType = "photo" | "video";
type StoryDuration = "6h" | "12h" | "24h" | "48h";
type StoryPrivacy = "all" | "contacts";

const DURATION_OPTIONS: { value: StoryDuration; label: string }[] = [
  { value: "6h", label: "6h" },
  { value: "12h", label: "12h" },
  { value: "24h", label: "24h" },
  { value: "48h", label: "48h" },
];

const DURATION_SECONDS: Record<StoryDuration, number> = {
  "6h": 21600,
  "12h": 43200,
  "24h": 86400,
  "48h": 172800,
};

interface StoriesConfig {
  mediaType: MediaType;
  mediaPath: string;
  mediaPaths: string;        // multiline list of paths (one per line)
  distributeMode: "all" | "unique";
  caption: string;
  tagUsers: boolean;
  tagFilePath: string;
  duration: StoryDuration;
  privacy: StoryPrivacy;
  maxFloodWait: number;
  threads: number;
  maxStoriesPerAccount: number;
  storiesMin: number;
  storiesMax: number;
  delayMin: number;
  delayMax: number;
  pinned: boolean;
  outputLinksPath: string;
}

const defaultConfig: StoriesConfig = {
  mediaType: "photo",
  mediaPath: "",
  mediaPaths: "",
  distributeMode: "all",
  caption: "",
  tagUsers: false,
  tagFilePath: "",
  duration: "24h",
  privacy: "all",
  maxFloodWait: 60,
  threads: 5,
  maxStoriesPerAccount: 1,
  storiesMin: 0,
  storiesMax: 0,
  delayMin: 0,
  delayMax: 0,
  pinned: true,
  outputLinksPath: "",
};

const STORAGE_KEY = "stories_config";
const IS_DEV = !("__TAURI_INTERNALS__" in window);

function loadSavedConfig(): StoriesConfig {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return { ...defaultConfig, ...JSON.parse(saved) };
  } catch {}
  return defaultConfig;
}

export function StoriesPage() {
  const t = useT();
  const [config, setConfig] = useState<StoriesConfig>(loadSavedConfig);
  const [running, setRunning] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [stats, setStats] = useState({ done: 0, errors: 0, inProgress: 0 });
  const [pickerOpen, setPickerOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const logsRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(config));
  }, [config]);

  useEffect(() => {
    if (IS_DEV) return;
    const unlisten = listen<string>("stories-log", (e) => {
      setLogs((prev) => [...prev, e.payload]);
      const msg = e.payload;
      if (msg === "Завершено" || msg === "Done") {
        setRunning(false);
      } else if (msg.includes("ОШИБКА") || msg.includes("ошибка:") || msg.includes("ERROR") || msg.includes("error:")) {
        setStats((s) => ({ ...s, errors: s.errors + 1 }));
      } else if (msg.includes("выполнено") || msg.includes("успешно") || msg.includes("done") || msg.includes("uploaded")) {
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

  const set = <K extends keyof StoriesConfig>(key: K, value: StoriesConfig[K]) => {
    setConfig((prev) => ({ ...prev, [key]: value }));
    setError(null);
  };

  const selectMedia = async () => {
    if (IS_DEV) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const filters = config.mediaType === "photo"
      ? [{ name: "Images", extensions: ["jpg", "jpeg", "png"] }]
      : [{ name: "Video", extensions: ["mp4", "mov"] }];
    const selected = await open({ multiple: false, filters });
    if (selected) set("mediaPath", selected as string);
  };

  const selectTagFile = async () => {
    if (IS_DEV) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const selected = await open({
      multiple: false,
      filters: [{ name: "Text files", extensions: ["txt"] }],
    });
    if (selected) set("tagFilePath", selected as string);
  };

  const validate = (): string | null => {
    if (!config.mediaPath.trim() && !config.mediaPaths.trim()) return t("stories.errNoMedia");
    if (config.tagUsers && !config.tagFilePath.trim()) return t("stories.errNoTagFile");
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

    // validate: if tagging, check that accounts * maxStories can cover all tags
    if (config.tagUsers && config.tagFilePath) {
      try {
        const lines = await invoke<string[]>("read_authkey_txt", { path: config.tagFilePath });
        const tagCount = lines.filter((l: string) => l.trim().length > 0).length;
        const capacity = ids.length * config.maxStoriesPerAccount;
        if (capacity < tagCount) {
          setError(`${t("stories.errCapacity")}: ${ids.length} x ${config.maxStoriesPerAccount} = ${capacity}, ${t("stories.errTags")} ${tagCount}`);
          setRunning(false);
          return;
        }
      } catch {}
    }

    setLogs([]);
    setStats({ done: 0, errors: 0, inProgress: ids.length });
    setRunning(true);
    setError(null);
    try {
      const tid = await invoke<string>("stories_start", {
        ids,
        config: {
          media_type: config.mediaType,
          media_path: config.mediaPath.trim(),
          media_paths: config.mediaPaths.split("\n").map(l => l.trim()).filter(Boolean),
          distribute_mode: config.distributeMode,
          caption: config.caption.trim(),
          tag_users: config.tagUsers,
          tag_file_path: config.tagUsers ? config.tagFilePath.trim() : "",
          duration_seconds: DURATION_SECONDS[config.duration],
          max_flood_wait: config.maxFloodWait,
          privacy: config.privacy,
          max_stories_per_account: config.maxStoriesPerAccount,
          stories_min: config.storiesMin,
          stories_max: config.storiesMax,
          delay_min: config.delayMin,
          delay_max: config.delayMax,
          pinned: config.pinned,
          output_links_path: config.outputLinksPath.trim(),
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
    if (!IS_DEV && taskId) await invoke("stories_stop", { taskId }).catch(() => {});
    setRunning(false);
    setLogs((prev) => [...prev, t("common.stoppedByUser")]);
  };

  // calculate how many tags fit per story

  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">{t("descriptions.stories")}</p>
      {/* config panel */}
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div className="p-6 space-y-5">

          {/* media type */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("stories.mediaTypeLabel")}</label>
            <select
              value={config.mediaType}
              onChange={(e) => set("mediaType", e.target.value as MediaType)}
              className="mt-1.5 w-full max-w-xs rounded-md border border-border bg-card px-3 py-2 text-sm outline-none focus:border-primary/50 appearance-none bg-[url('data:image/svg+xml;charset=utf-8,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%20width%3D%2216%22%20height%3D%2216%22%20viewBox%3D%220%200%2024%2024%22%20fill%3D%22none%22%20stroke%3D%22%23888%22%20stroke-width%3D%222%22%3E%3Cpath%20d%3D%22m6%209%206%206%206-6%22%2F%3E%3C%2Fsvg%3E')] bg-[length:16px] bg-[right_8px_center] bg-no-repeat pr-8"
            >
              <option value="photo">{t("stories.mediaPhoto")}</option>
              <option value="video">{t("stories.mediaVideo")}</option>
            </select>
          </div>

          {/* media file */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("stories.mediaFileLabel")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <button
                onClick={selectMedia}
                className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition"
              >
                <FolderOpen className="h-4 w-4" />
                {t("common.selectFile")}
              </button>
              <span className="text-xs text-muted-foreground truncate max-w-sm">
                {config.mediaPath || t("common.notSelected")}
              </span>
            </div>
            <div className="mt-2">
              <label className="text-sm font-medium text-foreground">{t("stories.mediaListLabel")}</label>
              <textarea
                value={config.mediaPaths}
                onChange={(e) => set("mediaPaths", e.target.value)}
                placeholder={"C:\\photos\\1.jpg\nC:\\photos\\2.jpg"}
                rows={2}
                className="mt-1.5 w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50 resize-y"
              />
              <p className="text-xs text-muted-foreground mt-1">{t("stories.mediaListHint")}</p>
            </div>
            <div className="flex items-center gap-2 mt-2">
              <ModeBtn active={config.distributeMode === "all"} onClick={() => set("distributeMode", "all")}>{t("stories.distributeAll")}</ModeBtn>
              <ModeBtn active={config.distributeMode === "unique"} onClick={() => set("distributeMode", "unique")}>{t("stories.distributeUnique")}</ModeBtn>
            </div>
          </div>

          {/* caption */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("stories.caption")}</label>
            <textarea
              value={config.caption}
              onChange={(e) => set("caption", e.target.value)}
              placeholder={t("stories.captionPlaceholder")}
              rows={3}
              className="mt-1.5 w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50 resize-y"
            />
          </div>

          {/* tag users */}
          <label className="flex items-center gap-3 cursor-pointer">
            <input
              type="checkbox"
              checked={config.tagUsers}
              onChange={(e) => set("tagUsers", e.target.checked)}
              className="rounded border-border accent-primary h-4 w-4"
            />
            <span className="text-sm font-medium text-foreground">{t("stories.tagUsers")}</span>
          </label>

          {config.tagUsers && (
            <div className="ml-7 space-y-2">
              <div className="flex items-center gap-2">
                <button
                  onClick={selectTagFile}
                  className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition"
                >
                  <FolderOpen className="h-4 w-4" />
                  {t("common.selectFile")}
                </button>
                <span className="text-xs text-muted-foreground truncate max-w-sm">
                  {config.tagFilePath || t("common.notSelected")}
                </span>
              </div>
              <p className="text-xs text-muted-foreground">{t("stories.tagFileHint")}</p>
            </div>
          )}

          {/* max stories per account */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("stories.storiesPerAccount")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <input
                type="number"
                min={1}
                max={30}
                value={config.maxStoriesPerAccount}
                onChange={(e) => set("maxStoriesPerAccount", Math.max(1, Math.min(30, Number(e.target.value) || 1)))}
                className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50"
              />
            </div>
            <p className="text-xs text-muted-foreground mt-1">{t("stories.randomLabel")}:</p>
            <div className="flex items-center gap-2 mt-1">
              <input type="number" min={0} max={30} value={config.storiesMin} onChange={(e) => set("storiesMin", Math.max(0, Number(e.target.value)))} placeholder={t("common.delayMin")} className="w-16 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
              <span className="text-muted-foreground text-sm">—</span>
              <input type="number" min={0} max={30} value={config.storiesMax} onChange={(e) => set("storiesMax", Math.max(0, Number(e.target.value)))} placeholder={t("common.delayMax")} className="w-16 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
              <span className="text-xs text-muted-foreground">{t("stories.randomHint")}</span>
            </div>
          </div>

          {/* delay between stories */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("stories.delayLabel")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <input type="number" min={0} max={3600} value={config.delayMin} onChange={(e) => set("delayMin", Math.max(0, Number(e.target.value)))} className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
              <span className="text-muted-foreground text-sm">—</span>
              <input type="number" min={0} max={3600} value={config.delayMax} onChange={(e) => set("delayMax", Math.max(0, Number(e.target.value)))} className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
              <span className="text-xs text-muted-foreground">{t("stories.delayHint")}</span>
            </div>
          </div>

          {/* pinned + links */}
          <label className="flex items-center gap-3 cursor-pointer">
            <input type="checkbox" checked={config.pinned} onChange={(e) => set("pinned", e.target.checked)} className="rounded border-border accent-primary h-4 w-4" />
            <span className="text-sm font-medium text-foreground">{t("stories.pinToProfile")}</span>
          </label>

          {/* duration + privacy */}
          <div className="grid grid-cols-2 gap-4">
            <div>
              <label className="text-sm font-medium text-foreground">{t("stories.duration")}</label>
              <select
                value={config.duration}
                onChange={(e) => set("duration", e.target.value as StoryDuration)}
                className="mt-1 w-full rounded-md border border-border bg-card px-3 py-2 text-sm outline-none focus:border-primary/50 appearance-none bg-[url('data:image/svg+xml;charset=utf-8,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%20width%3D%2216%22%20height%3D%2216%22%20viewBox%3D%220%200%2024%2024%22%20fill%3D%22none%22%20stroke%3D%22%23888%22%20stroke-width%3D%222%22%3E%3Cpath%20d%3D%22m6%209%206%206%206-6%22%2F%3E%3C%2Fsvg%3E')] bg-[length:16px] bg-[right_8px_center] bg-no-repeat pr-8"
              >
                {DURATION_OPTIONS.map((o) => (
                  <option key={o.value} value={o.value}>{o.label}</option>
                ))}
              </select>
            </div>
            <div>
              <label className="text-sm font-medium text-foreground">{t("stories.privacy")}</label>
              <select
                value={config.privacy}
                onChange={(e) => set("privacy", e.target.value as StoryPrivacy)}
                className="mt-1 w-full rounded-md border border-border bg-card px-3 py-2 text-sm outline-none focus:border-primary/50 appearance-none bg-[url('data:image/svg+xml;charset=utf-8,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%20width%3D%2216%22%20height%3D%2216%22%20viewBox%3D%220%200%2024%2024%22%20fill%3D%22none%22%20stroke%3D%22%23888%22%20stroke-width%3D%222%22%3E%3Cpath%20d%3D%22m6%209%206%206%206-6%22%2F%3E%3C%2Fsvg%3E')] bg-[length:16px] bg-[right_8px_center] bg-no-repeat pr-8"
              >
                <option value="all">{t("stories.privacyAll")}</option>
                <option value="contacts">{t("stories.privacyContacts")}</option>
              </select>
            </div>
          </div>

          {/* flood wait */}
          <div className="border-t border-border pt-4">
            <label className="text-sm font-medium text-foreground">{t("common.maxFloodWait")}</label>
            <div className="flex items-center gap-2 mt-1.5">
              <input
                type="number"
                min={0}
                max={86400}
                value={config.maxFloodWait}
                onChange={(e) => set("maxFloodWait", Math.max(0, Math.min(86400, Number(e.target.value))))}
                className="w-28 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50"
              />
              <span className="text-xs text-muted-foreground">0 = ∞</span>
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
          <span>{t("common.done")}: <span className="text-[oklch(0.65_0.1_150)]">{stats.done}</span></span>
          <span>{t("common.errors")}: <span className="text-[oklch(0.55_0.1_25)]">{stats.errors}</span></span>
          <span>{t("stories.remaining")}: <span className="text-[oklch(0.65_0.1_280)]">{Math.max(0, stats.inProgress - stats.done - stats.errors)}</span></span>
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
    </div>
  );
}

function ModeBtn({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button onClick={onClick} className={`rounded-md border px-3 py-1.5 text-xs font-medium transition ${active ? "border-primary/50 bg-primary/10 text-primary" : "border-border bg-background text-muted-foreground hover:border-primary/30"}`}>
      {children}
    </button>
  );
}
