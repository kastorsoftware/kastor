import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Play, Square } from "lucide-react";
import { AccountPickerModal } from "@/components/AccountPickerModal";
import { useT } from "@/i18n";
import { isDone, isError, isCopied, isClonerSkipped } from "@/lib/eventParser";

type DestinationMode = "new_channel" | "existing";
type DestinationVisibility = "public" | "private";

interface ClonerConfig {
  // source
  sourceChannel: string;
  sourceFromId: number;
  sourceToId: number;

  // content filters
  copyDocuments: boolean;
  copyPhotos: boolean;
  copyVideos: boolean;
  copyMessagesWithVideo: boolean;

  showLinkPreview: boolean;
  forwardExternalLinks: boolean;
  forwardTelegramLinks: boolean;

  // size limits in MB; 0 = unlimited
  maxVideoSizeMb: number;
  maxFileSizeMb: number;
  maxPhotoSizeMb: number;

  // destination
  destinationMode: DestinationMode;

  // existing channel
  existingChannelId: string;

  // new channel
  newChannelVisibility: DestinationVisibility;
  newChannelUsername: string;
  copyTitle: boolean;
  copyDescription: boolean;
  copyPhoto: boolean;

  // textual transforms
  // 1 line = "from:to"
  replacements: string;
  // optional split form
  replaceFromList: string;
  replaceToList: string;

  // 1 word per line — if any matches the post is skipped
  skipKeywords: string;

  // pacing & resilience
  delayMinSec: number;
  delayMaxSec: number;
  maxFloodWait: number;

  // preserve reply chains within the source channel
  preserveReplies: boolean;
}

const defaultConfig: ClonerConfig = {
  sourceChannel: "",
  sourceFromId: 0,
  sourceToId: 0,

  copyDocuments: true,
  copyPhotos: true,
  copyVideos: true,
  copyMessagesWithVideo: true,

  showLinkPreview: true,
  forwardExternalLinks: true,
  forwardTelegramLinks: true,

  maxVideoSizeMb: 0,
  maxFileSizeMb: 0,
  maxPhotoSizeMb: 0,

  destinationMode: "new_channel",
  existingChannelId: "",

  newChannelVisibility: "public",
  newChannelUsername: "",
  copyTitle: true,
  copyDescription: true,
  copyPhoto: true,

  replacements: "",
  replaceFromList: "",
  replaceToList: "",

  skipKeywords: "",

  delayMinSec: 5,
  delayMaxSec: 15,
  maxFloodWait: 60,

  preserveReplies: true,
};

const STORAGE_KEY = "cloner_config";
const IS_DEV = !("__TAURI_INTERNALS__" in window);

function loadSavedConfig(): ClonerConfig {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return { ...defaultConfig, ...JSON.parse(saved) };
  } catch {}
  return defaultConfig;
}

