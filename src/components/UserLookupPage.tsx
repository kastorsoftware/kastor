import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Play, Square, FolderOpen } from "lucide-react";
import { AccountPickerModal } from "@/components/AccountPickerModal";
import { useT } from "@/i18n";

interface UserLookupConfig {
  inputPath: string;
  outputPath: string;
  saveName: boolean;
  saveSurname: boolean;
  saveUsername: boolean;
  savePhone: boolean;
  saveNftGifts: boolean;
  savePersonalChannel: boolean;
}

const defaultConfig: UserLookupConfig = {
  inputPath: "",
  outputPath: "",
  saveName: true,
  saveSurname: true,
  saveUsername: true,
  savePhone: true,
  saveNftGifts: false,
  savePersonalChannel: false,
};

const STORAGE_KEY = "user_lookup_config";
const IS_DEV = !("__TAURI_INTERNALS__" in window);

function loadSavedConfig(): UserLookupConfig {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return { ...defaultConfig, ...JSON.parse(saved) };
  } catch {}
  return defaultConfig;
}

export function UserLookupPage() {
  const t = useT();
  const [config, setConfig] = useState<UserLookupConfig>(loadSavedConfig);
  const [running, setRunning] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [stats, setStats] = useState({ found: 0, notFound: 0 });
  const [pickerOpen, setPickerOpen] = useState(false);
  const logsRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(config));
  }, [config]);

  useEffect(() => {
    if (IS_DEV) return;
    const unlisten = listen<string>("user-lookup-log", (e) => {
      setLogs((prev) => [...prev, e.payload]);
      const msg = e.payload;
      if (msg === "Завершено" || msg === "Done") {
        setRunning(false);
      } else if (msg.startsWith("  не найден:") || msg.startsWith("  not found:")) {
        setStats((s) => ({ ...s, notFound: s.notFound + 1 }));
      } else if (msg.startsWith("  найден:") || msg.startsWith("  found:")) {
        setStats((s) => ({ ...s, found: s.found + 1 }));
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

  const set = <K extends keyof UserLookupConfig>(key: K, value: UserLookupConfig[K]) => {
    setConfig((prev) => ({ ...prev, [key]: value }));
  };

  const handleStart = () => {
    if (!config.inputPath.trim()) {
      setLogs((prev) => [...prev, t("userLookup.errNoInput")]);
      return;
    }
    setPickerOpen(true);
  };

  const handleAccountSelected = async (ids: string[]) => {
    if (ids.length === 0) return;
    setLogs([]);
    setStats({ found: 0, notFound: 0 });
    setRunning(true);
    try {
      const tid = await invoke<string>("user_lookup_start", {
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
    if (!IS_DEV && taskId) await invoke("user_lookup_stop", { taskId }).catch(() => {});
    setRunning(false);
    setLogs((prev) => [...prev, t("common.stoppedByUser")]);
  };

  const selectInputFile = async () => {
    if (IS_DEV) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open({ multiple: false, filters: [{ name: "Text", extensions: ["txt"] }] });
    if (path) set("inputPath", path as string);
  };

  const selectOutputFile = async () => {
    if (IS_DEV) return;
    const { save } = await import("@tauri-apps/plugin-dialog");
    const path = await save({ defaultPath: "users.txt", filters: [{ name: "Text", extensions: ["txt"] }] });
    if (path) set("outputPath", path);
  };

  return (
    <div className="space-y-6">
      {/* config panel */}
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div className="p-6 space-y-6">

          <Section title={t("userLookup.inputFileTitle")}>
            <div>
              <p className="text-xs text-muted-foreground mb-2">
                {t("userLookup.inputFileHint")}
              </p>
              <button onClick={selectInputFile} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition">
                <FolderOpen className="h-4 w-4" /> {t("common.selectFile")}
              </button>
              <span className="text-xs text-muted-foreground truncate max-w-sm">{config.inputPath ? config.inputPath.split(/[/\\]/).pop() : t("common.notSelected")}</span>
            </div>
          </Section>

          <Section title={t("userLookup.saveTitle")}>
            <div className="grid grid-cols-2 gap-y-2 gap-x-6">
              <CheckRow label={t("userLookup.saveStatus")} checked disabled />
              <CheckRow label={t("userLookup.saveName")} checked={config.saveName} onChange={(v) => set("saveName", v)} />
              <CheckRow label={t("userLookup.saveSurname")} checked={config.saveSurname} onChange={(v) => set("saveSurname", v)} />
              <CheckRow label={t("userLookup.saveUsername")} checked={config.saveUsername} onChange={(v) => set("saveUsername", v)} />
              <CheckRow label={t("userLookup.savePhone")} checked={config.savePhone} onChange={(v) => set("savePhone", v)} />
              <CheckRow label={t("userLookup.saveNftGifts")} checked={config.saveNftGifts} onChange={(v) => set("saveNftGifts", v)} />
              <CheckRow label={t("userLookup.savePersonalChannel")} checked={config.savePersonalChannel} onChange={(v) => set("savePersonalChannel", v)} />
            </div>
          </Section>

          <Section title={t("userLookup.outputTitle")}>
            <div>
              <label className="text-sm font-medium text-foreground">{t("userLookup.outputFile")}</label>
              <div className="flex items-center gap-2 mt-1.5">
                <button onClick={selectOutputFile} className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium hover:border-primary/50 transition">
                  <Play className="h-3.5 w-3.5" /> {t("common.selectFile")}
                </button>
                <span className="text-xs text-muted-foreground truncate max-w-sm">{config.outputPath ? config.outputPath.split(/[/\\]/).pop() : t("userLookup.outputDefault")}</span>
              </div>
              <p className="mt-1 text-xs text-muted-foreground">
                {t("userLookup.outputHint")}
              </p>
            </div>
          </Section>

          <p className="text-xs text-muted-foreground">
            {t("userLookup.distributionHint")}
          </p>

        </div>
      </div>

      {/* controls */}
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-6 text-sm font-semibold">
          <span>{t("userLookup.found")} <span className="text-[oklch(0.65_0.1_150)]">{stats.found}</span></span>
          <span>{t("userLookup.notFound")} <span className="text-[oklch(0.55_0.1_25)]">{stats.notFound}</span></span>
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
        title={t("userLookup.pickerTitle")}
      />
    </div>
  );
}

function serializeConfig(c: UserLookupConfig) {
  return {
    input_path: c.inputPath.trim(),
    output_path: c.outputPath.trim(),
    save_name: c.saveName,
    save_surname: c.saveSurname,
    save_username: c.saveUsername,
    save_phone: c.savePhone,
    save_nft_gifts: c.saveNftGifts,
    save_personal_channel: c.savePersonalChannel,
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

function CheckRow({ label, checked, onChange, disabled }: { label: string; checked: boolean; onChange?: (v: boolean) => void; disabled?: boolean }) {
  return (
    <label className="flex items-center gap-2.5 cursor-pointer text-sm">
      <input
        type="checkbox"
        checked={checked}
        onChange={onChange ? (e) => onChange(e.target.checked) : undefined}
        disabled={disabled}
        className="rounded border-border accent-primary h-4 w-4 disabled:opacity-60"
      />
      <span className="text-foreground font-medium">{label}</span>
    </label>
  );
}
