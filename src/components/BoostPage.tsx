import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Play, Square } from "lucide-react";
import { AccountPickerModal } from "@/components/AccountPickerModal";
import { ThreadInput } from "@/components/ThreadInput";
import { useT } from "@/i18n";
import { isDone, isError, isBoostDone } from "@/lib/eventParser";

type BoostMode =
  | "bot"
  | "views"
  | "reactions"
  | "subscribe-channel"
  | "subscribe-group"
  | "import-folder";

type EmojiMode = "random_positive" | "random_negative" | "specific" | "custom_list";
type PostTargetMode = "specific" | "last_n" | "all" | "pin";

// telegram standard reactions, split by sentiment
const POSITIVE_EMOJIS = ["👍", "❤️", "🔥", "🥰", "👏", "🎉", "🤩", "💯", "⚡", "🆒", "🏆", "😍", "🤝", "🙏"];
const NEGATIVE_EMOJIS = ["👎", "😢", "💩", "🤮", "😱", "🤬", "💔", "🥱", "🤡", "🤨"];
const ALL_EMOJIS = [...POSITIVE_EMOJIS, ...NEGATIVE_EMOJIS, "😁", "🤔", "🤯", "👌", "😐", "🥴", "😘"];

interface BoostConfig {
  mode: BoostMode;

  // bot activation
  botLink: string;
  botLinks: string;
  botRefParam: string;
  botUseReferral: boolean;
  botDeleteAfter: boolean;
  botDistributeMode: "all" | "unique";
  botMaxPerAccount: number;

  // views (also used by reactions)
  viewsPostLink: string;
  viewsIsPrivate: boolean;
  viewsJoinLink: string;
  viewsLeaveAfter: boolean;
  viewsArchiveAfter: boolean;

  // post target: specific link OR last N posts of a channel
  viewsPostTarget: PostTargetMode;
  viewsLastNMin: number;
  viewsLastNMax: number;

  // reactions (extends views)
  reactionsEmojiMode: EmojiMode;
  reactionsSpecificEmoji: string;
  reactionsEmojiList: string[];
  reactionsShuffle: boolean;
  reactionsPerPostMin: number;
  reactionsPerPostMax: number;
  reactionsDelayMin: number;
  reactionsDelayMax: number;
  reactionsPostLinks: string;
  reactionsViewAfterEach: boolean;
  reactionsAutoJoin: boolean;

  // subscribe to channels
  subChannelJoinLink: string;
  subChannelArchiveAfter: boolean;

  // subscribe to groups
  subGroupJoinLink: string;
  subGroupArchiveAfter: boolean;

  // import folders (addlist)
  importLinks: string;

  // shared
  threads: number;
  // 0 = unlimited; otherwise FLOOD_WAIT longer than this aborts the task for the account
  maxFloodWait: number;
}

const defaultConfig: BoostConfig = {
  mode: "bot",

  botLink: "",
  botLinks: "",
  botRefParam: "",
  botUseReferral: false,
  botDeleteAfter: false,
  botDistributeMode: "all",
  botMaxPerAccount: 0,

  viewsPostLink: "",
  viewsIsPrivate: false,
  viewsJoinLink: "",
  viewsLeaveAfter: false,
  viewsArchiveAfter: false,

  viewsPostTarget: "specific",
  viewsLastNMin: 5,
  viewsLastNMax: 10,

  reactionsEmojiMode: "random_positive",
  reactionsSpecificEmoji: "👍",
  reactionsEmojiList: [],
  reactionsShuffle: true,
  reactionsPerPostMin: 1,
  reactionsPerPostMax: 1,
  reactionsDelayMin: 1,
  reactionsDelayMax: 3,
  reactionsPostLinks: "",
  reactionsViewAfterEach: true,
  reactionsAutoJoin: true,

  subChannelJoinLink: "",
  subChannelArchiveAfter: false,

  subGroupJoinLink: "",
  subGroupArchiveAfter: false,

  importLinks: "",

  threads: 5,
  maxFloodWait: 60,
};

