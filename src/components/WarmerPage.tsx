import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Play, Square } from "lucide-react";
import { AccountPickerModal } from "@/components/AccountPickerModal";
import { useT } from "@/i18n";

interface WarmerConfig {
  doAll: boolean;
  searchRandomWords: boolean;
  readRandomChannels: boolean;
  readChannelsReact: boolean;
  subscribeChannels: boolean;
  fakeChats: boolean;
  fakeChatsUseLlm: boolean;
  writeSavedMessages: boolean;
  restBetweenActions: boolean;
  browseGroupMembers: boolean;
  browseGroupAvatars: boolean;
  browseGroupAddContacts: boolean;
  readDialogs: boolean;
  viewStories: boolean;
  addContactsFromSearch: boolean;
  cleanupAfter: boolean;
  appendEmoji: boolean;
  durationMinutes: number; // 0 = infinite
  maxFloodWait: number; // max seconds to wait on FLOOD_WAIT, 0 = unlimited
}

const defaultConfig: WarmerConfig = {
  doAll: true,
  searchRandomWords: true,
  readRandomChannels: true,
  readChannelsReact: true,
  subscribeChannels: true,
  fakeChats: true,
  fakeChatsUseLlm: false,
  writeSavedMessages: true,
  restBetweenActions: true,
  browseGroupMembers: true,
  browseGroupAvatars: true,
  browseGroupAddContacts: true,
  readDialogs: true,
  viewStories: true,
  addContactsFromSearch: true,
  cleanupAfter: true,
  appendEmoji: true,
  durationMinutes: 0,
  maxFloodWait: 60,
};

const STORAGE_KEY = "warmer_config";
const IS_DEV = !("__TAURI_INTERNALS__" in window);

function loadSavedConfig(): WarmerConfig {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return { ...defaultConfig, ...JSON.parse(saved) };
  } catch {}
  return defaultConfig;
}

