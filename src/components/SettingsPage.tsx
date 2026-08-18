import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AlertTriangle, RefreshCw } from "lucide-react";
import { useT } from "@/i18n";

interface AppSettings {
  allow_no_proxy: boolean;
  llm_api_url: string;
  llm_token: string;
  llm_model: string;
  llm_api_type: string;
}

export function SettingsPage() {
  const t = useT();
  const [settings, setSettings] = useState<AppSettings>({ allow_no_proxy: false, llm_api_url: "", llm_token: "", llm_model: "", llm_api_type: "openai" });
  const [warning, setWarning] = useState(false);
  const [models, setModels] = useState<string[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [detectingType, setDetectingType] = useState(false);

  const IS_DEV = !("__TAURI_INTERNALS__" in window);

  useEffect(() => {
    if (IS_DEV) return;
    invoke<AppSettings>("get_settings").then(setSettings).catch(() => {});
  }, []);

  const handleToggleNoProxy = async () => {
    if (!settings.allow_no_proxy) {
      // turning ON - show warning first
      setWarning(true);
    } else {
      // turning OFF - just save
      const updated = { ...settings, allow_no_proxy: false };
      setSettings(updated);
      if (!IS_DEV) await invoke("save_settings", { settings: updated });
    }
  };

  const confirmNoProxy = async () => {
    const updated = { ...settings, allow_no_proxy: true };
    setSettings(updated);
    setWarning(false);
    if (!IS_DEV) await invoke("save_settings", { settings: updated });
  };

  const handleLlmChange = async (key: keyof AppSettings, value: string) => {
    const updated = { ...settings, [key]: value };
    setSettings(updated);
    if (!IS_DEV) await invoke("patch_settings", { patch: { [key]: value } });
  };

  const handleFetchModels = async () => {
    if (IS_DEV) { setModels(["gpt-4o", "gpt-4o-mini", "gpt-3.5-turbo"]); return; }
    setModelsLoading(true);
    try {
      const result = await invoke<string[]>("llm_get_models");
      setModels(result);
    } catch (e: any) {
      setModels([]);
      alert(`${t("common.error")}: ${e}`);
    } finally {
      setModelsLoading(false);
    }
  };

  const handleDetectApiType = async () => {
    if (IS_DEV) { handleLlmChange("llm_api_type", "openai"); return; }
    setDetectingType(true);
    try {
      const result = await invoke<string>("llm_detect_api_type");
      handleLlmChange("llm_api_type", result);
    } catch (e: any) {
      alert(`${t("common.error")}: ${e}`);
    } finally {
      setDetectingType(false);
    }
  };

  return (
    <div className="space-y-6 max-w-2xl">
      <div>
        <h2 className="text-lg font-semibold">{t("settings.title")}</h2>
        <p className="text-sm text-muted-foreground mt-1">{t("settings.subtitle")}</p>
      </div>

      <div className="rounded-xl border border-border bg-card p-5 space-y-4">
        <h3 className="text-sm font-semibold">{t("settings.proxySection")}</h3>

        <label className="flex items-start gap-3 cursor-pointer">
          <input
            type="checkbox"
            checked={settings.allow_no_proxy}
            onChange={handleToggleNoProxy}
            className="rounded border-border mt-0.5"
          />
          <div>
            <div className="text-sm font-medium">{t("settings.allowNoProxy")}</div>
            <div className="text-xs text-muted-foreground mt-0.5">
              {t("settings.allowNoProxyDesc")}
            </div>
          </div>
        </label>
      </div>

      {/* LLM */}
      <div className="rounded-xl border border-border bg-card p-5 space-y-4">
        <h3 className="text-sm font-semibold">{t("settings.llmSection")}</h3>

        <div>
          <label className="text-sm font-medium text-foreground">{t("settings.llmApiUrl")}</label>
          <input
            value={settings.llm_api_url}
            onChange={(e) => handleLlmChange("llm_api_url", e.target.value)}
            placeholder="https://api.openai.com/v1"
            className="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50"
          />
        </div>

        <div>
          <label className="text-sm font-medium text-foreground">{t("settings.llmToken")}</label>
          <input
            type="password"
            value={settings.llm_token}
            onChange={(e) => handleLlmChange("llm_token", e.target.value)}
            placeholder="sk-..."
            className="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50"
          />
        </div>

        <div>
          <label className="text-sm font-medium text-foreground">{t("settings.llmModel")}</label>
          <div className="flex items-center gap-2 mt-1">
            <input
              value={settings.llm_model}
              onChange={(e) => handleLlmChange("llm_model", e.target.value)}
              placeholder="gpt-4o-mini"
              className="flex-1 rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50"
            />
            <button
              onClick={handleFetchModels}
              disabled={modelsLoading}
              className="flex items-center gap-1.5 rounded-md border border-border px-3 py-2 text-xs font-medium hover:border-primary/50 transition disabled:opacity-50"
            >
              <RefreshCw className={`h-3.5 w-3.5 ${modelsLoading ? "animate-spin" : ""}`} />
              {t("settings.llmGetModels")}
            </button>
          </div>
          {models.length > 0 && (
            <div className="mt-2 flex flex-wrap gap-1.5">
              {models.map((m) => (
                <button
                  key={m}
                  onClick={() => handleLlmChange("llm_model", m)}
                  className={`rounded-md border px-2 py-1 text-xs transition ${
                    settings.llm_model === m
                      ? "border-primary/50 bg-primary/10 text-primary"
                      : "border-border text-muted-foreground hover:border-primary/30"
                  }`}
                >
                  {m}
                </button>
              ))}
            </div>
          )}
          <p className="mt-1 text-xs text-muted-foreground">{t("settings.llmProxyNote")}</p>
        </div>

        <div>
          <label className="text-sm font-medium text-foreground">{t("settings.llmApiType")}</label>
          <div className="flex items-center gap-2 mt-1">
            <button
              onClick={() => handleLlmChange("llm_api_type", "openai")}
              className={`rounded-md border px-3 py-1.5 text-xs font-medium transition ${
                settings.llm_api_type === "openai" ? "border-primary/50 bg-primary/10 text-primary" : "border-border text-muted-foreground hover:border-primary/30"
              }`}
            >
              ChatGPT / OpenAI
            </button>
            <button
              onClick={() => handleLlmChange("llm_api_type", "claude")}
              className={`rounded-md border px-3 py-1.5 text-xs font-medium transition ${
                settings.llm_api_type === "claude" ? "border-primary/50 bg-primary/10 text-primary" : "border-border text-muted-foreground hover:border-primary/30"
              }`}
            >
              Claude / Anthropic
            </button>
            <button
              onClick={handleDetectApiType}
              disabled={detectingType}
              className="rounded-md border border-border px-3 py-1.5 text-xs font-medium text-muted-foreground hover:border-primary/30 transition disabled:opacity-50"
            >
              {detectingType ? t("settings.llmDetecting") : t("settings.llmDetect")}
            </button>
          </div>
        </div>
      </div>

      {/* warning dialog */}
      {warning && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div className="rounded-xl border border-border bg-card p-6 w-96 shadow-2xl">
            <div className="flex items-center gap-3 mb-3">
              <AlertTriangle className="h-6 w-6 text-[oklch(0.6_0.15_60)]" />
              <h3 className="text-lg font-semibold">{t("settings.warning")}</h3>
            </div>
            <p className="text-sm text-muted-foreground mb-4">
              {t("settings.allowNoProxyWarning")}
            </p>
            <div className="flex gap-2">
              <button
                onClick={() => setWarning(false)}
                className="flex-1 rounded-md border border-border px-3 py-2 text-sm hover:bg-accent/50 transition"
              >
                {t("common.cancel")}
              </button>
              <button
                onClick={confirmNoProxy}
                className="flex-1 rounded-md border border-[oklch(0.6_0.15_60)]/50 bg-[oklch(0.6_0.15_60)]/10 px-3 py-2 text-sm text-[oklch(0.6_0.15_60)] font-medium hover:bg-[oklch(0.6_0.15_60)]/20 transition"
              >
                {t("settings.allow")}
              </button>
            </div>
          </div>
        </div>
      )}

    </div>
  );
}
