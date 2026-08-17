import { useState, useEffect, useRef, lazy, Suspense } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Play, Square } from "lucide-react";
import { AccountPickerModal } from "@/components/AccountPickerModal";
import { ThreadInput } from "@/components/ThreadInput";
import { useT } from "@/i18n";
import { isDone, isError } from "@/lib/eventParser";

const UserLookupPage = lazy(() => import("@/components/UserLookupPage").then(m => ({ default: m.UserLookupPage })));

type ParserMode = "group" | "channel-admin" | "messages" | "comments" | "user-lookup";

interface ParserConfig {
  mode: ParserMode;

  // targets — список ссылок (multiline)
  targets: string;

  // online-state filters
  parseDeleted: boolean;
  parseRecent: boolean;
  parseWeek: boolean;
  parseMonth: boolean;
  parseLong: boolean;

  // misc filters
  premiumOnly: boolean;
  parseAdmins: boolean;
  parseBots: boolean;
  excludeAdmins: boolean;
  excludeNoUsername: boolean;

  // alphabet search (Python-style bypass for 10k limit)
  charsEn: boolean;
  charsRu: boolean;
  charsCn: boolean;
  charsAr: boolean;
  charsHe: boolean;
  charsFa: boolean;
  charsEmoji: boolean;

  // join/leave
  leaveAfter: boolean;

  // messages mode
  parsingDays: number;

  // output
  outputPath: string;
  createTxt: boolean;

  // FLOOD_WAIT cap (sec); 0 = unlimited
  maxFloodWait: number;

  threads: number;
}

const defaultConfig: ParserConfig = {
  mode: "group",

  targets: "",

  parseDeleted: false,
  parseRecent: true,
  parseWeek: false,
  parseMonth: false,
  parseLong: false,

  premiumOnly: false,
  parseAdmins: false,
  parseBots: false,
  excludeAdmins: false,
  excludeNoUsername: false,

  charsEn: true,
  charsRu: true,
  charsCn: false,
  charsAr: false,
  charsHe: false,
  charsFa: false,
  charsEmoji: false,

  leaveAfter: false,

  parsingDays: 30,

  outputPath: "",
  createTxt: false,

  maxFloodWait: 60,
  threads: 5,
};

const STORAGE_KEY = "parser_config";
const IS_DEV = !("__TAURI_INTERNALS__" in window);

function loadSavedConfig(): ParserConfig {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return { ...defaultConfig, ...JSON.parse(saved) };
  } catch {}
  return defaultConfig;
}

