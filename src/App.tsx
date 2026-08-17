import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { WifiOff, X } from "lucide-react";
import { toast } from "sonner";
import { Dashboard } from "@/components/Dashboard";
import { ToolsRecommendation } from "@/components/ToolsRecommendation";
import { Toaster } from "@/components/ui/sonner";
import { I18nProvider } from "@/i18n";
import { en } from "@/i18n/en";
import { ru } from "@/i18n/ru";

const IS_DEV = !("__TAURI_INTERNALS__" in window);

function App() {
  const [showToolsRec, setShowToolsRec] = useState(false);
  const shown = useRef(false);

  useEffect(() => {
    if (shown.current || IS_DEV) return;
    shown.current = true;
    requestAnimationFrame(() => invoke("show_window"));
  }, []);

  useEffect(() => {
    if (!localStorage.getItem("tools_rec_skipped")) setShowToolsRec(true);
  }, []);

  useEffect(() => {
    if (IS_DEV) return;

    invoke<boolean>("check_telegram_connectivity").then((ok) => {
      if (ok) return;
      const locale = localStorage.getItem("app_locale") || "ru";
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
