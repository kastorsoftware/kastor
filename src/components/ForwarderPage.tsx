import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Play, Square } from "lucide-react";
import { AccountPickerModal } from "@/components/AccountPickerModal";
import { useT } from "@/i18n";

interface Config {
  groupLink: string;
  maxFloodWait: number;
  typingMin: number;
  typingMax: number;
  messWaitMin: number;
  messWaitMax: number;
  sendWaitMin: number;
  sendWaitMax: number;
  resendOld: boolean;
  resendOldWaitMin: number;
  resendOldWaitMax: number;
  leaveOnStop: boolean;
  sendReaction: boolean;
}

const IS_DEV = !("__TAURI_INTERNALS__" in window);
const STORAGE_KEY = "forwarder_config";

const defaultConfig: Config = {
  groupLink: "",
  maxFloodWait: 60,
  typingMin: 3,
  typingMax: 8,
  messWaitMin: 1,
  messWaitMax: 3,
  sendWaitMin: 2,
  sendWaitMax: 5,
  resendOld: false,
  resendOldWaitMin: 3,
  resendOldWaitMax: 10,
  leaveOnStop: false,
  sendReaction: true,
};

function loadSavedConfig(): Config {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return { ...defaultConfig, ...JSON.parse(saved) };
  } catch {}
  return { ...defaultConfig };
}