export function ParserPage() {
  const t = useT();
  const [config, setConfig] = useState<ParserConfig>(loadSavedConfig);
  const [running, setRunning] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [stats, setStats] = useState({ collected: 0, errors: 0, groupsDone: 0, groupsTotal: 0 });
  const [pickerOpen, setPickerOpen] = useState(false);
  const logsRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(config));
  }, [config]);

  useEffect(() => {
    if (IS_DEV) return;
    const unlisten = listen<string>("parser-log", (e) => {
      setLogs((prev) => [...prev, e.payload]);
      const msg = e.payload;
      if (isDone(msg)) {
        setRunning(false);
      } else if (isError(msg)) {
        setStats((s) => ({ ...s, errors: s.errors + 1 }));
      }
    });
    return () => { unlisten.then((f) => f()); };
  }, []);

  useEffect(() => {
    if (IS_DEV) return;
    const unlisten = listen<string>("parser-progress", (e) => {
      try {
        const data = JSON.parse(e.payload);
        setStats((s) => ({
          ...s,
          collected: data.users_total ?? s.collected,
          groupsDone: data.groups_done ?? s.groupsDone,
          groupsTotal: data.groups_total ?? s.groupsTotal,
        }));
      } catch {}
    });
    return () => { unlisten.then((f) => f()); };
  }, []);

  useEffect(() => {
    const el = logsRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    if (atBottom) el.scrollTop = el.scrollHeight;
  }, [logs]);

  const set = <K extends keyof ParserConfig>(key: K, value: ParserConfig[K]) => {
    setConfig((prev) => ({ ...prev, [key]: value }));
  };

  const validate = (): string | null => {
    if (!config.targets.trim()) return t("validation.noParseTargets");
    if (config.mode !== "messages" && config.mode !== "comments") {
      const noFilter =
        !config.parseDeleted && !config.parseRecent && !config.parseWeek &&
        !config.parseMonth && !config.parseLong;
      if (noFilter) return t("validation.noFilter");
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
    setLogs([]);
    setStats({ collected: 0, errors: 0, groupsDone: 0, groupsTotal: 0 });
    setRunning(true);
    try {
      const tid = await invoke<string>("parser_start", {
        ids,
        config: serializeConfig(config),
        threads: config.threads,
      });
      setTaskId(tid);
    } catch (e: any) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${e}`]);
      setRunning(false);
    }
  };

  const handleStop = async () => {
    if (!IS_DEV && taskId) await invoke("parser_stop", { taskId }).catch(() => {});
    setRunning(false);
    setLogs((prev) => [...prev, t("common.stoppedByUser")]);
  };

  const selectOutputFile = async () => {
    if (IS_DEV) return;
    const { save } = await import("@tauri-apps/plugin-dialog");
    const path = await save({ defaultPath: "parsed.db", filters: [{ name: "SQLite DB", extensions: ["db"] }] });
    if (path) set("outputPath", path);
  };

  const accountPickerTitle = t("parser.selectAccountsForParsing");

  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">{t("descriptions.parser")}</p>
      {/* mode tabs */}
      <div className="flex gap-2 flex-wrap">
        <ModeTab active={config.mode === "group"} onClick={() => set("mode", "group")}>{t("parser.modeGroupTab")}</ModeTab>
        <ModeTab active={config.mode === "channel-admin"} onClick={() => set("mode", "channel-admin")}>{t("parser.modeChannelAdminTab")}</ModeTab>
        <ModeTab active={config.mode === "messages"} onClick={() => set("mode", "messages")}>{t("parser.modeMessagesTab")}</ModeTab>
        <ModeTab active={config.mode === "comments"} onClick={() => set("mode", "comments")}>{t("parser.modeCommentsTab")}</ModeTab>
        <ModeTab active={config.mode === "user-lookup"} onClick={() => set("mode", "user-lookup")}>{t("parser.modeUserLookupTab")}</ModeTab>
      </div>

      {config.mode === "user-lookup" ? (
        <Suspense fallback={<div className="flex items-center justify-center py-12"><div className="h-6 w-6 animate-spin rounded-full border-2 border-primary/30 border-t-primary" /></div>}>
          <UserLookupPage />
        </Suspense>
      ) : (
      <>
      {/* config panel */}
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div className="p-6 space-y-6">

          <div className="flex flex-col">
            <label className="block text-sm font-medium text-foreground mb-1">{config.mode === "channel-admin" ? t("parser.linksToChannels") : config.mode === "messages" ? t("parser.linksToGroupsChannels") : config.mode === "comments" ? t("parser.linksToChannels") : t("parser.linksToGroups")}</label>
            <p className="text-xs text-muted-foreground mb-1.5">
              {t("parser.linksHint")}
            </p>
            <textarea
              value={config.targets}
              onChange={(e) => set("targets", e.target.value)}
              placeholder={"@username\nhttps://t.me/...\nhttps://t.me/+invite..."}
              rows={4}
              className="w-full max-w-md rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50 resize-y"
            />
          </div>

          {config.mode !== "messages" && config.mode !== "comments" && (
          <Section title={t("parser.alphabetTitle")}>
            <p className="text-xs text-muted-foreground mb-2">
              {t("parser.alphabetHint")}
            </p>
            <div className="grid grid-cols-2 sm:grid-cols-3 gap-y-2 gap-x-6">
              <CheckRow label={t("parser.charsLatin")} checked={config.charsEn} onChange={(v) => set("charsEn", v)} />
              <CheckRow label={t("parser.charsCyrillic")} checked={config.charsRu} onChange={(v) => set("charsRu", v)} />
              <CheckRow label={t("parser.charsChinese")} checked={config.charsCn} onChange={(v) => set("charsCn", v)} />
              <CheckRow label={t("parser.charsArabic")} checked={config.charsAr} onChange={(v) => set("charsAr", v)} />
              <CheckRow label={t("parser.charsHebrew")} checked={config.charsHe} onChange={(v) => set("charsHe", v)} />
              <CheckRow label={t("parser.charsFarsi")} checked={config.charsFa} onChange={(v) => set("charsFa", v)} />
              <CheckRow label={t("parser.charsEmoji")} checked={config.charsEmoji} onChange={(v) => set("charsEmoji", v)} />
            </div>
            <p className="text-xs text-muted-foreground mt-2">
              {t("parser.alphabetNoneHint")}
            </p>
          </Section>
          )}

          {(config.mode === "messages" || config.mode === "comments") && (
          <Section title={config.mode === "comments" ? t("parser.commentsParams") : t("parser.messagesParams")}>
            <Field label={t("parser.depthDays")}>
              <div className="flex items-center gap-2">
                <input
                  type="number"
                  min={0}
                  max={3650}
                  value={config.parsingDays}
                  onChange={(e) => set("parsingDays", Math.max(0, Number(e.target.value)))}
                  className="w-28 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50"
                />
                <span className="text-xs text-muted-foreground">{t("parser.depthZero")}</span>
              </div>
            </Field>
            <p className="text-xs text-muted-foreground mt-2">
              {config.mode === "comments"
                ? t("parser.commentsDesc")
                : t("parser.messagesDesc")}
            </p>
          </Section>
          )}

          {config.mode !== "messages" && config.mode !== "comments" && (
          <Section title={t("parser.onlineTitle")}>
            <div className="grid grid-cols-2 gap-y-2 gap-x-6">
              <CheckRow label={t("parser.onlineRecent")} checked={config.parseRecent} onChange={(v) => set("parseRecent", v)} />
              <CheckRow label={t("parser.onlineWeek")} checked={config.parseWeek} onChange={(v) => set("parseWeek", v)} />
              <CheckRow label={t("parser.onlineMonth")} checked={config.parseMonth} onChange={(v) => set("parseMonth", v)} />
              <CheckRow label={t("parser.onlineLong")} checked={config.parseLong} onChange={(v) => set("parseLong", v)} />
              <CheckRow label={t("parser.onlineDeleted")} checked={config.parseDeleted} onChange={(v) => set("parseDeleted", v)} />
            </div>
          </Section>
          )}

          <Section title={t("parser.additionalTitle")}>
            <div className="grid grid-cols-2 gap-y-2 gap-x-6">
              <CheckRow label={t("parser.premiumOnlyLabel")} checked={config.premiumOnly} onChange={(v) => set("premiumOnly", v)} />
              <CheckRow label={t("parser.excludeNoUsername")} checked={config.excludeNoUsername} onChange={(v) => set("excludeNoUsername", v)} />
              {config.mode !== "messages" && config.mode !== "comments" && (
                <>
                  <CheckRow label={t("parser.parseAdmins")} checked={config.parseAdmins} onChange={(v) => set("parseAdmins", v)} />
                  <CheckRow label={t("parser.excludeAdmins")} checked={config.excludeAdmins} onChange={(v) => set("excludeAdmins", v)} />
                  <CheckRow label={t("parser.parseBots")} checked={config.parseBots} onChange={(v) => set("parseBots", v)} />
                </>
              )}
              <CheckRow label={t("parser.leaveAfterLabel")} checked={config.leaveAfter} onChange={(v) => set("leaveAfter", v)} />
            </div>
          </Section>

          <Section title={t("parser.saveTitle")}>
            <div>
              <label className="block text-sm font-medium text-foreground mb-1">{t("parser.saveFileLabel")}</label>
              <p className="text-xs text-muted-foreground mb-1.5">{t("parser.saveFileHint")}</p>
              <div className="flex items-center gap-2">
                <button onClick={selectOutputFile} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition">
                  <Play className="h-3.5 w-3.5" /> {t("parser.selectBtn")}
                </button>
                <span className="text-xs text-muted-foreground truncate max-w-sm">{config.outputPath ? config.outputPath.split(/[/\\]/).pop() : t("parser.saveFileDefault")}</span>
              </div>
            </div>
          </Section>

          <Section title={t("parser.tempoTitle")}>
            <Field label={t("parser.floodWaitLabel")}>
              <div className="flex items-center gap-2">
                <input
                  type="number"
                  min={0}
                  max={86400}
                  value={config.maxFloodWait}
                  onChange={(e) => set("maxFloodWait", Math.max(0, Math.min(86400, Number(e.target.value))))}
                  className="w-28 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50"
                />
                <span className="text-xs text-muted-foreground">{t("parser.floodWaitZero")}</span>
              </div>
            </Field>
          </Section>

        </div>
      </div>

      {/* controls */}
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-6 text-sm font-semibold">
          <span>{t("parser.collected")}: <span className="text-[oklch(0.65_0.1_150)]">{stats.collected}</span></span>
          {stats.groupsTotal > 0 && <span>{t("parser.groups")}: <span className="text-muted-foreground">{stats.groupsDone}/{stats.groupsTotal}</span></span>}
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

      <AccountPickerModal
        open={pickerOpen}
        onClose={() => setPickerOpen(false)}
        onSelect={handleAccountSelected}
        title={accountPickerTitle}
      />
      </>
      )}
    </div>
  );
}

function serializeConfig(c: ParserConfig) {
  const mode = c.mode === "channel-admin" ? "channel_admin" : c.mode;
  const targets = c.targets.split("\n").map(t => t.trim()).filter(Boolean);
  return {
    mode,
    targets,

    parse_deleted: c.parseDeleted,
    parse_recent: c.parseRecent,
    parse_week: c.parseWeek,
    parse_month: c.parseMonth,
    parse_long: c.parseLong,

    premium_only: c.premiumOnly,
    parse_admins: c.parseAdmins,
    parse_bots: c.parseBots,
    exclude_admins: c.excludeAdmins,
    exclude_no_username: c.excludeNoUsername,

    chars_en: c.charsEn,
    chars_ru: c.charsRu,
    chars_cn: c.charsCn,
    chars_ar: c.charsAr,
    chars_he: c.charsHe,
    chars_fa: c.charsFa,
    chars_emoji: c.charsEmoji,

    parsing_days: c.parsingDays,

    leave_after: c.leaveAfter,

    output_path: c.outputPath.trim(),
    create_txt: c.createTxt,
    max_flood_wait: c.maxFloodWait,
  };
}

function ModeTab({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      onClick={onClick}
      className={`rounded-md border px-4 py-2 text-sm font-medium transition ${
        active
          ? "border-primary/50 bg-primary/10 text-primary"
          : "border-border bg-background text-muted-foreground hover:border-primary/30"
      }`}
    >
      {children}
    </button>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <h3 className="text-sm font-semibold text-foreground border-b border-border pb-1.5 mb-3">{title}</h3>
      <div className="space-y-3">{children}</div>
    </div>
  );
}

function Field({ label, indent, children }: { label: string; indent?: boolean; children: React.ReactNode }) {
  return (
    <div className={indent ? "ml-7" : ""}>
      <label className="block text-sm font-medium text-foreground mb-1">{label}</label>
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
      <span className="text-foreground font-medium">{label}</span>
    </label>
  );
}