const STORAGE_KEY = "boost_config";
const IS_DEV = !("__TAURI_INTERNALS__" in window);

function loadSavedConfig(): BoostConfig {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return { ...defaultConfig, ...JSON.parse(saved) };
  } catch {}
  return defaultConfig;
}

export function BoostPage() {
  const t = useT();
  const [config, setConfig] = useState<BoostConfig>(loadSavedConfig);
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
    const unlisten = listen<string>("boost-log", (e) => {
      setLogs((prev) => [...prev, e.payload]);
      const msg = e.payload;
      if (isDone(msg)) {
        setRunning(false);
      } else if (isError(msg)) {
        setStats((s) => ({ ...s, errors: s.errors + 1 }));
      } else if (isBoostDone(msg)) {
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

  const set = <K extends keyof BoostConfig>(key: K, value: BoostConfig[K]) => {
    setConfig((prev) => ({ ...prev, [key]: value }));
  };

  const validate = (): string | null => {
    switch (config.mode) {
      case "bot":
        if (!config.botLink.trim() && !config.botLinks.trim()) return t("validation.noBoostBotLink");
        return null;
      case "views":
        if (!config.viewsPostLink.trim()) return t("validation.noBoostLink");
        if (config.viewsPostTarget === "last_n" && config.viewsLastNMin > config.viewsLastNMax) return t("validation.minGreaterThanMax");
        if (config.viewsPostTarget === "specific" && config.viewsIsPrivate && !config.viewsJoinLink.trim()) return t("validation.privateNeedsJoinLink");
        return null;
      case "reactions":
        if (!config.viewsPostLink.trim()) return t("validation.noBoostLink");
        if (config.viewsPostTarget === "last_n" && config.viewsLastNMin > config.viewsLastNMax) return t("validation.minGreaterThanMax");
        if (config.viewsPostTarget === "specific" && config.viewsIsPrivate && !config.viewsJoinLink.trim()) return t("validation.privateNeedsJoinLink");
        if (config.reactionsEmojiMode === "specific" && !config.reactionsSpecificEmoji.trim()) return t("validation.noEmoji");
        return null;
      case "subscribe-channel":
        if (!config.subChannelJoinLink.trim()) return t("validation.noChannelLink");
        return null;
      case "subscribe-group":
        if (!config.subGroupJoinLink.trim()) return t("validation.noGroupLink");
        return null;
      case "import-folder":
        if (!config.importLinks.trim()) return t("validation.noFolderLinks");
        return null;
    }
  };

  const handleStart = () => {
    const err = validate();
    if (err) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${err}`]);
      return;
    }
    setPickerOpen(true);
  };

  const handleAccountsSelected = async (ids: string[]) => {
    if (ids.length === 0) return;
    setLogs([]);
    setStats({ done: 0, errors: 0, inProgress: ids.length });
    setRunning(true);
    try {
      const tid = await invoke<string>("boost_start", {
        ids,
        config: serializeConfig(config),
        threads: config.threads,
        maxFloodWait: config.maxFloodWait,
      });
      setTaskId(tid);
    } catch (e: any) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${e}`]);
      setRunning(false);
    }
  };

  const handleStop = async () => {
    if (!IS_DEV && taskId) await invoke("boost_stop", { taskId }).catch(() => {});
    setRunning(false);
    setLogs((prev) => [...prev, t("common.stoppedByUser")]);
  };

  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">{t("descriptions.boost")}</p>
      {/* mode tabs */}
      <div className="flex gap-2 flex-wrap">
        <ModeTab active={config.mode === "bot"} onClick={() => set("mode", "bot")}>{t("boost.modeBotTab")}</ModeTab>
        <ModeTab active={config.mode === "views"} onClick={() => set("mode", "views")}>{t("boost.modeViewsTab")}</ModeTab>
        <ModeTab active={config.mode === "reactions"} onClick={() => set("mode", "reactions")}>{t("boost.modeReactionsTab")}</ModeTab>
        <ModeTab active={config.mode === "subscribe-channel"} onClick={() => set("mode", "subscribe-channel")}>{t("boost.modeSubChannelTab")}</ModeTab>
        <ModeTab active={config.mode === "subscribe-group"} onClick={() => set("mode", "subscribe-group")}>{t("boost.modeSubGroupTab")}</ModeTab>
        <ModeTab active={config.mode === "import-folder"} onClick={() => set("mode", "import-folder")}>{t("boost.modeImportTab")}</ModeTab>
      </div>

      {/* config panel */}
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div className="p-6 space-y-5">

          {config.mode === "bot" && (
            <>
              <Field label={t("boost.botListLabel")}>
                <textarea
                  value={config.botLinks}
                  onChange={(e) => set("botLinks", e.target.value)}
                  placeholder={"@bot1\nhttps://t.me/bot2?start=ref\n@bot3"}
                  rows={4}
                  className="mt-1.5 w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50 resize-y"
                />
                <p className="mt-1 text-xs text-muted-foreground">{t("boost.botListHint")}</p>
              </Field>
              <div className="flex items-center gap-2 mt-2">
                <ModeTab active={config.botDistributeMode === "all"} onClick={() => set("botDistributeMode", "all")}>{t("boost.botAllToAll")}</ModeTab>
                <ModeTab active={config.botDistributeMode === "unique"} onClick={() => set("botDistributeMode", "unique")}>{t("boost.botUniqueDistrib")}</ModeTab>
              </div>
              <p className="text-xs text-muted-foreground">{t("boost.botAllToAllDesc")}</p>
              {config.botDistributeMode === "unique" && (
                <Field label={t("boost.botMaxLabel")}>
                  <input type="number" min={0} max={9999} value={config.botMaxPerAccount} onChange={(e) => set("botMaxPerAccount", Math.max(0, Number(e.target.value)))} className="mt-1.5 w-24 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
                  <p className="mt-1 text-xs text-muted-foreground">{t("boost.botMaxZero")}</p>
                </Field>
              )}
              <CheckRow label={t("boost.botDeleteLabel")} checked={config.botDeleteAfter} onChange={(v) => set("botDeleteAfter", v)} />
            </>
          )}

          {(config.mode === "views" || config.mode === "reactions") && (
            <>
              <div>
                <label className="text-sm font-medium text-foreground">{t("boost.targetLabel")}</label>
                <div className="flex items-center gap-2 mt-1.5 flex-wrap">
                  <ModeButton active={config.viewsPostTarget === "specific"} onClick={() => set("viewsPostTarget", "specific")}>{t("boost.targetSpecificBtn")}</ModeButton>
                  <ModeButton active={config.viewsPostTarget === "last_n"} onClick={() => set("viewsPostTarget", "last_n")}>{t("boost.targetLastNBtn")}</ModeButton>
                  <ModeButton active={config.viewsPostTarget === "all"} onClick={() => set("viewsPostTarget", "all")}>{t("boost.targetAllBtn")}</ModeButton>
                  <ModeButton active={config.viewsPostTarget === "pin"} onClick={() => set("viewsPostTarget", "pin")}>{t("boost.targetPinnedBtn")}</ModeButton>
                </div>
              </div>

              {config.viewsPostTarget === "specific" && (
                <Field label={t("boost.postLinkLabel")}>
                  <input
                    value={config.viewsPostLink}
                    onChange={(e) => set("viewsPostLink", e.target.value)}
                    placeholder="https://t.me/channel/123"
                    className="mt-1.5 w-full max-w-md rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50"
                  />
                </Field>
              )}

              {config.viewsPostTarget === "last_n" && (
                <>
                  <Field label={t("boost.channelLinkLabel")}>
                    <input
                      value={config.viewsPostLink}
                      onChange={(e) => set("viewsPostLink", e.target.value)}
                      placeholder="@channel / https://t.me/channel"
                      className="mt-1.5 w-full max-w-md rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50"
                    />
                  </Field>
                  <Field label={t("boost.lastNLabel")}>
                    <div className="flex items-center gap-2 mt-1.5">
                      <input
                        type="number"
                        min={1}
                        max={100}
                        value={config.viewsLastNMin}
                        onChange={(e) => set("viewsLastNMin", Math.max(1, Math.min(100, Number(e.target.value))))}
                        className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50"
                      />
                      <span className="text-xs text-muted-foreground">—</span>
                      <input
                        type="number"
                        min={1}
                        max={100}
                        value={config.viewsLastNMax}
                        onChange={(e) => set("viewsLastNMax", Math.max(1, Math.min(100, Number(e.target.value))))}
                        className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50"
                      />
                      <span className="text-xs text-muted-foreground">{t("boost.lastNHint")}</span>
                    </div>
                  </Field>
                </>
              )}

              {(config.viewsPostTarget === "all" || config.viewsPostTarget === "pin") && (
                <Field label={t("boost.channelLinkLabel")}>
                  <input
                    value={config.viewsPostLink}
                    onChange={(e) => set("viewsPostLink", e.target.value)}
                    placeholder="@channel / https://t.me/channel"
                    className="mt-1.5 w-full max-w-md rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50"
                  />
                  <p className="text-xs text-muted-foreground mt-1">
                    {config.viewsPostTarget === "all" ? t("boost.viewAllHint") : t("boost.viewPinnedHint")}
                  </p>
                </Field>
              )}

              {config.viewsPostTarget === "specific" && (
                <>
                  <CheckRow label={t("boost.privateLabel")} checked={config.viewsIsPrivate} onChange={(v) => set("viewsIsPrivate", v)} />
                  {config.viewsIsPrivate && (
                    <Field label={t("boost.joinLinkLabel")} indent>
                      <input
                        value={config.viewsJoinLink}
                        onChange={(e) => set("viewsJoinLink", e.target.value)}
                        placeholder="https://t.me/+abcXYZ..."
                        className="mt-1.5 w-full max-w-md rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50"
                      />
                    </Field>
                  )}
                </>
              )}

              {config.mode === "reactions" && (
                <>
                <div>
                  <label className="text-sm font-medium text-foreground">{t("boost.emojiLabel")}</label>
                  <div className="flex items-center gap-2 mt-1.5 flex-wrap">
                    <ModeButton active={config.reactionsEmojiMode === "random_positive"} onClick={() => set("reactionsEmojiMode", "random_positive")}>{t("boost.emojiRandomPositiveBtn")}</ModeButton>
                    <ModeButton active={config.reactionsEmojiMode === "random_negative"} onClick={() => set("reactionsEmojiMode", "random_negative")}>{t("boost.emojiRandomNegativeBtn")}</ModeButton>
                    <ModeButton active={config.reactionsEmojiMode === "specific"} onClick={() => set("reactionsEmojiMode", "specific")}>{t("boost.emojiSpecificBtn")}</ModeButton>
                    <ModeButton active={config.reactionsEmojiMode === "custom_list"} onClick={() => set("reactionsEmojiMode", "custom_list")}>{t("boost.emojiCustomListBtn")}</ModeButton>
                  </div>

                  {config.reactionsEmojiMode === "random_positive" && (
                    <div className="mt-2">
                      <p className="text-xs text-muted-foreground mb-1">{t("boost.emojiPositivePool")}</p>
                      <div className="flex flex-wrap gap-1.5">
                        {POSITIVE_EMOJIS.map((e) => (
                          <span key={e} className="text-lg" aria-hidden>{e}</span>
                        ))}
                      </div>
                    </div>
                  )}
                  {config.reactionsEmojiMode === "random_negative" && (
                    <div className="mt-2">
                      <p className="text-xs text-muted-foreground mb-1">{t("boost.emojiNegativePool")}</p>
                      <div className="flex flex-wrap gap-1.5">
                        {NEGATIVE_EMOJIS.map((e) => (
                          <span key={e} className="text-lg" aria-hidden>{e}</span>
                        ))}
                      </div>
                    </div>
                  )}
                  {config.reactionsEmojiMode === "specific" && (
                    <div className="mt-2 space-y-2">
                      <div className="flex items-center gap-2">
                        <input
                          value={config.reactionsSpecificEmoji}
                          onChange={(e) => set("reactionsSpecificEmoji", e.target.value)}
                          placeholder="👍"
                          maxLength={4}
                          className="w-20 rounded-md border border-border bg-background px-3 py-2 text-center text-base outline-none focus:border-primary/50"
                        />
                        <span className="text-xs text-muted-foreground">{t("boost.emojiSelectHint")}</span>
                      </div>
                      <div className="flex flex-wrap gap-1">
                        {ALL_EMOJIS.map((e) => (
                          <button
                            key={e}
                            type="button"
                            onClick={() => set("reactionsSpecificEmoji", e)}
                            className={`text-lg p-1 rounded transition ${config.reactionsSpecificEmoji === e ? "bg-primary/20 ring-1 ring-primary/50" : "hover:bg-accent"}`}
                            title={e}
                          >
                            {e}
                          </button>
                        ))}
                      </div>
                    </div>
                  )}
                  {config.reactionsEmojiMode === "custom_list" && (
                    <div className="mt-2 space-y-2">
                      <p className="text-xs text-muted-foreground">{t("boost.emojiCustomHint")}</p>
                      <div className="flex flex-wrap gap-1">
                        {ALL_EMOJIS.map((e) => (
                          <button
                            key={e}
                            type="button"
                            onClick={() => {
                              const list = [...config.reactionsEmojiList];
                              const idx = list.indexOf(e);
                              if (idx >= 0) list.splice(idx, 1); else list.push(e);
                              set("reactionsEmojiList", list);
                            }}
                            className={`text-lg p-1 rounded transition ${config.reactionsEmojiList.includes(e) ? "bg-primary/20 ring-1 ring-primary/50" : "hover:bg-accent opacity-50"}`}
                          >
                            {e}
                          </button>
                        ))}
                      </div>
                      {config.reactionsEmojiList.length > 0 && (
                        <p className="text-xs text-muted-foreground">{t("boost.emojiSelected")} {config.reactionsEmojiList.join(" ")} ({config.reactionsEmojiList.length})</p>
                      )}
                    </div>
                  )}
                </div>

                {/* Post links */}
                <div>
                  <label className="text-sm font-medium text-foreground">{t("boost.postLinksLabel")}</label>
                  <textarea
                    value={config.reactionsPostLinks}
                    onChange={(e) => set("reactionsPostLinks", e.target.value)}
                    placeholder="https://t.me/channel/1&#10;https://t.me/channel2/5&#10;(по одной на строку)"
                    rows={3}
                    className="mt-1.5 w-full max-w-md rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50 resize-y"
                  />
                  <p className="text-xs text-muted-foreground mt-1">{t("boost.postLinksHint")}</p>
                </div>

                {/* Reactions per post */}
                <div>
                  <label className="text-sm font-medium text-foreground">{t("boost.reactionsPerPost")}</label>
                  <div className="flex items-center gap-2 mt-1.5">
                    <input type="number" min={1} max={50} value={config.reactionsPerPostMin}
                      onChange={(e) => set("reactionsPerPostMin", Math.max(1, Math.min(50, Number(e.target.value))))}
                      className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
                    <span className="text-xs text-muted-foreground">—</span>
                    <input type="number" min={1} max={50} value={config.reactionsPerPostMax}
                      onChange={(e) => set("reactionsPerPostMax", Math.max(1, Math.min(50, Number(e.target.value))))}
                      className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
                    <span className="text-xs text-muted-foreground">{t("boost.reactionsPerPostHint")}</span>
                  </div>
                </div>

                {/* Delay between reactions */}
                <div>
                  <label className="text-sm font-medium text-foreground">{t("boost.reactionsDelay")}</label>
                  <div className="flex items-center gap-2 mt-1.5">
                    <input type="number" min={0} max={60} value={config.reactionsDelayMin}
                      onChange={(e) => set("reactionsDelayMin", Math.max(0, Number(e.target.value)))}
                      className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
                    <span className="text-xs text-muted-foreground">—</span>
                    <input type="number" min={0} max={60} value={config.reactionsDelayMax}
                      onChange={(e) => set("reactionsDelayMax", Math.max(0, Number(e.target.value)))}
                      className="w-20 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
                  </div>
                </div>

                {/* Options */}
                <div className="space-y-2">
                  <CheckRow label={t("boost.shuffleEmoji")} checked={config.reactionsShuffle} onChange={(v) => set("reactionsShuffle", v)} />
                  <CheckRow label={t("boost.viewAfterEach")} checked={config.reactionsViewAfterEach} onChange={(v) => set("reactionsViewAfterEach", v)} />
                  <CheckRow label={t("boost.autoJoinChannel")} checked={config.reactionsAutoJoin} onChange={(v) => set("reactionsAutoJoin", v)} />
                </div>
                </>
              )}
            </>
          )}

          {config.mode === "subscribe-channel" && (
            <>
              <Field label={t("boost.subChannelLabel")}>
                <input
                  value={config.subChannelJoinLink}
                  onChange={(e) => set("subChannelJoinLink", e.target.value)}
                  placeholder="@channel / https://t.me/channel / t.me/+invite"
                  className="mt-1.5 w-full max-w-md rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50"
                />
              </Field>
              <CheckRow label={t("boost.subArchiveLabel")} checked={config.subChannelArchiveAfter} onChange={(v) => set("subChannelArchiveAfter", v)} />
            </>
          )}

          {config.mode === "subscribe-group" && (
            <>
              <Field label={t("boost.subGroupLabel")}>
                <input
                  value={config.subGroupJoinLink}
                  onChange={(e) => set("subGroupJoinLink", e.target.value)}
                  placeholder="@group / https://t.me/group / t.me/+invite"
                  className="mt-1.5 w-full max-w-md rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50"
                />
              </Field>
              <CheckRow label={t("boost.subArchiveLabel")} checked={config.subGroupArchiveAfter} onChange={(v) => set("subGroupArchiveAfter", v)} />
            </>
          )}

          {config.mode === "import-folder" && (
            <Field label={t("boost.importLabel")}>
              <textarea
                value={config.importLinks}
                onChange={(e) => set("importLinks", e.target.value)}
                placeholder="https://t.me/addlist/zZ5Qduyc6iliM2E0&#10;https://t.me/addlist/anotherSlug"
                rows={6}
                className="mt-1.5 w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50 font-mono resize-y block"
              />
              <p className="mt-1 text-xs text-muted-foreground">{t("boost.importHint")}</p>
            </Field>
          )}

          {/* shared: flood wait limit */}
          <div className="border-t border-border pt-4">
            <Field label={t("boost.floodWaitLabel")}>
              <div className="flex items-center gap-2 mt-1.5">
                <input
                  type="number"
                  min={0}
                  max={86400}
                  value={config.maxFloodWait}
                  onChange={(e) => set("maxFloodWait", Math.max(0, Math.min(86400, Number(e.target.value))))}
                  className="w-28 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50"
                />
                <span className="text-xs text-muted-foreground">{t("boost.floodWaitZero")}</span>
              </div>
            </Field>
          </div>

        </div>
      </div>

      {/* controls */}
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-6 text-sm font-semibold">
          <span>{t("boost.completed")}: <span className="text-[oklch(0.65_0.1_150)]">{stats.done}</span></span>
          <span>{t("common.errors")}: <span className="text-[oklch(0.55_0.1_25)]">{stats.errors}</span></span>
          <span>{t("boost.remaining")}: <span className="text-[oklch(0.65_0.1_280)]">{Math.max(0, stats.inProgress - stats.done - stats.errors)}</span></span>
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

function serializeConfig(c: BoostConfig) {
  // shape payload by active mode so backend receives only what it needs
  const base: Record<string, unknown> = { mode: c.mode };
  switch (c.mode) {
    case "bot":
      return {
        ...base,
        bot_link: c.botLink.trim(),
        bot_links: c.botLinks.split("\n").map(l => l.trim()).filter(Boolean),
        use_referral: true,
        ref_param: c.botRefParam.trim(),
        delete_after: c.botDeleteAfter,
        distribute_mode: c.botDistributeMode,
        max_per_account: c.botMaxPerAccount,
      };
    case "views":
      return {
        ...base,
        post_link: c.viewsPostLink.trim(),
        is_private: c.viewsIsPrivate,
        join_link: c.viewsIsPrivate ? c.viewsJoinLink.trim() : "",
        leave_after: c.viewsLeaveAfter,
        archive_after: c.viewsArchiveAfter,
        post_target: c.viewsPostTarget,
        last_n_min: c.viewsLastNMin,
        last_n_max: c.viewsLastNMax,
      };
    case "reactions":
      return {
        ...base,
        post_link: c.viewsPostLink.trim(),
        post_links: c.reactionsPostLinks.split(/[,\n]/).map((l) => l.trim()).filter(Boolean),
        is_private: c.viewsIsPrivate,
        join_link: c.viewsIsPrivate ? c.viewsJoinLink.trim() : "",
        leave_after: c.viewsLeaveAfter,
        archive_after: c.viewsArchiveAfter,
        emoji_mode: c.reactionsEmojiMode,
        specific_emoji: c.reactionsEmojiMode === "specific" ? c.reactionsSpecificEmoji : "",
        emoji_list: c.reactionsEmojiMode === "custom_list" ? c.reactionsEmojiList : [],
        reactions_shuffle: c.reactionsShuffle,
        reactions_per_post_min: c.reactionsPerPostMin,
        reactions_per_post_max: c.reactionsPerPostMax,
        reactions_delay_min: c.reactionsDelayMin,
        reactions_delay_max: c.reactionsDelayMax,
        post_target: c.viewsPostTarget,
        last_n_min: c.viewsLastNMin,
        last_n_max: c.viewsLastNMax,
        view_after_each: c.reactionsViewAfterEach,
        auto_join: c.reactionsAutoJoin,
      };
    case "subscribe-channel":
      return {
        ...base,
        join_link: c.subChannelJoinLink.trim(),
        archive_after: c.subChannelArchiveAfter,
      };
    case "subscribe-group":
      return {
        ...base,
        join_link: c.subGroupJoinLink.trim(),
        archive_after: c.subGroupArchiveAfter,
      };
    case "import-folder":
      return {
        ...base,
        links: c.importLinks
          .split(/\r?\n/)
          .map((s) => s.trim())
          .filter(Boolean),
      };
  }
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

function ModeButton({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button onClick={onClick} className={`rounded-md border px-3 py-1 text-xs font-medium transition ${active ? "border-primary/50 bg-primary/10 text-primary" : "border-border bg-background text-muted-foreground hover:border-primary/30"}`}>
      {children}
    </button>
  );
}

function Field({ label, indent, children }: { label: string; indent?: boolean; children: React.ReactNode }) {
  return (
    <div className={indent ? "ml-7" : ""}>
      <label className="text-sm font-medium text-foreground">{label}</label>
      {children}
    </div>
  );
}

function CheckRow({ label, checked, onChange }: { label: string; checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <label className="flex items-center gap-3 cursor-pointer">
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        className="rounded border-border accent-primary h-4 w-4"
      />
      <span className="text-sm font-medium text-foreground">{label}</span>
    </label>
  );
}
