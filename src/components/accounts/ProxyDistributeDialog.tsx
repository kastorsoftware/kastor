import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useT } from "@/i18n";

export function ProxyDistributeDialog({ onClose, onDone }: { onClose: () => void; onDone: () => Promise<void> | void }) {
  const t = useT();
  const [info, setInfo] = useState<{ accounts: number; proxies: number; unassigned: number } | null>(null);
  const [distributing, setDistributing] = useState(false);

  const IS_DEV = !("__TAURI_INTERNALS__" in window);

  useEffect(() => {
    if (IS_DEV) { setInfo({ accounts: 5, proxies: 2, unassigned: 3 }); return; }
    invoke<[number, number, number]>("get_proxy_distribution_info").then(([a, p, u]) => {
      setInfo({ accounts: a, proxies: p, unassigned: u });
    });
  }, []);

  const handleDistribute = async (mode: string) => {
    if (IS_DEV) { onClose(); return; }
    setDistributing(true);
    await invoke("distribute_proxies", { mode });
    setDistributing(false);
    await onDone();
    onClose();
  };

  if (!info) return null;

  const needsChoice = info.proxies < info.unassigned && info.proxies > 0;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div className="rounded-xl border border-border bg-card p-6 w-96 shadow-2xl">
        <h3 className="text-lg font-semibold mb-3">{t("distributeDialog.title")}</h3>

        <div className="text-sm text-muted-foreground space-y-1 mb-3">
          <div>{t("distributeDialog.unassigned")}: <span className="text-foreground font-medium">{info.unassigned}</span></div>
          <div>{t("distributeDialog.available")}: <span className="text-foreground font-medium">{info.proxies}</span></div>
        </div>

        {info.proxies === 0 ? (
          <div className="text-sm text-destructive mb-4">
            {t("distributeDialog.noProxies")}
          </div>
        ) : info.unassigned === 0 ? (
          <div className="text-sm text-[oklch(0.65_0.1_150)] mb-4">
            {t("distributeDialog.allAssigned")}
          </div>
        ) : needsChoice ? (
          <div className="text-sm text-muted-foreground mb-4">
            {t("distributeDialog.fewerProxies")}
          </div>
        ) : null}

        <div className="flex flex-col gap-2">
          {info.proxies > 0 && info.unassigned > 0 && (
            <>
              {needsChoice ? (
                <>
                  <button
                    onClick={() => handleDistribute("skip")}
                    disabled={distributing}
                    className="w-full rounded-md border border-border bg-background px-4 py-2.5 text-sm font-medium hover:border-primary/50 transition text-left disabled:opacity-50"
                  >
                    <div>{t("distributeDialog.modeSkip")}</div>
                    <div className="text-xs text-muted-foreground mt-0.5">
                      {t("distributeDialog.modeSkipDesc", { proxies: info.proxies, remaining: info.unassigned - info.proxies })}
                    </div>
                  </button>
                  <button
                    onClick={() => handleDistribute("reuse")}
                    disabled={distributing}
                    className="w-full rounded-md border border-border bg-background px-4 py-2.5 text-sm font-medium hover:border-primary/50 transition text-left disabled:opacity-50"
                  >
                    <div>{t("distributeDialog.modeReuse")}</div>
                    <div className="text-xs text-muted-foreground mt-0.5">
                      {t("distributeDialog.modeReuseDesc")}
                    </div>
                  </button>
                </>
              ) : (
                <button
                  onClick={() => handleDistribute("reuse")}
                  disabled={distributing}
                  className="w-full rounded-md border border-border bg-background px-4 py-2.5 text-sm font-medium hover:border-primary/50 transition disabled:opacity-50"
                >
                  {distributing ? t("distributeDialog.distributing") : t("distributeDialog.distribute")}
                </button>
              )}
            </>
          )}
          {info.proxies > 0 && (
            <button
              onClick={() => handleDistribute("clear_proxies")}
              disabled={distributing}
              className="w-full rounded-md border border-border bg-background px-4 py-2.5 text-sm font-medium hover:border-destructive/30 hover:text-destructive transition text-left disabled:opacity-50"
            >
              <div>{t("distributeDialog.clearProxies")}</div>
              <div className="text-xs text-muted-foreground mt-0.5">
                {t("distributeDialog.clearProxiesDesc")}
              </div>
            </button>
          )}
          {info.proxies > 0 && (
            <button
              onClick={() => handleDistribute("redistribute")}
              disabled={distributing}
              className="w-full rounded-md border border-border bg-background px-4 py-2.5 text-sm font-medium hover:border-primary/50 transition text-left disabled:opacity-50"
            >
              <div>{t("distributeDialog.redistribute")}</div>
              <div className="text-xs text-muted-foreground mt-0.5">
                {t("distributeDialog.redistributeDesc")}
              </div>
            </button>
          )}
          <button
            onClick={onClose}
            className="w-full rounded-md border border-border px-3 py-2 text-sm text-muted-foreground hover:text-foreground transition"
          >
            {t("common.cancel")}
          </button>
        </div>
      </div>
    </div>
  );
}