export function WarmerPage() {
  const t = useT();
  const [config, setConfig] = useState<WarmerConfig>(loadSavedConfig);
  const [running, setRunning] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [stats, setStats] = useState({ actions: 0, errors: 0 });
  const [pickerOpen, setPickerOpen] = useState(false);
  const logsRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(config));
  }, [config]);

  useEffect(() => {
    if (IS_DEV) return;
    const unlisten = listen<string>("warmer-log", (e) => {
      setLogs((prev) => [...prev, e.payload]);
      const msg = e.payload;
      if (msg === "Завершено" || msg === "Done") {
        setRunning(false);
      } else if (msg.startsWith("[action]")) {
        setStats((s) => ({ ...s, actions: s.actions + 1 }));
      } else if (msg.includes("ОШИБКА") || msg.includes("ошибка") || msg.includes("ERROR") || msg.includes("error")) {
        setStats((s) => ({ ...s, errors: s.errors + 1 }));
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

  const set = <K extends keyof WarmerConfig>(key: K, value: WarmerConfig[K]) => {
    setConfig((prev) => {
      const next = { ...prev, [key]: value };
      if (key === "doAll" && value === true) {
        // "do all" enables all actions but preserves LLM toggle
        const preserved = { fakeChatsUseLlm: prev.fakeChatsUseLlm, appendEmoji: prev.appendEmoji };
        return { ...defaultConfig, doAll: true, ...preserved };
      }
      if (key !== "doAll" && key !== "fakeChatsUseLlm" && key !== "appendEmoji" && prev.doAll) {
        next.doAll = false;
      }
      return next;
    });
  };

  const handleStart = () => {
    setPickerOpen(true);
  };

  const handleAccountSelected = async (ids: string[]) => {
    if (ids.length === 0) return;
    setLogs([]);
    setStats({ actions: 0, errors: 0 });
    setRunning(true);
    try {
      const tid = await invoke<string>("warmer_start", {
        ids,
        config: serializeConfig(config),
      });
      setTaskId(tid);
    } catch (e: any) {
      setLogs((prev) => [...prev, `${t("common.error")}: ${e}`]);
      setRunning(false);
    }
  };

  const handleStop = async () => {
    if (!IS_DEV && taskId) await invoke("warmer_stop", { taskId }).catch(() => {});
    setRunning(false);
    setLogs((prev) => [...prev, t("common.stoppedByUser")]);
  };

  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">{t("descriptions.warmer")}</p>
      {/* config panel */}
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div className="p-6 space-y-4">

          <CheckRow
            label={t("warmer.doAll")}
            checked={config.doAll}
            onChange={(v) => set("doAll", v)}
            bold
          />

          <div className="border-t border-border pt-3 space-y-2">
            <CheckRow label={t("warmer.searchRandomWords")} checked={config.searchRandomWords} onChange={(v) => set("searchRandomWords", v)} />
            <CheckRow label={t("warmer.readRandomChannels")} checked={config.readRandomChannels} onChange={(v) => set("readRandomChannels", v)} />
            <SubCheck label={t("warmer.readChannelsReact")} checked={config.readChannelsReact} onChange={(v) => set("readChannelsReact", v)} parent={config.readRandomChannels} />
            <CheckRow label={t("warmer.subscribeChannels")} checked={config.subscribeChannels} onChange={(v) => set("subscribeChannels", v)} />
            <CheckRow label={t("warmer.fakeChats")} checked={config.fakeChats} onChange={(v) => set("fakeChats", v)} />
            <SubCheck label={t("warmer.fakeChatsUseLlm")} checked={config.fakeChatsUseLlm} onChange={(v) => set("fakeChatsUseLlm", v)} parent={config.fakeChats} />
            {config.fakeChats && config.fakeChatsUseLlm && (
              <p className="ml-7 text-xs font-medium text-[oklch(0.6_0.15_60)]">
                {t("warmer.llmWarning")}
              </p>
            )}
            <SubCheck label={t("warmer.appendEmoji")} checked={config.appendEmoji} onChange={(v) => set("appendEmoji", v)} parent={config.fakeChats} />
            <CheckRow label={t("warmer.writeSavedMessages")} checked={config.writeSavedMessages} onChange={(v) => set("writeSavedMessages", v)} />
            <CheckRow label={t("warmer.restBetweenActions")} checked={config.restBetweenActions} onChange={(v) => set("restBetweenActions", v)} />
            <CheckRow label={t("warmer.browseGroupMembers")} checked={config.browseGroupMembers} onChange={(v) => set("browseGroupMembers", v)} />
            <SubCheck label={t("warmer.browseGroupAvatars")} checked={config.browseGroupAvatars} onChange={(v) => set("browseGroupAvatars", v)} parent={config.browseGroupMembers} />
            <SubCheck label={t("warmer.browseGroupAddContacts")} checked={config.browseGroupAddContacts} onChange={(v) => set("browseGroupAddContacts", v)} parent={config.browseGroupMembers} />
            <CheckRow label={t("warmer.readDialogs")} checked={config.readDialogs} onChange={(v) => set("readDialogs", v)} />
            <CheckRow label={t("warmer.viewStories")} checked={config.viewStories} onChange={(v) => set("viewStories", v)} />
            <CheckRow label={t("warmer.cleanupAfter")} checked={config.cleanupAfter} onChange={(v) => set("cleanupAfter", v)} />
          </div>

          <div className="border-t border-border pt-3 space-y-2">
            <label className="flex items-center gap-2.5 text-sm">
              <span className="text-foreground">{t("warmer.duration")}:</span>
              <input
                type="number"
                min={0}
                value={config.durationMinutes}
                onChange={(e) => set("durationMinutes", Math.max(0, parseInt(e.target.value) || 0))}
                className="w-20 rounded border border-border bg-background px-2 py-1 text-sm"
              />
            </label>
            <label className="flex items-center gap-2.5 text-sm">
              <span className="text-foreground">{t("common.maxFloodWait")}:</span>
              <input
                type="number"
                min={0}
                value={config.maxFloodWait}
                onChange={(e) => set("maxFloodWait", Math.max(0, parseInt(e.target.value) || 0))}
                className="w-20 rounded border border-border bg-background px-2 py-1 text-sm"
              />
            </label>
          </div>

          <p className="text-xs text-muted-foreground pt-2">
            {t("warmer.description")}
          </p>

        </div>
      </div>

      {/* controls */}
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-6 text-sm font-semibold">
          <span>{t("common.actions")}: <span className="text-[oklch(0.65_0.1_150)]">{stats.actions}</span></span>
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

function serializeConfig(c: WarmerConfig) {
  return {
    do_all: c.doAll,
    search_random_words: c.searchRandomWords,
    read_random_channels: c.readRandomChannels,
    read_channels_react: c.readChannelsReact,
    subscribe_channels: c.subscribeChannels,
    fake_chats: c.fakeChats,
    fake_chats_use_llm: c.fakeChatsUseLlm,
    write_saved_messages: c.writeSavedMessages,
    rest_between_actions: c.restBetweenActions,
    browse_group_members: c.browseGroupMembers,
    browse_group_avatars: c.browseGroupAvatars,
    browse_group_add_contacts: c.browseGroupAddContacts,
    read_dialogs: c.readDialogs,
    view_stories: c.viewStories,
    add_contacts_from_search: c.addContactsFromSearch,
    cleanup_after: c.cleanupAfter,
    append_emoji: c.appendEmoji,
    duration_minutes: c.durationMinutes,
    max_flood_wait: c.maxFloodWait,
  };
}

function CheckRow({ label, checked, onChange, bold }: { label: string; checked: boolean; onChange: (v: boolean) => void; bold?: boolean }) {
  return (
    <label className="flex items-center gap-2.5 cursor-pointer text-sm">
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        className="rounded border-border accent-primary h-4 w-4"
      />
      <span className={`text-foreground ${bold ? "font-semibold" : ""}`}>{label}</span>
    </label>
  );
}

function SubCheck({ label, checked, onChange, parent }: { label: string; checked: boolean; onChange: (v: boolean) => void; parent: boolean }) {
  return (
    <label className={`flex items-center gap-2.5 cursor-pointer text-sm ml-7 ${!parent ? "opacity-40 pointer-events-none" : ""}`}>
      <input
        type="checkbox"
        checked={checked && parent}
        onChange={(e) => onChange(e.target.checked)}
        disabled={!parent}
        className="rounded border-border accent-primary h-4 w-4"
      />
      <span className="text-foreground">{label}</span>
    </label>
  );
}