export function ClonerPage() {
  const t = useT();
  const [config, setConfig] = useState<ClonerConfig>(loadSavedConfig);
  const [running, setRunning] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [stats, setStats] = useState({ copied: 0, skipped: 0, errors: 0 });
  const [pickerOpen, setPickerOpen] = useState(false);
  const logsRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(config));
  }, [config]);

  useEffect(() => {
    if (IS_DEV) return;
    const unlisten = listen<string>("cloner-log", (e) => {
      setLogs((prev) => [...prev, e.payload]);
      const msg = e.payload;
      if (isDone(msg)) {
        setRunning(false);
      } else if (isError(msg)) {
        setStats((s) => ({ ...s, errors: s.errors + 1 }));
      } else if (isClonerSkipped(msg)) {
        setStats((s) => ({ ...s, skipped: s.skipped + 1 }));
      } else if (isCopied(msg)) {
        setStats((s) => ({ ...s, copied: s.copied + 1 }));
      }
    });
    return () => { unlisten.then((f) => f()); };
  }, []);

  useEffect(() => {
    const el = logsRef.current;
    if (!el) return;
    // only autoscroll if the user is already pinned near the bottom — letting
    // them scroll up and read older logs without being yanked back.
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    if (atBottom) el.scrollTop = el.scrollHeight;
  }, [logs]);

  const set = <K extends keyof ClonerConfig>(key: K, value: ClonerConfig[K]) => {
    setConfig((prev) => ({ ...prev, [key]: value }));
  };

  const validate = (): string | null => {
    if (!config.sourceChannel.trim()) return t("validation.noSourceChannel");
    if (config.sourceFromId < 0 || config.sourceToId < 0) return t("validation.negativeId");
    if (config.sourceFromId !== 0 && config.sourceToId !== 0 && config.sourceFromId > config.sourceToId) {
      return t("validation.fromIdGreaterThanTo");
    }
    if (config.delayMinSec < 3) return t("validation.minDelay3sec");
    if (config.delayMaxSec > 60) return t("validation.maxDelay60sec");
    if (config.delayMinSec > config.delayMaxSec) return t("validation.delayMinGreaterMax");
    if (config.destinationMode === "existing" && !config.existingChannelId.trim()) {
      return t("validation.noExistingChannelId");
    }
    if (config.destinationMode === "new_channel" && config.newChannelVisibility === "public" && !config.newChannelUsername.trim()) {
      return t("validation.publicNeedsUsername");
    }
    if (parseReplacements(config).invalid.length > 0) {
      return `invalid replacement lines: ${parseReplacements(config).invalid.slice(0, 3).join(", ")}`;
    }
    return null;
  };

  const handleStart = () => {
    const err = validate();
    if (err) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${err}`]);
      return;
    }
    setPickerOpen(true);
  };

  const handleAccountSelected = async (ids: string[]) => {
    if (ids.length === 0) return;
    // cloner is single-threaded — use the first selected account
    const id = ids[0];
    setLogs([]);
    setStats({ copied: 0, skipped: 0, errors: 0 });
    setRunning(true);
    try {
      const tid = await invoke<string>("cloner_start", {
        accountId: id,
        config: serializeConfig(config),
        maxFloodWait: config.maxFloodWait,
      });
      setTaskId(tid);
    } catch (e: any) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${e}`]);
      setRunning(false);
    }
  };

  const handleStop = async () => {
    if (!IS_DEV && taskId) await invoke("cloner_stop", { taskId }).catch(() => {});
    setRunning(false);
    setLogs((prev) => [...prev, t("common.stoppedByUser")]);
  };

  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">{t("descriptions.cloner")}</p>
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div className="p-6 space-y-6">

          {/* source */}
          <Section title={t("cloner.sourceChannel")}>
            <Field label={t("cloner.sourceChannel")}>
              <input
                value={config.sourceChannel}
                onChange={(e) => set("sourceChannel", e.target.value)}
                placeholder="@channel, https://t.me/channel"
                className="mt-1.5 w-full max-w-md rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50"
              />
            </Field>

            <Field label={t("cloner.fromId") + " — " + t("cloner.toId")}>
              <div className="flex items-center gap-2 mt-1.5">
                <input
                  type="number"
                  min={0}
                  value={config.sourceFromId}
                  onChange={(e) => set("sourceFromId", Math.max(0, Number(e.target.value)))}
                  className="w-28 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50"
                />
                <span className="text-xs text-muted-foreground">—</span>
                <input
                  type="number"
                  min={0}
                  value={config.sourceToId}
                  onChange={(e) => set("sourceToId", Math.max(0, Number(e.target.value)))}
                  className="w-28 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50"
                />
                <span className="text-xs text-muted-foreground">{t("cloner.rangeHint")}</span>
              </div>
            </Field>
          </Section>

          {/* content filters */}
          <Section title={t("cloner.copyDocuments")}>
            <div className="grid grid-cols-2 gap-y-2 gap-x-6">
              <CheckRow label={t("cloner.copyDocuments")} checked={config.copyDocuments} onChange={(v) => set("copyDocuments", v)} />
              <CheckRow label={t("cloner.copyPhotos")} checked={config.copyPhotos} onChange={(v) => set("copyPhotos", v)} />
              <CheckRow label={t("cloner.copyVideos")} checked={config.copyVideos} onChange={(v) => set("copyVideos", v)} />
              <CheckRow label={t("cloner.copyMessagesWithVideo")} checked={config.copyMessagesWithVideo} onChange={(v) => set("copyMessagesWithVideo", v)} />
              <CheckRow label={t("cloner.showLinkPreview")} checked={config.showLinkPreview} onChange={(v) => set("showLinkPreview", v)} />
              <CheckRow label={t("cloner.preserveReplies")} checked={config.preserveReplies} onChange={(v) => set("preserveReplies", v)} />
              <CheckRow label={t("cloner.forwardExternalLinks")} checked={config.forwardExternalLinks} onChange={(v) => set("forwardExternalLinks", v)} />
              <CheckRow label={t("cloner.forwardTelegramLinks")} checked={config.forwardTelegramLinks} onChange={(v) => set("forwardTelegramLinks", v)} />
            </div>
          </Section>

          {/* limits */}
          <Section title={t("cloner.maxVideoSize")}>
            <div className="grid grid-cols-3 gap-4">
              <SizeInput label={t("cloner.maxVideoSize")} value={config.maxVideoSizeMb} onChange={(v) => set("maxVideoSizeMb", v)} />
              <SizeInput label={t("cloner.maxFileSize")} value={config.maxFileSizeMb} onChange={(v) => set("maxFileSizeMb", v)} />
              <SizeInput label={t("cloner.maxPhotoSize")} value={config.maxPhotoSizeMb} onChange={(v) => set("maxPhotoSizeMb", v)} />
            </div>
          </Section>

          {/* destination */}
          <Section title={t("cloner.destinationNew")}>
            <div className="flex items-center gap-2">
              <ModeButton active={config.destinationMode === "new_channel"} onClick={() => set("destinationMode", "new_channel")}>{t("cloner.destinationNew")}</ModeButton>
              <ModeButton active={config.destinationMode === "existing"} onClick={() => set("destinationMode", "existing")}>{t("cloner.destinationExisting")}</ModeButton>
            </div>

            {config.destinationMode === "new_channel" && (
              <div className="mt-3 space-y-3">
                <div className="flex items-center gap-2">
                  <ModeButton active={config.newChannelVisibility === "public"} onClick={() => set("newChannelVisibility", "public")}>{t("cloner.newPublic")}</ModeButton>
                  <ModeButton active={config.newChannelVisibility === "private"} onClick={() => set("newChannelVisibility", "private")}>{t("cloner.newPrivate")}</ModeButton>
                </div>
                {config.newChannelVisibility === "public" && (
                  <Field label={t("cloner.newChannelUsername")}>
                    <input
                      value={config.newChannelUsername}
                      onChange={(e) => set("newChannelUsername", e.target.value.replace(/[^a-zA-Z0-9_]/g, ""))}
                      placeholder="myCloneChannel"
                      maxLength={32}
                      className="mt-1.5 w-full max-w-md rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50"
                    />
                    <p className="mt-1 text-xs text-muted-foreground">{t("cloner.usernameHint")}</p>
                  </Field>
                )}
                <div className="grid grid-cols-3 gap-y-2 gap-x-6">
                  <CheckRow label={t("cloner.copyTitle")} checked={config.copyTitle} onChange={(v) => set("copyTitle", v)} />
                  <CheckRow label={t("cloner.copyDescription")} checked={config.copyDescription} onChange={(v) => set("copyDescription", v)} />
                  <CheckRow label={t("cloner.copyPhoto")} checked={config.copyPhoto} onChange={(v) => set("copyPhoto", v)} />
                </div>
              </div>
            )}

            {config.destinationMode === "existing" && (
              <Field label={t("cloner.existingChannelLabel")}>
                <input
                  value={config.existingChannelId}
                  onChange={(e) => set("existingChannelId", e.target.value)}
                  placeholder="@mychannel или -1001234567890"
                  className="mt-1.5 w-full max-w-md rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50"
                />
                <p className="mt-1 text-xs text-muted-foreground">{t("cloner.existingChannelHint")}</p>
              </Field>
            )}
          </Section>

          {/* replacements */}
          <Section title={t("cloner.replacements")}>
            <Field label={t("cloner.replacements")}>
              <textarea
                value={config.replacements}
                onChange={(e) => set("replacements", e.target.value)}
                placeholder={"old_word:new_word\n@oldchannel:@newchannel"}
                rows={4}
                className="mt-1.5 w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50 font-mono resize-y block"
              />
            </Field>

            <details className="text-xs">
              <summary className="cursor-pointer text-muted-foreground hover:text-foreground transition">{t("cloner.altInputTitle")}</summary>
              <div className="mt-2 grid grid-cols-2 gap-4 max-w-2xl items-start">
                <Field label={t("cloner.replaceFrom")}>
                  <textarea
                    value={config.replaceFromList}
                    onChange={(e) => set("replaceFromList", e.target.value)}
                    rows={4}
                    className="mt-1.5 w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50 font-mono resize-y"
                  />
                </Field>
                <Field label={t("cloner.replaceTo")}>
                  <textarea
                    value={config.replaceToList}
                    onChange={(e) => set("replaceToList", e.target.value)}
                    rows={4}
                    className="mt-1.5 w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50 font-mono resize-y"
                  />
                </Field>
              </div>
              <p className="mt-1 text-xs text-muted-foreground">{t("cloner.altInputHint")}</p>
            </details>
          </Section>

          {/* skip keywords */}
          <Section title={t("cloner.skipKeywords")}>
            <Field label={t("cloner.skipKeywords")}>
              <textarea
                value={config.skipKeywords}
                onChange={(e) => set("skipKeywords", e.target.value)}
                placeholder={"реклама\n#реклама\nspam"}
                rows={3}
                className="mt-1.5 w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50 font-mono resize-y block"
              />
            </Field>
          </Section>

          {/* pacing */}
          <Section title={t("common.delay")}>
            <Field label={t("common.delay")}>
              <div className="flex items-center gap-2 mt-1.5">
                <input
                  type="number"
                  min={3}
                  max={60}
                  value={config.delayMinSec}
                  onChange={(e) => set("delayMinSec", clampInt(e.target.value, 3, 60))}
                  className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50"
                />
                <span className="text-xs text-muted-foreground">—</span>
                <input
                  type="number"
                  min={3}
                  max={60}
                  value={config.delayMaxSec}
                  onChange={(e) => set("delayMaxSec", clampInt(e.target.value, 3, 60))}
                  className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50"
                />
                <span className="text-xs text-muted-foreground">{t("cloner.delayHint")}</span>
              </div>
            </Field>
            <Field label={t("common.maxFloodWait")}>
              <div className="flex items-center gap-2 mt-1.5">
                <input
                  type="number"
                  min={0}
                  max={86400}
                  value={config.maxFloodWait}
                  onChange={(e) => set("maxFloodWait", clampInt(e.target.value, 0, 86400))}
                  className="w-28 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50"
                />
                <span className="text-xs text-muted-foreground">0 = no limit</span>
              </div>
            </Field>
            <p className="text-xs text-muted-foreground">{t("cloner.singleThreadHint")}</p>
          </Section>

        </div>
      </div>

      {/* controls */}
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-6 text-sm font-semibold">
          <span>{t("common.done")}: <span className="text-[oklch(0.65_0.1_150)]">{stats.copied}</span></span>
          <span>{t("common.total")}: <span className="text-[oklch(0.65_0.1_280)]">{stats.skipped}</span></span>
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