export function ForwarderPage() {
  const t = useT();
  const [config, setConfig] = useState<Config>(loadSavedConfig());
  const [running, setRunning] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [stats, setStats] = useState({ forwarded: 0, replied: 0 });
  const [pickerOpen, setPickerOpen] = useState(false);
  const logsRef = useRef<HTMLDivElement>(null);

  useEffect(() => { localStorage.setItem(STORAGE_KEY, JSON.stringify(config)); }, [config]);

  useEffect(() => {
    if (IS_DEV) return;
    const unlisten = listen<string>("forwarder-log", (e) => {
      setLogs((prev) => [...prev, e.payload]);
      const msg = e.payload;
      if (msg === "Завершено" || msg === "Done") setRunning(false);
      else if (msg.includes("переслано") || msg.includes("forwarded")) setStats((s) => ({ ...s, forwarded: s.forwarded + 1 }));
      else if (msg.includes("скопирован") || msg.includes("copied")) setStats((s) => ({ ...s, replied: s.replied + 1 }));
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

  const handleStart = () => {
    if (!config.groupLink.trim()) { setLogs((p) => [...p, `${t("common.error")}: ${t("forwarder.errNoGroup")}`]); return; }
    setPickerOpen(true);
  };

  const handleAccountsSelected = async (ids: string[]) => {
    if (ids.length === 0) return;
    setLogs([]); setStats({ forwarded: 0, replied: 0 }); setRunning(true);
    try {
      const tid = await invoke<string>("forwarder_start", {
        ids,
        config: {
          group_link: config.groupLink.trim(),
          max_flood_wait: config.maxFloodWait,
          typing_min: config.typingMin,
          typing_max: config.typingMax,
          mess_wait_min: config.messWaitMin,
          mess_wait_max: config.messWaitMax,
          send_wait_min: config.sendWaitMin,
          send_wait_max: config.sendWaitMax,
          resend_old: config.resendOld,
          resend_old_wait_min: config.resendOldWaitMin,
          resend_old_wait_max: config.resendOldWaitMax,
          leave_on_stop: config.leaveOnStop,
          send_reaction: config.sendReaction,
        },
      });
      setTaskId(tid);
    } catch (e: any) { setLogs((p) => [...p, `${t("common.error")}: ${e}`]); setRunning(false); }
  };

  const handleStop = async () => {
    if (!IS_DEV && taskId) await invoke("forwarder_stop", { taskId }).catch(() => {});
    setRunning(false); setLogs((p) => [...p, t("common.stoppedByUser")]);
  };

  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">{t("descriptions.forwarder")}</p>
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div className="p-6 space-y-5">

          <div>
            <label className="text-sm font-medium text-foreground">{t("forwarder.groupLink")}</label>
            <input value={config.groupLink} onChange={(e) => set("groupLink", e.target.value)}
              placeholder="@group, https://t.me/group, https://t.me/+invite"
              className="mt-1.5 w-full max-w-md rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50" />
            <p className="text-xs text-muted-foreground mt-1">{t("forwarder.groupLinkHint")}</p>
          </div>

          <Section title={t("forwarder.delaysTitle")}>
            <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
              <DelayRow label={t("forwarder.messageWait")} min={config.messWaitMin} max={config.messWaitMax} onMinChange={(v) => set("messWaitMin", v)} onMaxChange={(v) => set("messWaitMax", v)} />
              <DelayRow label={t("forwarder.sendWait")} min={config.sendWaitMin} max={config.sendWaitMax} onMinChange={(v) => set("sendWaitMin", v)} onMaxChange={(v) => set("sendWaitMax", v)} />
              <DelayRow label={t("forwarder.typingDelay")} min={config.typingMin} max={config.typingMax} onMinChange={(v) => set("typingMin", v)} onMaxChange={(v) => set("typingMax", v)} />
            </div>
          </Section>

          <Section title={t("forwarder.oldMessagesTitle")}>
            <CheckRow label={t("forwarder.resendOld")} checked={config.resendOld} onChange={(v) => set("resendOld", v)} />
            {config.resendOld && (
              <div className="ml-7 mt-2">
                <DelayRow label={t("forwarder.resendDelay")} min={config.resendOldWaitMin} max={config.resendOldWaitMax} onMinChange={(v) => set("resendOldWaitMin", v)} onMaxChange={(v) => set("resendOldWaitMax", v)} />
              </div>
            )}
          </Section>

          <Section title={t("forwarder.otherTitle")}>
            <div className="grid grid-cols-2 gap-y-2 gap-x-6">
              <CheckRow label={t("forwarder.sendReaction")} checked={config.sendReaction} onChange={(v) => set("sendReaction", v)} />
              <CheckRow label={t("forwarder.leaveOnStop")} checked={config.leaveOnStop} onChange={(v) => set("leaveOnStop", v)} />
            </div>
            <div className="mt-3">
              <label className="text-sm font-medium text-foreground">{t("common.maxFloodWait")}</label>
              <input type="number" min={0} value={config.maxFloodWait} onChange={(e) => set("maxFloodWait", Math.max(0, Number(e.target.value)))}
                className="mt-1 w-24 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary/50" />
            </div>
          </Section>

        </div>
      </div>

      {/* controls */}
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-6 text-sm font-semibold">
          <span>{t("forwarder.forwarded")}: <span className="text-[oklch(0.65_0.1_150)]">{stats.forwarded}</span></span>
          <span>{t("forwarder.replied")}: <span className="text-[oklch(0.65_0.1_280)]">{stats.replied}</span></span>
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

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (<div><h3 className="text-sm font-semibold text-foreground border-b border-border pb-1.5 mb-3">{title}</h3><div className="space-y-3">{children}</div></div>);
}

function CheckRow({ label, checked, onChange }: { label: string; checked: boolean; onChange: (v: boolean) => void }) {
  return <label className="flex items-center gap-2.5 cursor-pointer text-sm"><input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} className="rounded border-border accent-primary h-4 w-4" /><span className="text-foreground font-medium">{label}</span></label>;
}

function DelayRow({ label, min, max, onMinChange, onMaxChange }: { label: string; min: number; max: number; onMinChange: (v: number) => void; onMaxChange: (v: number) => void }) {
  return (
    <div>
      <label className="text-xs font-medium text-muted-foreground">{label}</label>
      <div className="flex items-center gap-1.5 mt-1">
        <input type="number" min={0} value={min} onChange={(e) => onMinChange(Math.max(0, Number(e.target.value)))} className="w-14 rounded-md border border-border bg-background px-2 py-1 text-xs text-center" />
        <span className="text-xs text-muted-foreground">—</span>
        <input type="number" min={0} value={max} onChange={(e) => onMaxChange(Math.max(0, Number(e.target.value)))} className="w-14 rounded-md border border-border bg-background px-2 py-1 text-xs text-center" />
      </div>
    </div>
  );
}
