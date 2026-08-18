import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ExternalLink, Download, CheckCircle, Database, FileEdit, X } from "lucide-react";
import { useI18n, useT } from "@/i18n";

interface ToolStatus {
  beekeeper: boolean;
  notepadpp: boolean;
}

interface Props {
  onDismiss: () => void;
}

export function ToolsRecommendation({ onDismiss }: Props) {
  const handleDismiss = () => {
    localStorage.setItem("tools_rec_skipped", "true");
    onDismiss();
  };
  const t = useT();
  const { locale, setLocale } = useI18n();
  const [status, setStatus] = useState<ToolStatus | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    invoke<ToolStatus>("check_installed_tools")
      .then((s) => {
        setStatus(s);
        setLoading(false);
      })
      .catch(() => setLoading(false));
  }, []);

  const openLink = (url: string) => {
    invoke("open_url", { url });
  };

  if (loading) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-background">
        <div className="h-8 w-8 animate-spin rounded-full border-2 border-primary border-t-transparent" />
      </div>
    );
  }

  const allInstalled = status?.beekeeper && status?.notepadpp;
  if (allInstalled) {
    handleDismiss();
    return null;
  }

  const tools = [
    {
      id: "beekeeper" as const,
      name: "Beekeeper Studio",
      icon: Database,
      color: "oklch(0.65 0.15 145)",
      bgColor: "oklch(0.65 0.15 145 / 0.08)",
      borderColor: "oklch(0.65 0.15 145 / 0.25)",
      description: t("toolsRec.beekeeperDesc"),
      recommended: true,
      url: "https://github.com/beekeeper-studio/beekeeper-studio/releases/download/v5.8.1/Beekeeper-Studio-Setup-5.8.1.exe",
      installed: status?.beekeeper ?? false,
    },
    {
      id: "notepadpp" as const,
      name: "Notepad++",
      icon: FileEdit,
      color: "oklch(0.6 0.12 250)",
      bgColor: "oklch(0.6 0.12 250 / 0.08)",
      borderColor: "oklch(0.6 0.12 250 / 0.25)",
      description: t("toolsRec.notepadppDesc"),
      recommended: false,
      url: "https://github.com/notepad-plus-plus/notepad-plus-plus/releases/download/v8.9.6.4/npp.8.9.6.4.Installer.x64.exe",
      installed: status?.notepadpp ?? false,
    },
  ];

  const missing = tools.filter((t) => !t.installed);

  return (
    <div className="flex min-h-screen items-center justify-center bg-background px-4">
      <div
        className="pointer-events-none absolute inset-0"
        style={{ background: "var(--gradient-glow)" }}
      />
      <button
        onClick={() => setLocale(locale === "en" ? "ru" : "en")}
        className="absolute right-4 top-4 z-10 flex h-11 w-11 flex-col items-center justify-center rounded-lg border border-border bg-card text-base shadow-sm transition hover:border-primary/50 hover:bg-accent/50"
        title={locale === "en" ? "Переключить на русский" : "Switch to English"}
        aria-label={locale === "en" ? "Switch to Russian" : "Switch to English"}
      >
        <span className={locale === "en" ? "opacity-100" : "opacity-45"}>🇬🇧</span>
        <span className={locale === "ru" ? "opacity-100" : "opacity-45"}>🇷🇺</span>
      </button>
      <div className="relative w-full max-w-lg">
        <div className="rounded-2xl border border-border bg-card/90 backdrop-blur-xl p-8 shadow-2xl">
          <div className="flex items-start justify-between">
            <div>
              <h1 className="text-xl font-bold tracking-tight text-foreground">
                {t("toolsRec.title")}
              </h1>
              <p className="mt-1 text-sm text-muted-foreground">
                {t("toolsRec.subtitle")}
              </p>
            </div>
            <button
              onClick={handleDismiss}
              className="rounded-lg p-1.5 text-muted-foreground hover:bg-accent/50 hover:text-foreground transition"
            >
              <X className="h-4 w-4" />
            </button>
          </div>

          <div className="mt-6 space-y-4">
            {missing.map((tool) => (
              <div
                key={tool.id}
                className="relative overflow-hidden rounded-xl border p-5 transition"
                style={{
                  borderColor: tool.borderColor,
                  background: tool.bgColor,
                }}
              >
                {tool.recommended && (
                  <div
                    className="absolute right-0 top-0 rounded-bl-lg px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wider text-white"
                    style={{ background: tool.color }}
                  >
                    {t("toolsRec.recommended")}
                  </div>
                )}
                <div className="flex items-start gap-4">
                  <div
                    className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg"
                    style={{ background: tool.color }}
                  >
                    <tool.icon className="h-5 w-5 text-white" />
                  </div>
                  <div className="min-w-0 flex-1">
                    <h3 className="text-sm font-semibold text-foreground">
                      {tool.name}
                    </h3>
                    <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
                      {tool.description}
                    </p>
                    <button
                      onClick={() => openLink(tool.url)}
                      className="mt-3 inline-flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium text-white transition hover:opacity-90"
                      style={{ background: tool.color }}
                    >
                      <Download className="h-3.5 w-3.5" />
                      {t("toolsRec.download")}
                      <ExternalLink className="h-3 w-3 opacity-70" />
                    </button>
                  </div>
                </div>
              </div>
            ))}
          </div>

          <div className="mt-6 flex items-center gap-2 rounded-lg border border-border/50 bg-background/50 p-3">
            <CheckCircle className="h-4 w-4 shrink-0 text-primary" />
            <p className="text-xs text-muted-foreground">
              {t("toolsRec.skipHint")}
            </p>
          </div>

          <button
            onClick={handleDismiss}
            className="mt-5 w-full rounded-lg border border-border bg-background py-2.5 text-sm font-medium text-foreground transition hover:bg-accent/50"
          >
            {t("toolsRec.continue")}
          </button>
        </div>
      </div>
    </div>
  );
}