function clampInt(raw: string, min: number, max: number): number {
  const n = Number(raw);
  if (!Number.isFinite(n)) return min;
  return Math.max(min, Math.min(max, Math.round(n)));
}

interface ParsedReplacements {
  pairs: Array<[string, string]>;
  invalid: string[];
}

function parseReplacements(c: ClonerConfig): ParsedReplacements {
  const pairs: Array<[string, string]> = [];
  const invalid: string[] = [];

  for (const raw of c.replacements.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line) continue;
    const idx = line.indexOf(":");
    if (idx <= 0 || idx === line.length - 1) {
      invalid.push(line);
      continue;
    }
    pairs.push([line.slice(0, idx), line.slice(idx + 1)]);
  }

  // optional split form: pair lines together
  const fromLines = c.replaceFromList.split(/\r?\n/).map((s) => s.trim()).filter(Boolean);
  const toLines = c.replaceToList.split(/\r?\n/).map((s) => s.trim()).filter(Boolean);
  const n = Math.min(fromLines.length, toLines.length);
  for (let i = 0; i < n; i++) pairs.push([fromLines[i], toLines[i]]);

  return { pairs, invalid };
}

function serializeConfig(c: ClonerConfig) {
  const { pairs } = parseReplacements(c);
  return {
    source_channel: c.sourceChannel.trim(),
    source_from_id: c.sourceFromId,
    source_to_id: c.sourceToId,

    copy_documents: c.copyDocuments,
    copy_photos: c.copyPhotos,
    copy_videos: c.copyVideos,
    copy_messages_with_video: c.copyMessagesWithVideo,

    show_link_preview: c.showLinkPreview,
    forward_external_links: c.forwardExternalLinks,
    forward_telegram_links: c.forwardTelegramLinks,

    max_video_size_mb: c.maxVideoSizeMb,
    max_file_size_mb: c.maxFileSizeMb,
    max_photo_size_mb: c.maxPhotoSizeMb,

    destination_mode: c.destinationMode,
    existing_channel_id: c.existingChannelId.trim(),
    new_channel_visibility: c.newChannelVisibility,
    new_channel_username: c.newChannelUsername.trim(),
    copy_title: c.copyTitle,
    copy_description: c.copyDescription,
    copy_photo: c.copyPhoto,

    replacements: pairs,
    skip_keywords: c.skipKeywords
      .split(/\r?\n/)
      .map((s) => s.trim())
      .filter(Boolean),

    delay_min_sec: c.delayMinSec,
    delay_max_sec: c.delayMaxSec,
    preserve_replies: c.preserveReplies,
  };
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <h3 className="text-sm font-semibold text-foreground border-b border-border pb-1.5 mb-3">{title}</h3>
      <div className="space-y-3">{children}</div>
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <label className="block text-sm font-medium text-foreground">{label}</label>
      {children}
    </div>
  );
}

function CheckRow({ label, checked, onChange }: { label: string; checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <label className="flex items-center gap-2.5 cursor-pointer text-sm">
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        className="rounded border-border accent-primary h-4 w-4"
      />
      <span className="text-foreground">{label}</span>
    </label>
  );
}

function ModeButton({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button onClick={onClick} className={`rounded-md border px-3 py-1.5 text-xs font-medium transition ${active ? "border-primary/50 bg-primary/10 text-primary" : "border-border bg-background text-muted-foreground hover:border-primary/30"}`}>
      {children}
    </button>
  );
}

function SizeInput({ label, value, onChange }: { label: string; value: number; onChange: (v: number) => void }) {
  return (
    <div>
      <label className="text-xs text-muted-foreground">{label}</label>
      <div className="flex items-center gap-1.5 mt-1">
        <input
          type="number"
          min={0}
          max={4096}
          value={value}
          onChange={(e) => onChange(clampInt(e.target.value, 0, 4096))}
          className="w-24 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50"
        />
        <span className="text-xs text-muted-foreground">MB</span>
      </div>
    </div>
  );
}
