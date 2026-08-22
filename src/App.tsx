import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Download, WifiOff, X } from "lucide-react";
import { toast } from "sonner";
import { Dashboard } from "@/components/Dashboard";
import { ToolsRecommendation } from "@/components/ToolsRecommendation";
import { Toaster } from "@/components/ui/sonner";
import { I18nProvider } from "@/i18n";
import { en } from "@/i18n/en";
import { ru } from "@/i18n/ru";

const IS_DEV = !("__TAURI_INTERNALS__" in window);

interface UpdateInfo {
  current_version: string;
  version: string;
}

interface StoredSettings {
  auto_update: boolean;
}

function UpdateToast({ update, automatic, onInstall, onCancel }: {
  update: UpdateInfo;
  automatic: boolean;
  onInstall: () => void;
  onCancel: () => void;
}) {
  const [seconds, setSeconds] = useState(30);
  const locale = localStorage.getItem("app_locale") || "en";
  const ru = locale !== "en";

  useEffect(() => {
    if (!automatic) return;
    if (seconds === 0) { onInstall(); return; }
    const timer = window.setTimeout(() => setSeconds((value) => value - 1), 1000);
    return () => window.clearTimeout(timer);
  }, [automatic, onInstall, seconds]);

  return (
    <div className="update-toast">
      <div className="update-toast__icon"><Download className="h-4 w-4" /></div>
      <div className="min-w-0 flex-1">
        <div className="update-toast__title">{ru ? "Доступно обновление" : "Update available"}</div>
        <div className="update-toast__text">
          {update.current_version} → {update.version}
          {automatic && ` · ${ru ? `через ${seconds} сек.` : `in ${seconds}s`}`}
        </div>
      </div>
      {automatic ? (
        <button className="update-toast__close" type="button" onClick={onCancel} title={ru ? "Отменить" : "Cancel"} aria-label={ru ? "Отменить" : "Cancel"}>
          <X className="h-4 w-4" />
        </button>
      ) : (
        <button className="update-toast__close" type="button" onClick={onInstall} title={ru ? "Обновить" : "Update"} aria-label={ru ? "Обновить" : "Update"}>
          <Download className="h-4 w-4" />
        </button>
      )}
    </div>
  );
}

function WaitingForTasksToast({ update, onCancel }: { update: UpdateInfo; onCancel: () => void }) {
  const locale = localStorage.getItem("app_locale") || "en";
  const ru = locale !== "en";

  return (
    <div className="update-toast">
      <div className="update-toast__icon"><Download className="h-4 w-4" /></div>
      <div className="min-w-0 flex-1">
        <div className="update-toast__title">{ru ? "Обновление ожидает" : "Update pending"}</div>
        <div className="update-toast__text">
          {update.current_version} → {update.version} · {ru ? "установится через 30 сек. после завершения задач" : "will install 30s after tasks finish"}
        </div>
      </div>
      <button className="update-toast__close" type="button" onClick={onCancel} title={ru ? "Отменить" : "Cancel"} aria-label={ru ? "Отменить" : "Cancel"}>
        <X className="h-4 w-4" />
      </button>
    </div>
  );
}

function App() {
  const [showToolsRec, setShowToolsRec] = useState(false);
  const shown = useRef(false);
  const updateStarted = useRef(false);
  const updateCancelled = useRef(false);

  const installUpdate = useCallback(async () => {
    if (updateStarted.current) return;
    updateStarted.current = true;
    try {
      await invoke("download_and_apply_update");
    } catch (error) {
      updateStarted.current = false;
      toast.error(String(error));
    }
  }, []);

  useEffect(() => {
    if (shown.current || IS_DEV) return;
    shown.current = true;
    requestAnimationFrame(() => invoke("show_window"));
  }, []);

  useEffect(() => {
    if (IS_DEV) return;
    let retryTimer: number | undefined;
    let dismissed = false;

    const showUpdate = (update: UpdateInfo, automatic: boolean) => {
      const id = toast.custom(
        () => <UpdateToast
          update={update}
          automatic={automatic}
          onInstall={installUpdate}
          onCancel={() => { updateCancelled.current = true; toast.dismiss(id); }}
        />,
        { duration: Infinity, position: "bottom-right" },
      );
    };

    const waitForTasks = async (update: UpdateInfo, waitingToastId: string | number) => {
      if (dismissed || updateCancelled.current) return;
      try {
        const activeTasks = await invoke<number>("get_active_task_count");
        if (activeTasks === 0) {
          toast.dismiss(waitingToastId);
          showUpdate(update, true);
          return;
        }
      } catch {
        toast.dismiss(waitingToastId);
        return;
      }
      retryTimer = window.setTimeout(() => { void waitForTasks(update, waitingToastId); }, 2000);
    };

    const check = async () => {
      try {
        const settings = await invoke<StoredSettings>("get_settings");
        const update = await invoke<UpdateInfo | null>("check_for_update");
        if (!update) return;
        if (!settings.auto_update) {
          showUpdate(update, false);
          return;
        }
        const waitingToastId = toast.custom(
          (id) => <WaitingForTasksToast update={update} onCancel={() => { updateCancelled.current = true; toast.dismiss(id); }} />,
          { duration: Infinity, position: "bottom-right" },
        );
        await waitForTasks(update, waitingToastId);
      } catch {
        // The update check must never block application startup.
      }
    };

    void check();
    return () => {
      dismissed = true;
      if (retryTimer !== undefined) window.clearTimeout(retryTimer);
    };
  }, [installUpdate]);

  useEffect(() => {
    if (!localStorage.getItem("tools_rec_skipped")) setShowToolsRec(true);
  }, []);

  useEffect(() => {
    if (IS_DEV) return;

    invoke<boolean>("check_telegram_connectivity").then((ok) => {
      if (ok) return;
      const locale = localStorage.getItem("app_locale") || "en";
      const dict = locale === "en" ? en : ru;
      toast.custom(
        (id) => (
          <div className="update-toast">
            <div className="update-toast__icon"><WifiOff className="h-4 w-4" /></div>
            <div className="min-w-0 flex-1">
              <div className="update-toast__title">{dict.telegramUnavailable.title}</div>
              <div className="update-toast__text">{dict.telegramUnavailable.description}</div>
            </div>
            <button className="update-toast__close" type="button" onClick={() => toast.dismiss(id)}>
              <X className="h-4 w-4" />
            </button>
          </div>
        ),
        { duration: 20000, position: "bottom-right" }
      );
    }).catch(() => {});
  }, []);

  return (
    <I18nProvider>
      <Toaster />
      {showToolsRec ? <ToolsRecommendation onDismiss={() => setShowToolsRec(false)} /> : <Dashboard />}
    </I18nProvider>
  );
}

export default App;
